//! rat-commander — a self-contained Norton/Midnight-Commander-style file
//! manager built on Ratatui.

#![allow(dead_code)]

mod app;
mod config;
mod details;
mod diff;
mod disk;
mod drive;
mod editor;
mod flash;
mod mount;
mod ops;
mod panel;
mod proc;
mod rename;
mod shell;
mod syntax;
mod ui;
mod usermenu;
mod util;
mod vfs;
mod viewer;

fn main() {
    // Privileged flash helper: `rc --flash-write <device> <image>` writes the
    // image to the device (re-invoked through `sudo` by the flasher), reporting
    // committed-byte counts on stdout. It does no TUI / async work.
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some(flash::FLASH_WRITE_FLAG) => {
            let code = match (args.next(), args.next()) {
                (Some(dev), Some(img)) => flash::helper_main(&dev, &img),
                _ => {
                    eprintln!("{} requires <device> <image>", flash::FLASH_WRITE_FLAG);
                    2
                }
            };
            std::process::exit(code);
        }
        Some(flash::IMAGE_READ_FLAG) => {
            let code = match args.next() {
                Some(dev) => flash::image_helper_main(&dev),
                None => {
                    eprintln!("{} requires <device>", flash::IMAGE_READ_FLAG);
                    2
                }
            };
            std::process::exit(code);
        }
        _ => {}
    }

    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("rat-commander error: {e}");
            std::process::exit(1);
        }
    };
    rt.block_on(async {
        if let Err(e) = app::run().await {
            // Terminal is already restored by `app::run` on its way out.
            eprintln!("rat-commander error: {e}");
            std::process::exit(1);
        }
    });
}
