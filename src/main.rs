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
mod l10n;
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
    // (Windows) that points back at this binary.
    let edit_file = match editor_startup(
        argv.first().map(|s| s.as_os_str()),
        argv.get(1).map(|s| s.as_os_str()),
        argv.get(2).map(|s| s.as_os_str()),
    ) {
        Ok(f) => f,
        Err(()) => {
            eprintln!("usage: rc /edit <file>");
            std::process::exit(2);
        }
    };

    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("rat-commander error: {e}");
            std::process::exit(1);
        }
    };
    rt.block_on(async {
        if let Err(e) = app::run(edit_file).await {
            // Terminal is already restored by `app::run` on its way out.
            eprintln!("rat-commander error: {e}");
            std::process::exit(1);
        }
    });
}

/// Decide whether to start in editor mode from the program name (`argv0`) and the
/// first two arguments. Returns the file to edit (`Some`) or the normal
/// file-manager start (`None`); `Err(())` means `/edit` was given with no file (a
/// usage error). Editor mode triggers when invoked as `rcedit` (any extension) or
/// when the first argument is `/edit` / `--edit`.
fn editor_startup(
    argv0: Option<&std::ffi::OsStr>,
    arg1: Option<&std::ffi::OsStr>,
    arg2: Option<&std::ffi::OsStr>,
) -> Result<Option<std::path::PathBuf>, ()> {
    let invoked_as_edit = argv0
        .map(std::path::Path::new)
        .and_then(|p| p.file_stem())
        .map(|s| s.eq_ignore_ascii_case("rcedit"))
        .unwrap_or(false);
    if invoked_as_edit {
        return Ok(arg1.map(std::path::PathBuf::from));
    }
    match arg1.and_then(|s| s.to_str()) {
        Some("/edit" | "--edit") => arg2.map(|f| Some(std::path::PathBuf::from(f))).ok_or(()),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::editor_startup;
    use std::ffi::OsStr;
    use std::path::PathBuf;

    fn os(s: &str) -> &OsStr {
        OsStr::new(s)
    }

    #[test]
    fn rc_without_edit_starts_normally() {
        assert_eq!(editor_startup(Some(os("rc")), None, None), Ok(None));
        assert_eq!(editor_startup(Some(os("/usr/bin/rc")), Some(os("somedir")), None), Ok(None));
    }

    #[test]
    fn edit_flag_takes_the_following_file() {
        assert_eq!(
            editor_startup(Some(os("rc")), Some(os("/edit")), Some(os("notes.txt"))),
            Ok(Some(PathBuf::from("notes.txt")))
        );
        assert_eq!(
            editor_startup(Some(os("rc")), Some(os("--edit")), Some(os("a.rs"))),
            Ok(Some(PathBuf::from("a.rs")))
        );
    }

    #[test]
    fn edit_flag_without_a_file_is_a_usage_error() {
        assert_eq!(editor_startup(Some(os("rc")), Some(os("/edit")), None), Err(()));
    }

    #[test]
    fn rcedit_shim_treats_first_arg_as_the_file() {
        // Plain name, an absolute path, and `.exe`/`.cmd` extensions all count.
        // (Backslash paths only parse as paths on Windows, so they're not tested
        // here — the basename stem is what matters.)
        for argv0 in ["rcedit", "/usr/bin/rcedit", "rcedit.exe", "rcedit.cmd"] {
            assert_eq!(
                editor_startup(Some(os(argv0)), Some(os("file.txt")), None),
                Ok(Some(PathBuf::from("file.txt"))),
                "argv0={argv0}"
            );
        }
        // `rcedit` with no file falls back to the normal start.
        assert_eq!(editor_startup(Some(os("rcedit")), None, None), Ok(None));
    }
}
