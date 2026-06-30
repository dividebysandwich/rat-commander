//! Windows drive-letter support.
//!
//! Enumerating drive letters needs the Win32 API; the string helpers
//! (`drive_of` / `drive_root`) are platform-agnostic so they can be unit-tested
//! anywhere. On non-Windows systems there are simply no drives.

use std::path::{Path, PathBuf};

/// The drive letter at the start of `path` (e.g. `C:\Users` → `'C'`), upper-cased.
/// `None` when the path is not drive-letter-rooted (e.g. a Unix path).
pub fn drive_of(path: &Path) -> Option<char> {
    let s = path.to_string_lossy();
    let mut chars = s.chars();
    let first = chars.next()?;
    if first.is_ascii_alphabetic() && chars.next() == Some(':') {
        Some(first.to_ascii_uppercase())
    } else {
        None
    }
}

/// The root path of a drive letter, e.g. `'C'` → `C:\`.
pub fn drive_root(letter: char) -> PathBuf {
    PathBuf::from(format!("{}:\\", letter.to_ascii_uppercase()))
}

/// The drive letters currently present on the system (`A`..`Z`). Empty on
/// non-Windows platforms.
#[cfg(windows)]
pub fn available_drives() -> Vec<char> {
    let mask = unsafe { GetLogicalDrives() };
    (0u32..26)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| (b'A' + i as u8) as char)
        .collect()
}

#[cfg(not(windows))]
pub fn available_drives() -> Vec<char> {
    Vec::new()
}

// `GetLogicalDrives` returns a bitmask of present drive letters (bit 0 = A).
#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetLogicalDrives() -> u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_of_parses_letter() {
        assert_eq!(drive_of(Path::new("C:\\Users\\bob")), Some('C'));
        assert_eq!(drive_of(Path::new("d:\\")), Some('D'));
        assert_eq!(drive_of(Path::new("Z:")), Some('Z'));
        // Not drive-rooted:
        assert_eq!(drive_of(Path::new("/home/bob")), None);
        assert_eq!(drive_of(Path::new("relative\\path")), None);
        assert_eq!(drive_of(Path::new("1:\\")), None);
    }

    #[test]
    fn drive_root_builds_root_path() {
        assert_eq!(drive_root('c'), PathBuf::from("C:\\"));
        assert_eq!(drive_root('Z'), PathBuf::from("Z:\\"));
    }

    #[cfg(not(windows))]
    #[test]
    fn no_drives_off_windows() {
        assert!(available_drives().is_empty());
    }
}
