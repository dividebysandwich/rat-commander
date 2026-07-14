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
    // Now that the language is active, raise the (localized) nested-subshell
    // warning if this instance was started inside another's Ctrl-O subshell.
    state.warn_nested_subshell();
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

    let (mut term, kbd) = setup_terminal(None)?;
    state.kbd_enhanced = kbd;
    // Detect terminal pixel-graphics support once, in raw mode + alternate
    // screen, before the event stream starts consuming stdin (the probe reads
    // the terminal's query responses directly).
    state.gfx = crate::ui::graphics::Gfx::detect(&state.config.graphics);

    let result = run_loop(&mut term, &mut state, &mut rx).await;

    // Remember each panel's view format and sort order, and the command-line
    // history, for the next session.
    state.persist_panel_views();
    state.persist_command_history();
    restore_terminal(&mut term, state.kbd_enhanced)?;
    result
}

async fn run_loop(
    term: &mut Term,
    state: &mut AppState,
    rx: &mut AppReceiver,
) -> Result<()> {
    // ~100 ms tick drives animations and the system-status sampler.
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(100));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Persistent Ctrl-O shells, kept alive across toggles: the local subshell plus
    // one per open SFTP/SCP session (opened on demand).
    let mut shells = Shells::default();
    // The crossterm event stream is owned here (not by `run`) so the Ctrl-O
    // subshell path can drop it: its background reader thread would otherwise
    // keep competing for stdin while the subshell does its own blocking read,
    // stealing the very Ctrl-O that should toggle back.
    let mut events = EventStream::new();

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
        // Detect a panel directory change and (re)start its background git-status
        // scan; cheap when nothing changed.
        state.update_git();
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
                                Flow::RunCommand(cmd) => {
                                    run_command(term, state, &mut shells, &cmd).await?
                                }
                                Flow::RunExternal { program, path } => {
                                    run_external(term, state, &program, &path).await?
                                }
                                Flow::SubShell => {
                                    // Release the event stream's stdin reader
                                    // so the subshell owns the terminal input
                                    // while toggled in (its Ctrl-O to return
                                    // would otherwise be swallowed by the reader
                                    // thread). A fresh stream is recreated after.
                                    drop(events);
                                    toggle_subshell(term, state, &mut shells).await?;
                                    events = EventStream::new();
                                }
                                Flow::Continue => {}
                            }
                        }
                    }
                    Some(Ok(Event::Mouse(me))) => {
                        match state.handle_mouse(me).await {
                            Flow::Quit => break,
                            Flow::RunCommand(cmd) => {
                                run_command(term, state, &mut shells, &cmd).await?
                            }
                            Flow::RunExternal { program, path } => {
                                run_external(term, state, &program, &path).await?
                            }
                            Flow::SubShell => {
                                drop(events);
                                toggle_subshell(term, state, &mut shells).await?;
                                events = EventStream::new();
                            }
                            Flow::Continue => {}
                        }
                    }
                    Some(Ok(Event::Resize(cols, rows))) => {
                        // Keep the console emulator and every live shell PTY the
                        // same size as the terminal, then redraw next iteration.
                        state.console.resize(rows, cols);
                        if let Some(sh) = shells.local.as_ref() {
                            sh.resize(rows, cols);
                        }
                        for rsh in shells.remote.values() {
                            rsh.resize(rows, cols);
                        }
                    }
                    Some(Ok(_)) => {} // other events: redraw next iteration
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

/// Clear the screen and force a full repaint without querying the terminal.
///
/// `Terminal::clear` (ratatui 0.30) reads the cursor position first, which
/// sends `ESC[6n` and waits for the reply through crossterm's internal event
/// reader. In the suspend/resume paths that reply is unreliable: dropping the
/// `EventStream` for the Ctrl-O subshell leaves a stale byte in crossterm's
/// waker pipe, which makes the next `poll_internal` bail out before ever
/// reading the reply — the query times out and the resulting I/O error exits
/// the whole app right as the user toggles back. `Terminal::resize` clears the
/// screen and resets the diff buffer without any terminal round-trip.
///
/// Whatever was on screen is gone, so cached graphics protocols must
/// re-transmit their images: the gfx cache is invalidated here too.
fn force_full_redraw(term: &mut Term, state: &mut AppState) -> Result<()> {
    let size = term.size()?;
    term.resize(ratatui::layout::Rect::new(0, 0, size.width, size.height))?;
    if let Some(g) = state.gfx.as_mut() {
        g.invalidate();
    }
    Ok(())
}

/// Re-enter the TUI after a full suspend (external command, editor, one-shot
/// shell): re-acquire the terminal, force a full repaint, and reload the
/// panels. The enhanced-keyboard capability probed at startup is reused —
/// re-querying it would be another terminal round-trip through crossterm's
/// event reader (see [`force_full_redraw`]), and the capability doesn't
/// change mid-session.
async fn resume_tui(term: &mut Term, state: &mut AppState) -> Result<()> {
    *term = setup_terminal(Some(state.kbd_enhanced))?.0;
    force_full_redraw(term, state)?;
    state.reload_all().await;
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

/// Build a `Command` for a line typed at Rat Commander's own command line.
///
/// Unlike [`shell_command`], this runs the user's login shell **interactively**
/// (`$SHELL -i -c …`), so the command sees the same aliases, shell functions and
/// rc-file environment as the user's normal prompt. A non-interactive `sh -c`
/// never sources `~/.bashrc`/`~/.zshrc`, and bash disables alias expansion
/// outright when non-interactive — so an alias typed here would silently expand
/// to nothing ("command not found"). This mirrors the Ctrl-O subshell, which is
/// already the interactive `$SHELL`. Windows has no rc-file aliases, so it keeps
/// the plain `cmd /C` form.
fn command_line_shell(cmd: &str) -> tokio::process::Command {
    if cfg!(windows) {
        return shell_command(cmd);
    }
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let mut c = tokio::process::Command::new(shell);
    c.arg("-i").arg("-c").arg(cmd);
    c
}

/// Run a foreground child to completion with `system(3)`-style signal handling
/// so **Ctrl-C** (and Ctrl-\) interrupt the program, not Rat Commander.
///
/// While the child owns the (cooked-mode) terminal it shares our process group,
/// so the tty delivers SIGINT/SIGQUIT to both of us. We ignore them here for the
/// duration; the child resets them to their defaults (via `pre_exec`) so it can
/// still be interrupted — SIG_IGN would otherwise be inherited across `exec`.
#[cfg(unix)]
async fn run_foreground(
    mut cmd: tokio::process::Command,
) -> std::io::Result<std::process::ExitStatus> {
    // In the forked child, before exec: restore default SIGINT/SIGQUIT.
    // `signal()` is async-signal-safe, so this is legal in `pre_exec`.
    unsafe {
        cmd.pre_exec(|| {
            nix::libc::signal(nix::libc::SIGINT, nix::libc::SIG_DFL);
            nix::libc::signal(nix::libc::SIGQUIT, nix::libc::SIG_DFL);
            Ok(())
        });
    }
    // In us: ignore the signals while the child runs, then restore.
    let prev_int = unsafe { nix::libc::signal(nix::libc::SIGINT, nix::libc::SIG_IGN) };
    let prev_quit = unsafe { nix::libc::signal(nix::libc::SIGQUIT, nix::libc::SIG_IGN) };
    let status = cmd.status().await;
    unsafe {
        nix::libc::signal(nix::libc::SIGINT, prev_int);
        nix::libc::signal(nix::libc::SIGQUIT, prev_quit);
    }
    status
}

/// Non-Unix: no special signal juggling is needed.
#[cfg(not(unix))]
async fn run_foreground(
    mut cmd: tokio::process::Command,
) -> std::io::Result<std::process::ExitStatus> {
    cmd.status().await
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

/// Run a command from the command line in the **persistent console shell** — the
/// same session `Ctrl-O` drops into. The command is written to that shell (so its
/// output lands on the console backdrop and its cwd/env/history carry across
/// commands), running in the active panel's directory. The TUI is *not*
/// suspended: hide a panel, go half-height, or press `Ctrl-O` to watch the
/// output. Falls back to a one-shot suspended run only when no PTY can be made.
/// The persistent Ctrl-O / command-line shells: the single local subshell plus a
/// remote SSH shell per open SFTP/SCP session (keyed by the session's scheme).
#[derive(Default)]
struct Shells {
    local: Option<crate::shell::Subshell>,
    remote: std::collections::HashMap<String, crate::shell::RemoteShell>,
}

/// If the active panel is on an SSH remote (SFTP/SCP), the session scheme to run
/// the shell / command on. `None` for local, archive, or FTP panels.
fn active_ssh_remote(state: &AppState) -> Option<String> {
    let c = &state.panels[state.active].cwd;
    (c.scheme.starts_with("sftp") || c.scheme.starts_with("scp")).then(|| c.scheme.clone())
}

/// Ensure a live remote shell exists for session `scheme` (opening a shell
/// channel on the active panel's SSH backend the first time), make its console
/// the current backdrop, and `cd` it into the panel's directory. Returns whether
/// a live remote shell is available.
async fn ensure_remote_shell(
    state: &AppState,
    shells: &mut Shells,
    scheme: &str,
    rows: u16,
    cols: u16,
) -> bool {
    if !shells.remote.get(scheme).is_some_and(|s| s.is_alive()) {
        let backend = state.panels[state.active].backend.clone();
        let feed = crate::console::ConsoleFeed::new(rows, cols);
        match backend.open_shell(rows, cols).await {
            Ok(ch) => {
                let sh = crate::shell::RemoteShell::spawn(ch, feed, state.event_sender());
                shells.remote.insert(scheme.to_string(), sh);
            }
            Err(_) => return false, // not an SSH backend, or the channel failed
        }
    }
    if let Some(sh) = shells.remote.get_mut(scheme) {
        state.console.set_current(sh.console());
        state.console.resize(rows, cols);
        // Follow the active panel's remote directory.
        sh.cd_to(&state.panels[state.active].cwd.posix_path());
        true
    } else {
        false
    }
}

async fn run_command(
    term: &mut Term,
    state: &mut AppState,
    shells: &mut Shells,
    cmd: &str,
) -> Result<()> {
    // On Windows the persistent PTY console isn't used (its Unix tty passthrough
    // and POSIX `cd` quoting don't hold — see `toggle_subshell`), so run each
    // command the classic suspended way, in the active panel's directory.
    if cfg!(windows) {
        return run_command_fallback(term, state, cmd).await;
    }
    let size = term.size()?;

    // On an SFTP/SCP panel, run the command on the remote host over the session's
    // SSH connection (its output lands on the same console backdrop).
    if let Some(scheme) = active_ssh_remote(state)
        && ensure_remote_shell(state, shells, &scheme, size.height, size.width).await
        && let Some(sh) = shells.remote.get_mut(&scheme)
    {
        sh.send_line(cmd);
        return Ok(());
    }

    // The active panel's directory when it's a real local path (the local shell
    // can't drive a remote/archive panel's cwd).
    // The active panel's directory when it's a real local path (the shell is
    // local, so a remote/archive panel can't drive its cwd — see the prompt,
    // which follows the highlighted directory in Tree view).
    let target = {
        let c = state.console_cwd();
        (c.scheme == "file").then_some(c.path)
    };
    let spawn_cwd = target
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/")));

    if !ensure_subshell(state, shells, &spawn_cwd, size.height, size.width) {
        // No PTY available — run this one command the old, suspended way.
        return run_command_fallback(term, state, cmd).await;
    }
    let Some(sh) = shells.local.as_mut() else { return Ok(()) };

    // Run in the active panel's directory: cd there first, but only when the
    // shell isn't already sitting in it (its live cwd is read from /proc on
    // Linux; elsewhere we always cd, which is correct if noisier).
    if let Some(dir) = target
        && sh.child_cwd().as_deref() != Some(dir.as_path())
    {
        sh.send_line(&format!("cd {}", crate::vfs::remote::shell_quote(&dir.to_string_lossy())));
    }
    sh.send_line(cmd);
    Ok(())
}

/// Ensure the local console subshell is alive, (re)spawning it in `cwd` when
/// absent or dead, and make its console the current backdrop. Returns whether a
/// live shell is available. Shared with `Ctrl-O` (one local session).
fn ensure_subshell(
    state: &AppState,
    shells: &mut Shells,
    cwd: &std::path::Path,
    rows: u16,
    cols: u16,
) -> bool {
    if !shells.local.as_mut().is_some_and(|s| s.is_alive()) {
        let feed = crate::console::ConsoleFeed::new(rows, cols);
        match crate::shell::Subshell::spawn(cwd, rows, cols, feed, state.event_sender()) {
            Ok(s) => shells.local = Some(s),
            Err(_) => return false,
        }
    }
    if let Some(sh) = shells.local.as_ref() {
        state.console.set_current(sh.console());
        state.console.resize(rows, cols);
        true
    } else {
        false
    }
}

/// Fallback when no PTY can be created: suspend the TUI, run one command in the
/// active panel's directory, wait for Enter, then restore the TUI.
async fn run_command_fallback(term: &mut Term, state: &mut AppState, cmd: &str) -> Result<()> {
    restore_terminal(term, state.kbd_enhanced)?;
    let console_cwd = state.console_cwd();
    let cwd = if console_cwd.scheme == "file" {
        console_cwd.path.clone()
    } else {
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"))
    };
    println!("$ {cmd}");
    let mut child = command_line_shell(cmd);
    child.current_dir(&cwd);
    let status = run_foreground(child).await;
    match status {
        Ok(s) if !s.success() => println!("\n[exit status: {s}]"),
        Err(e) => println!("\n[failed to run: {e}]"),
        _ => {}
    }
    print!("\n[Press Enter to return to Rat Commander]");
    io::stdout().flush().ok();
    let mut line = String::new();
    let _ = io::stdin().read_line(&mut line);
    resume_tui(term, state).await
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
    let status = run_foreground(shell_command(&cmd)).await;
    if let Err(e) = status {
        println!("\n[failed to run external program: {e}]");
        print!("[Press Enter to continue]");
        io::stdout().flush().ok();
        let mut line = String::new();
        let _ = io::stdin().read_line(&mut line);
    }

    resume_tui(term, state).await
}

/// Ctrl-O: toggle into the persistent console shell (Midnight Commander style).
/// On an SFTP/SCP panel this is a shell on the **remote host** (over the session's
/// SSH connection); otherwise the local subshell. Either keeps its state between
/// visits; Ctrl-O returns here.
async fn toggle_subshell(
    term: &mut Term,
    state: &mut AppState,
    shells: &mut Shells,
) -> Result<()> {
    let cwd = {
        let p = &state.panels[state.active];
        if p.cwd.scheme == "file" {
            p.cwd.path.clone()
        } else {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"))
        }
    };

    // Windows: the persistent PTY subshell relies on forwarding the real
    // terminal's raw VT byte stream and scanning it for Ctrl-O
    // (`run_until_toggle`). Windows delivers console input as key-event records,
    // not a VT byte stream, so that model doesn't work — drop into a one-shot
    // interactive shell instead. (Remote shells share this limitation.)
    if cfg!(windows) {
        return run_oneshot_shell(term, state, &cwd).await;
    }

    let size = term.size()?;

    // On an SFTP/SCP panel, drop into a shell on the remote host.
    if let Some(scheme) = active_ssh_remote(state)
        && ensure_remote_shell(state, shells, &scheme, size.height, size.width).await
    {
        hand_terminal_to_shell(term, state)?;
        if let Some(sh) = shells.remote.get_mut(&scheme) {
            sh.resize(size.height, size.width);
            repaint_console(state);
            sh.run_until_toggle();
        }
        take_terminal_back(term, state)?;
        // Remote files may have changed; refresh the listings.
        state.reload_all().await;
        return Ok(());
    }

    // Local: (re)create the local subshell; fall back to a one-shot shell if no
    // PTY can be made.
    if !ensure_subshell(state, shells, &cwd, size.height, size.width) {
        return run_oneshot_shell(term, state, &cwd).await;
    }
    hand_terminal_to_shell(term, state)?;
    if let Some(sh) = shells.local.as_mut() {
        sh.resize(size.height, size.width);
        repaint_console(state);
        sh.run_until_toggle();
    }
    take_terminal_back(term, state)?;

    // Follow the shell's directory change back into the active panel (Linux).
    if let Some(dir) = shells.local.as_ref().and_then(|s| s.child_cwd()) {
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

/// Hand the terminal to a console shell: leave the alternate screen (so the shell
/// is on the primary screen) and stop capturing the mouse. Raw mode stays on so
/// keystrokes pass through byte-for-byte; the PTY does its own cooking.
fn hand_terminal_to_shell(term: &mut Term, state: &AppState) -> Result<()> {
    let out = term.backend_mut();
    if state.kbd_enhanced {
        let _ = queue!(out, PopKeyboardEnhancementFlags);
    }
    queue!(out, LeaveAlternateScreen, DisableMouseCapture)?;
    out.flush()?;
    term.show_cursor()?;
    Ok(())
}

/// Repaint the current shell's screen (from its console emulator) so entering
/// Ctrl-O shows the live session — including anything run from the command line —
/// instead of a blank primary screen until the next keystroke.
fn repaint_console(state: &AppState) {
    let parser = state.console.parser();
    if let Ok(parser) = parser.lock() {
        let mut out = io::stdout();
        let _ = out.write_all(b"\x1b[2J\x1b[H");
        let _ = out.write_all(&parser.screen().contents_formatted());
        let _ = out.flush();
    }
}

/// Take the terminal back for the panels after a shell visit.
fn take_terminal_back(term: &mut Term, state: &mut AppState) -> Result<()> {
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
    force_full_redraw(term, state)
}

/// Fallback when a PTY can't be created: run an interactive shell once.
async fn run_oneshot_shell(term: &mut Term, state: &mut AppState, cwd: &std::path::Path) -> Result<()> {
    restore_terminal(term, state.kbd_enhanced)?;
    println!("[Rat Commander subshell — type 'exit' to return]");
    let _ = interactive_shell().current_dir(cwd).status().await;
    resume_tui(term, state).await
}

/// Set up the terminal, returning the terminal handle and whether the enhanced
/// keyboard protocol was enabled (so key release/repeat and standalone-modifier
/// events are reported — used to live-update the editor's F-key labels while
/// Shift/Ctrl is held). It is left off where the terminal doesn't support it.
///
/// `kbd` is the enhancement capability when already known; `None` probes the
/// terminal. Probe only at startup: the probe is a query round-trip through
/// crossterm's event reader, which the suspend/resume paths must avoid (see
/// [`force_full_redraw`]) — and the answer can't change mid-session anyway.
fn setup_terminal(kbd: Option<bool>) -> Result<(Term, bool)> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let kbd = kbd.unwrap_or_else(|| supports_keyboard_enhancement().unwrap_or(false));
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static SIGINT_HITS: AtomicU32 = AtomicU32::new(0);

    extern "C" fn count_sigint(_: nix::libc::c_int) {
        SIGINT_HITS.fetch_add(1, Ordering::SeqCst);
    }

    /// While a foreground child runs, SIGINT is ignored by us (so Ctrl-C reaches
    /// only the child) and our previous disposition is restored afterward.
    #[tokio::test]
    async fn run_foreground_shields_us_from_sigint_then_restores() {
        // Install a counting SIGINT handler as the "before" disposition. Using a
        // real handler (never SIG_DFL) keeps the test process alive even if a
        // raise lands outside the shielded window.
        let prev = unsafe {
            let handler = count_sigint as extern "C" fn(nix::libc::c_int);
            nix::libc::signal(nix::libc::SIGINT, handler as nix::libc::sighandler_t)
        };
        SIGINT_HITS.store(0, Ordering::SeqCst);

        // Raise SIGINT at ourselves partway through a 400 ms child.
        let raiser = tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
            unsafe { nix::libc::raise(nix::libc::SIGINT) };
        });
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg("sleep 0.4");
        let status = run_foreground(cmd).await.expect("child ran");
        raiser.await.unwrap();

        assert!(status.success(), "the child completed");
        assert_eq!(
            SIGINT_HITS.load(Ordering::SeqCst),
            0,
            "SIGINT during the child was ignored, not delivered to our handler"
        );

        // Our handler is restored: a SIGINT now is delivered again.
        unsafe { nix::libc::raise(nix::libc::SIGINT) };
        // Give the signal a moment to be delivered.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert_eq!(
            SIGINT_HITS.load(Ordering::SeqCst),
            1,
            "run_foreground restored our SIGINT handler afterward"
        );

        unsafe { nix::libc::signal(nix::libc::SIGINT, prev) };
    }

    /// The command line must invoke the shell interactively (`-i -c <cmd>`): the
    /// `-i` is what sources the rc files and enables alias expansion, so aliases
    /// typed at the command line actually run. Program is `$SHELL`, so it is not
    /// asserted here (it varies by environment).
    #[test]
    fn command_line_runs_shell_interactively() {
        let c = command_line_shell("ll");
        let args: Vec<std::ffi::OsString> =
            c.as_std().get_args().map(|a| a.to_owned()).collect();
        let expected: Vec<std::ffi::OsString> = ["-i", "-c", "ll"]
            .iter()
            .map(std::ffi::OsString::from)
            .collect();
        assert_eq!(args, expected);
    }

    #[test]
    fn active_ssh_remote_matches_sftp_and_scp_only() {
        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        st.active = 0;
        let set = |st: &mut AppState, scheme: &str| {
            st.panels[0].cwd = crate::vfs::VfsPath {
                scheme: scheme.to_string(),
                path: "/home/user".into(),
                container: None,
            };
        };
        // A local panel drives the local subshell.
        assert!(active_ssh_remote(&st).is_none(), "local → no remote shell");
        // SFTP / SCP panels drive a remote shell (keyed by the session scheme).
        set(&mut st, "sftp-0");
        assert_eq!(active_ssh_remote(&st).as_deref(), Some("sftp-0"));
        set(&mut st, "scp-2");
        assert_eq!(active_ssh_remote(&st).as_deref(), Some("scp-2"));
        // FTP has no shell channel, so it stays on the local shell.
        set(&mut st, "ftp-1");
        assert!(active_ssh_remote(&st).is_none(), "ftp → no remote shell");
    }

    #[tokio::test]
    async fn local_backend_has_no_remote_shell() {
        let reg = crate::vfs::registry::Registry::default();
        let local = reg.local();
        assert!(local.open_shell(24, 80).await.is_err(), "local fs has no shell channel");
    }
}
