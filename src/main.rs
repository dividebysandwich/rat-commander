//! rat-commander — a self-contained Norton/Midnight-Commander-style file
//! manager built on Ratatui.

#![allow(dead_code)]

mod app;
mod config;
mod console;
mod details;
mod diff;
mod disk;
mod drive;
mod editor;
mod ext;
mod flash;
mod l10n;
mod mount;
mod net;
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
    let argv: Vec<std::ffi::OsString> = std::env::args_os().collect();

    // Privileged flash helper: `rc --flash-write <device> <image>` writes the
    // image to the device (re-invoked through `sudo` by the flasher), reporting
    // committed-byte counts on stdout. It does no TUI / async work.
    match argv.get(1).and_then(|s| s.to_str()) {
        Some(flash::FLASH_WRITE_FLAG) => {
            let code = match (argv.get(2), argv.get(3)) {
                (Some(dev), Some(img)) => {
                    flash::helper_main(&dev.to_string_lossy(), &img.to_string_lossy())
                }
                _ => {
                    eprintln!("{} requires <device> <image>", flash::FLASH_WRITE_FLAG);
                    2
                }
            };
            std::process::exit(code);
        }
        Some(flash::IMAGE_READ_FLAG) => {
            let code = match argv.get(2) {
                Some(dev) => flash::image_helper_main(&dev.to_string_lossy()),
                None => {
                    eprintln!("{} requires <device>", flash::IMAGE_READ_FLAG);
                    2
                }
            };
            std::process::exit(code);
        }
        _ => {}
    }

    // Start directly in the editor when invoked as `rc /edit <file>` (or
    // `--edit`), or through the `rcedit` shim — a symlink (Unix) or `.cmd`
    // (Windows) that points back at this binary. With no file, the editor
    // opens on a fresh, unnamed buffer whose first save prompts for a name.
    let startup = editor_startup(
        argv.first().map(|s| s.as_os_str()),
        argv.get(1).map(|s| s.as_os_str()),
        argv.get(2).map(|s| s.as_os_str()),
    );

    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Rat Commander error: {e}");
            std::process::exit(1);
        }
    };
    rt.block_on(async {
        if let Err(e) = app::run(startup).await {
            // Terminal is already restored by `app::run` on its way out.
            eprintln!("Rat Commander error: {e}");
            std::process::exit(1);
        }
    });
}

/// How the program should start, decided from the command line.
#[derive(Debug, PartialEq, Eq)]
pub enum Startup {
    /// Normal file-manager start (the two panels).
    Panels,
    /// Open the editor straight on this file, then exit when it closes.
    Edit(std::path::PathBuf),
    /// Open the editor on a fresh, unnamed buffer (no file was given); its
    /// first save is routed through "Save as" so the user picks a filename.
    EditNew,
}

/// Decide how to start from the program name (`argv0`) and the first two
/// arguments. Editor mode triggers when invoked as `rcedit` (any extension) or
/// when the first argument is `/edit` / `--edit`; a following file name selects
/// [`Startup::Edit`], and its absence selects [`Startup::EditNew`] (a blank
/// buffer). Anything else is the normal [`Startup::Panels`] start.
fn editor_startup(
    argv0: Option<&std::ffi::OsStr>,
    arg1: Option<&std::ffi::OsStr>,
    arg2: Option<&std::ffi::OsStr>,
) -> Startup {
    let edit_or_new = |file: Option<&std::ffi::OsStr>| match file {
        Some(f) => Startup::Edit(std::path::PathBuf::from(f)),
        None => Startup::EditNew,
    };
    let invoked_as_edit = argv0
        .map(std::path::Path::new)
        .and_then(|p| p.file_stem())
        .map(|s| s.eq_ignore_ascii_case("rcedit"))
        .unwrap_or(false);
    if invoked_as_edit {
        return edit_or_new(arg1);
    }
    match arg1.and_then(|s| s.to_str()) {
        Some("/edit" | "--edit") => edit_or_new(arg2),
        _ => Startup::Panels,
    }
}

#[cfg(test)]
mod tests {
    use super::{Startup, editor_startup};
    use std::ffi::OsStr;
    use std::path::PathBuf;

    fn os(s: &str) -> &OsStr {
        OsStr::new(s)
    }

    #[test]
    fn rc_without_edit_starts_normally() {
        assert_eq!(editor_startup(Some(os("rc")), None, None), Startup::Panels);
        assert_eq!(
            editor_startup(Some(os("/usr/bin/rc")), Some(os("somedir")), None),
            Startup::Panels
        );
    }

    #[test]
    fn edit_flag_takes_the_following_file() {
        assert_eq!(
            editor_startup(Some(os("rc")), Some(os("/edit")), Some(os("notes.txt"))),
            Startup::Edit(PathBuf::from("notes.txt"))
        );
        assert_eq!(
            editor_startup(Some(os("rc")), Some(os("--edit")), Some(os("a.rs"))),
            Startup::Edit(PathBuf::from("a.rs"))
        );
    }

    #[test]
    fn edit_flag_without_a_file_opens_a_blank_buffer() {
        assert_eq!(editor_startup(Some(os("rc")), Some(os("/edit")), None), Startup::EditNew);
        assert_eq!(editor_startup(Some(os("rc")), Some(os("--edit")), None), Startup::EditNew);
    }

    #[test]
    fn rcedit_shim_treats_first_arg_as_the_file() {
        // Plain name, an absolute path, and `.exe`/`.cmd` extensions all count.
        // (Backslash paths only parse as paths on Windows, so they're not tested
        // here — the basename stem is what matters.)
        for argv0 in ["rcedit", "/usr/bin/rcedit", "rcedit.exe", "rcedit.cmd"] {
            assert_eq!(
                editor_startup(Some(os(argv0)), Some(os("file.txt")), None),
                Startup::Edit(PathBuf::from("file.txt")),
                "argv0={argv0}"
            );
        }
        // `rcedit` with no file opens a blank buffer (like `rc /edit`).
        assert_eq!(editor_startup(Some(os("rcedit")), None, None), Startup::EditNew);
    }
}
