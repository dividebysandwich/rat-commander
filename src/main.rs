//! rat-commander — a self-contained Norton/Midnight-Commander-style file
//! manager built on Ratatui.

// The codebase is built in phases (see the implementation plan). Several VFS,
// selection, and config APIs are intentionally defined ahead of the phase that
// consumes them, so dead-code is allowed crate-wide during the buildout and
// will be tightened as later phases land.
#![allow(dead_code)]

mod app;
mod ops;
mod panel;
mod ui;
mod util;
mod vfs;

#[tokio::main]
async fn main() {
    if let Err(e) = app::run().await {
        // Terminal is already restored by `app::run` on its way out.
        eprintln!("rat-commander error: {e}");
        std::process::exit(1);
    }
}
