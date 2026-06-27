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
};
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::crossterm::{execute, queue};
use state::{AppState, Flow};
use std::io::{self, Stdout, Write};

type Term = Terminal<CrosstermBackend<Stdout>>;

/// Set up, run, and tear down the application.
pub async fn run() -> Result<()> {
    let (tx, mut rx) = async_bridge::channel();
    let mut state = AppState::new(tx);
    state.init().await;

    let mut term = setup_terminal()?;
    let mut events = EventStream::new();

    let result = run_loop(&mut term, &mut state, &mut rx, &mut events).await;

    restore_terminal(&mut term)?;
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

    loop {
        term.draw(|f| ui::draw(f, state))?;

        tokio::select! {
            maybe_event = events.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                        match state.handle_key(key).await {
                            Flow::Quit => break,
                            Flow::RunCommand(cmd) => run_command(term, state, &cmd).await?,
                            Flow::RunExternal { program, path } => {
                                run_external(term, state, &program, &path).await?
                            }
                            Flow::SubShell => run_subshell(term, state).await?,
                            Flow::Continue => {}
                        }
                    }
                    Some(Ok(_)) => {} // resize / mouse / other: redraw next iteration
                    Some(Err(e)) => return Err(e.into()),
                    None => break, // stdin closed
                }
            }
            Some(app_event) = rx.recv() => {
                state.apply_event(app_event).await;
            }
            _ = ticker.tick(), if state.wants_ticks() => {
                state.on_tick();
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
    restore_terminal(term)?;

    // Use the panel's directory only when it's a real local path; otherwise
    // (remote/archive panel) fall back to the process cwd.
    let panel = &state.panels[state.active];
    let cwd = if panel.cwd.scheme == "file" {
        panel.cwd.path.clone()
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
    print!("\n[Press Enter to return to rat-commander]");
    io::stdout().flush().ok();
    let mut line = String::new();
    let _ = io::stdin().read_line(&mut line);

    *term = setup_terminal()?;
    term.clear()?;
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
    restore_terminal(term)?;

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

    *term = setup_terminal()?;
    term.clear()?;
    state.reload_all().await;
    Ok(())
}

/// Ctrl-O: suspend the TUI and launch an interactive shell in the active
/// panel's directory (local). The TUI restores when the shell exits.
async fn run_subshell(term: &mut Term, state: &mut AppState) -> Result<()> {
    restore_terminal(term)?;

    let cwd = {
        let p = &state.panels[state.active];
        if p.cwd.scheme == "file" {
            p.cwd.path.clone()
        } else {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"))
        }
    };
    println!("[rat-commander subshell — type 'exit' to return]");
    let _ = interactive_shell().current_dir(&cwd).status().await;

    *term = setup_terminal()?;
    term.clear()?;
    state.reload_all().await;
    Ok(())
}

fn setup_terminal() -> Result<Term> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut term = Terminal::new(backend)?;
    term.hide_cursor()?;
    Ok(term)
}

fn restore_terminal(term: &mut Term) -> Result<()> {
    disable_raw_mode()?;
    let out = term.backend_mut();
    queue!(out, LeaveAlternateScreen, DisableMouseCapture)?;
    out.flush()?;
    term.show_cursor()?;
    Ok(())
}
