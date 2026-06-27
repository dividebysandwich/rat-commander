//! rat-commander — a self-contained Norton/Midnight-Commander-style file
//! manager built on Ratatui.

#![allow(dead_code)]

mod app;
mod config;
mod editor;
mod ops;
mod shell;
mod panel;
mod ui;
mod usermenu;
mod util;
mod vfs;
mod viewer;

#[tokio::main]
async fn main() {
    if let Err(e) = app::run().await {
        // Terminal is already restored by `app::run` on its way out.
        eprintln!("rat-commander error: {e}");
        std::process::exit(1);
    }
}
