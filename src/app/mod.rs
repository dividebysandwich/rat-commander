//! Application entry point: terminal setup, the render/event loop, and the
//! suspend-and-run-command bridge.

pub mod event;
pub mod state;

use crate::ui;
use crate::util::async_bridge::{self, AppReceiver};
use crate::util::Result;
use futures::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyEventKind,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    supports_keyboard_enhancement,
};
use ratatui::crossterm::{execute, queue};
use state::{AppState, Flow};
use std::io::{self, Stdout, Write};

type Term = Terminal<CrosstermBackend<Stdout>>;

/// Set up, run, and tear down the application.
pub async fn run(startup: crate::Startup) -> Result<()> {
    // Load user themes (generating themes.toml from the presets on first run)
    // before the initial theme is derived from the config.
    crate::ui::theme::load_user_themes();
    let (tx, mut rx) = async_bridge::channel();
    let mut state = AppState::new(tx);
    // Generate/discover the `lang/` files and activate the configured language
    // before anything renders.
    crate::l10n::load_languages(state.config.language.as_deref());
    crate::l10n::set_reshape_rtl(state.config.reshape_rtl);
    state.init().await;

    // `rc /edit <file>` (or the `rcedit` shim) opens straight into the editor;
    // closing it then exits the program (rather than dropping to the panels).
    // With no file, a fresh unnamed buffer opens instead (first save prompts
    // for a name via "Save as").
    match startup {
        crate::Startup::Panels => {}
        crate::Startup::Edit(file) => {
            state.edit_only = true;
            state.open_path_in_editor(file).await;
        }
        crate::Startup::EditNew => {
            state.edit_only = true;
            state.open_new_editor();
        }
    }

    let (mut term, kbd) = setup_terminal()?;
    state.kbd_enhanced = kbd;
    // Detect terminal pixel-graphics support once, in raw mode + alternate
    // screen, before the event stream starts consuming stdin (the probe reads
    // the terminal's query responses directly).
    state.gfx = crate::ui::graphics::Gfx::detect(&state.config.graphics);
    let mut events = EventStream::new();

    let result = run_loop(&mut term, &mut state, &mut rx, &mut events).await;

    // Remember each panel's view format and sort order for the next session.
    state.persist_panel_views();
    restore_terminal(&mut term, state.kbd_enhanced)?;
    result
}

async fn run_loop(
    term: &mut Term,
    state: &mut AppState,
    rx: &mut AppReceiver,
    events: &mut EventStream,
) -> Result<()> {
    // ~100 ms tick drives animations and the system-status sampler.
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(100));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Persistent Ctrl-O subshell, kept alive across toggles.
    let mut subshell: Option<crate::shell::Subshell> = None;

    loop {
        // Start-in-editor mode (`rc /edit …`): once the editor and any of its
        // dialogs are closed, the program's work is done — exit instead of
        // revealing the file-manager panels.
        if state.edit_only && state.editor.is_none() && state.dialog.is_none() {
            break;
        }
        // Refresh the Details panel(s) before drawing: this detects when the
        // source panel's cursor/selection changed and (re)starts background size
        // scans. Cheap when nothing changed.
        state.update_details();
        term.draw(|f| ui::draw(f, state))?;

        tokio::select! {
            maybe_event = events.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(key))) => {
                        // Track held modifiers (where the terminal reports them via
                        // the enhanced keyboard protocol) so the editor's F-key bar
                        // can show the Shift/Ctrl alternate labels while held.
                        if state.kbd_enhanced
                            && let Some(ed) = state.editor.as_mut()
                        {
                            ed.note_key(key);
                        }
                        // Act on presses and auto-repeats; release events only
                        // update the modifier hint above.
                        if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                            match state.handle_key(key).await {
                                Flow::Quit => break,
                                Flow::RunCommand(cmd) => run_command(term, state, &cmd).await?,
                                Flow::RunExternal { program, path } => {
                                    run_external(term, state, &program, &path).await?
                                }
                                Flow::SubShell => toggle_subshell(term, state, &mut subshell).await?,
                                Flow::Continue => {}
                            }
                        }
                    }
                    Some(Ok(Event::Mouse(me))) => {
                        match state.handle_mouse(me).await {
                            Flow::Quit => break,
                            Flow::RunCommand(cmd) => run_command(term, state, &cmd).await?,
                            Flow::RunExternal { program, path } => {
                                run_external(term, state, &program, &path).await?
                            }
                            Flow::SubShell => toggle_subshell(term, state, &mut subshell).await?,
                            Flow::Continue => {}
                        }
                    }
                    Some(Ok(_)) => {} // resize / other: redraw next iteration
                    Some(Err(e)) => return Err(e.into()),
                    None => break, // stdin closed
                }
            }
            Some(app_event) = rx.recv() => {
                state.apply_event(app_event).await;
            }
            _ = ticker.tick(), if state.wants_ticks() => {
                state.on_tick();
                // Deliver a held Esc once its function-key window has elapsed.
                if let Flow::Quit = state.flush_expired_esc().await {
                    break;
                }
            }
        }
    }
    Ok(())
}

/// Build a `Command` that runs `cmd` through the platform shell
/// (`sh -c` on Unix, `cmd /C` on Windows).
fn shell_command(cmd: &str) -> tokio::process::Command {
    if cfg!(windows) {
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/C").arg(cmd);
        c
    } else {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c").arg(cmd);
        c
    }
}

/// The interactive shell to drop into for Ctrl-O.
fn interactive_shell() -> tokio::process::Command {
    if cfg!(windows) {
        let comspec = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string());
        tokio::process::Command::new(comspec)
    } else {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        tokio::process::Command::new(shell)
    }
}

/// Suspend the TUI, run a shell command in the active panel's directory, wait
/// for the user, then restore the TUI and refresh the panels.
async fn run_command(term: &mut Term, state: &mut AppState, cmd: &str) -> Result<()> {
    restore_terminal(term, state.kbd_enhanced)?;

    // Use the console directory only when it's a real local path; otherwise
    // (remote/archive panel) fall back to the process cwd. In Tree view this
    // follows the highlighted directory, matching the command-line prompt.
    let console = state.console_cwd();
    let cwd = if console.scheme == "file" {
        console.path.clone()
    } else {
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"))
    };
    println!("$ {cmd}");
    let status = shell_command(cmd).current_dir(&cwd).status().await;
    match status {
        Ok(s) if !s.success() => println!("\n[exit status: {s}]"),
        Err(e) => println!("\n[failed to run: {e}]"),
        _ => {}
    }
    print!("\n[Press Enter to return to Rat Commander]");
    io::stdout().flush().ok();
    let mut line = String::new();
    let _ = io::stdin().read_line(&mut line);

    let (t, k) = setup_terminal()?;
    *term = t;
    state.kbd_enhanced = k;
    term.clear()?;
    if let Some(g) = state.gfx.as_mut() {
        g.invalidate();
    }
    state.reload_all().await;
    Ok(())
}

/// Suspend the TUI and run an external program (editor/viewer) against a file.
async fn run_external(
    term: &mut Term,
    state: &mut AppState,
    program: &str,
    path: &std::path::Path,
) -> Result<()> {
    restore_terminal(term, state.kbd_enhanced)?;

    // Run `program <path>` via the shell so arguments in the command work.
    let cmd = format!("{program} \"{}\"", path.display());
    let status = shell_command(&cmd).status().await;
    if let Err(e) = status {
        println!("\n[failed to run external program: {e}]");
        print!("[Press Enter to continue]");
        io::stdout().flush().ok();
        let mut line = String::new();
        let _ = io::stdin().read_line(&mut line);
    }

    let (t, k) = setup_terminal()?;
    *term = t;
    state.kbd_enhanced = k;
    term.clear()?;
    if let Some(g) = state.gfx.as_mut() {
        g.invalidate();
    }
    state.reload_all().await;
    Ok(())
}

/// Ctrl-O: toggle the persistent subshell (Midnight Commander style). The shell
/// lives in a PTY and keeps its state between visits; Ctrl-O returns here.
async fn toggle_subshell(
    term: &mut Term,
    state: &mut AppState,
    subshell: &mut Option<crate::shell::Subshell>,
) -> Result<()> {
    let cwd = {
        let p = &state.panels[state.active];
        if p.cwd.scheme == "file" {
            p.cwd.path.clone()
        } else {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"))
        }
    };
    let size = term.size()?;

    // (Re)create the shell if needed.
    let needs_spawn = subshell.as_mut().map(|s| !s.is_alive()).unwrap_or(true);
    if needs_spawn {
        match crate::shell::Subshell::spawn(&cwd, size.height, size.width) {
            Ok(s) => *subshell = Some(s),
            Err(_) => {
                // Fall back to a one-shot shell if a PTY can't be created.
                return run_oneshot_shell(term, state, &cwd).await;
            }
        }
    }
    let Some(sh) = subshell.as_mut() else {
        return Ok(());
    };

    // Hand the terminal to the shell: leave the alternate screen (so the shell
    // is on the primary screen) and stop capturing the mouse. Raw mode stays on
    // so keystrokes pass through byte-for-byte; the PTY does its own cooking.
    {
        let out = term.backend_mut();
        // Hand normal keyboard reporting to the shell while it owns the screen.
        if state.kbd_enhanced {
            let _ = queue!(out, PopKeyboardEnhancementFlags);
        }
        queue!(out, LeaveAlternateScreen, DisableMouseCapture)?;
        out.flush()?;
    }
    term.show_cursor()?;
    sh.resize(size.height, size.width);

    sh.run_until_toggle();

    // Take the terminal back for the panels.
    {
        let out = term.backend_mut();
        queue!(out, EnterAlternateScreen, EnableMouseCapture)?;
        if state.kbd_enhanced {
            let _ = queue!(
                out,
                PushKeyboardEnhancementFlags(
                    KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                        | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
                )
            );
        }
        out.flush()?;
    }
    term.hide_cursor()?;
    term.clear()?;
    if let Some(g) = state.gfx.as_mut() {
        g.invalidate();
    }

    // Follow the shell's directory change back into the active panel (Linux).
    if let Some(dir) = sh.child_cwd() {
        let p = &mut state.panels[state.active];
        if p.cwd.scheme == "file" && dir != p.cwd.path {
            p.cwd = crate::vfs::VfsPath::local(dir);
            p.selection.clear();
            let _ = p.reload().await;
        }
    }
    state.reload_all().await;
    Ok(())
}

/// Fallback when a PTY can't be created: run an interactive shell once.
async fn run_oneshot_shell(term: &mut Term, state: &mut AppState, cwd: &std::path::Path) -> Result<()> {
    restore_terminal(term, state.kbd_enhanced)?;
    println!("[Rat Commander subshell — type 'exit' to return]");
    let _ = interactive_shell().current_dir(cwd).status().await;
    let (t, k) = setup_terminal()?;
    *term = t;
    state.kbd_enhanced = k;
    term.clear()?;
    if let Some(g) = state.gfx.as_mut() {
        g.invalidate();
    }
    state.reload_all().await;
    Ok(())
}

/// Set up the terminal, returning the terminal handle and whether the enhanced
/// keyboard protocol was enabled (so key release/repeat and standalone-modifier
/// events are reported — used to live-update the editor's F-key labels while
/// Shift/Ctrl is held). It is left off where the terminal doesn't support it.
fn setup_terminal() -> Result<(Term, bool)> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let kbd = supports_keyboard_enhancement().unwrap_or(false);
    if kbd {
        let _ = execute!(
            stdout,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
            )
        );
    }
    let backend = CrosstermBackend::new(stdout);
    let mut term = Terminal::new(backend)?;
    term.hide_cursor()?;
    Ok((term, kbd))
}

fn restore_terminal(term: &mut Term, kbd: bool) -> Result<()> {
    disable_raw_mode()?;
    let out = term.backend_mut();
    if kbd {
        let _ = queue!(out, PopKeyboardEnhancementFlags);
    }
    queue!(out, LeaveAlternateScreen, DisableMouseCapture)?;
    out.flush()?;
    term.show_cursor()?;
    Ok(())
}
