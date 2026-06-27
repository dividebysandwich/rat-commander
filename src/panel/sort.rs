//! Sort keys, toggles, and the comparator used to order a directory listing.

use crate::vfs::{VfsEntry, VfsKind};
use std::cmp::Ordering;

/// The field a listing is ordered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Unsorted,
    Name,
    Extension,
    Size,
    ModifyTime,
    AccessTime,
    ChangeTime,
    Inode,
}

impl SortKey {
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Unsorted => "Unsorted",
            SortKey::Name => "Name",
            SortKey::Extension => "Extension",
            SortKey::Size => "Size",
            SortKey::ModifyTime => "Modify time",
            SortKey::AccessTime => "Access time",
            SortKey::ChangeTime => "Change time",
            SortKey::Inode => "Inode",
        }
    }

    /// All keys, in menu order.
    pub const ALL: [SortKey; 8] = [
        SortKey::Unsorted,
        SortKey::Name,
        SortKey::Extension,
        SortKey::Size,
        SortKey::ModifyTime,
        SortKey::AccessTime,
        SortKey::ChangeTime,
        SortKey::Inode,
    ];
}

/// The full sort configuration: key plus the modifier toggles.
#[derive(Debug, Clone, Copy)]
pub struct SortConfig {
    pub key: SortKey,
    pub reverse: bool,
    pub exec_first: bool,
    pub case_sensitive: bool,
    /// Group directories before files (Midnight Commander default).
    pub dirs_first: bool,
}

impl Default for SortConfig {
    fn default() -> Self {
        SortConfig {
            key: SortKey::Name,
            reverse: false,
            exec_first: false,
            case_sensitive: false,
            dirs_first: true,
        }
    }
}

impl SortConfig {
    /// Sort a slice of entries in place according to this configuration.
    /// `..` (the parent link, represented as a `Dir` named "..") always sorts
    /// first regardless of other settings.
    pub fn apply(&self, entries: &mut [VfsEntry]) {
        entries.sort_by(|a, b| self.compare(a, b));
    }

    fn compare(&self, a: &VfsEntry, b: &VfsEntry) -> Ordering {
        // ".." pinned to the very top.
        let a_up = a.name == "..";
        let b_up = b.name == "..";
        if a_up || b_up {
            return b_up.cmp(&a_up); // a_up=true => a first
        }

        // Directories grouped first (not affected by `reverse`).
        if self.dirs_first {
            let a_dir = a.kind == VfsKind::Dir;
            let b_dir = b.kind == VfsKind::Dir;
            if a_dir != b_dir {
                return b_dir.cmp(&a_dir);
            }
        }

        // Executables-first among regular files (also not reversed).
        if self.exec_first {
            let ae = a.is_executable();
            let be = b.is_executable();
            if ae != be {
                return be.cmp(&ae);
            }
        }

        let ord = self.compare_key(a, b);
        if self.reverse { ord.reverse() } else { ord }
    }

    fn compare_key(&self, a: &VfsEntry, b: &VfsEntry) -> Ordering {
        match self.key {
            SortKey::Unsorted => Ordering::Equal,
            SortKey::Name => self.cmp_name(&a.name, &b.name),
            SortKey::Extension => self
                .cmp_name(a.extension(), b.extension())
                .then_with(|| self.cmp_name(&a.name, &b.name)),
            SortKey::Size => a.size.cmp(&b.size).then_with(|| self.cmp_name(&a.name, &b.name)),
            SortKey::ModifyTime => a.mtime.cmp(&b.mtime).then_with(|| self.cmp_name(&a.name, &b.name)),
            SortKey::AccessTime => a.atime.cmp(&b.atime).then_with(|| self.cmp_name(&a.name, &b.name)),
            SortKey::ChangeTime => a.ctime.cmp(&b.ctime).then_with(|| self.cmp_name(&a.name, &b.name)),
            SortKey::Inode => a.inode.cmp(&b.inode).then_with(|| self.cmp_name(&a.name, &b.name)),
        }
    }

    fn cmp_name(&self, a: &str, b: &str) -> Ordering {
        if self.case_sensitive {
            a.cmp(b)
        } else {
            a.to_lowercase().cmp(&b.to_lowercase())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn ent(name: &str, kind: VfsKind, size: u64, mode: u32) -> VfsEntry {
        VfsEntry {
            name: name.to_string(),
            kind,
            size,
            mtime: Some(UNIX_EPOCH + Duration::from_secs(size)),
            atime: None,
            ctime: None,
            inode: Some(size),
            mode: Some(mode),
            uid: None,
            gid: None,
            symlink_target: None,
            symlink_broken: false,
        }
    }

    fn names(v: &[VfsEntry]) -> Vec<String> {
        v.iter().map(|e| e.name.clone()).collect()
    }

    #[test]
    fn dirs_first_and_parent_pinned() {
        let mut v = vec![
            ent("zeta.txt", VfsKind::File, 1, 0o644),
            ent("alpha", VfsKind::Dir, 0, 0o755),
            ent("..", VfsKind::Dir, 0, 0o755),
            ent("beta.txt", VfsKind::File, 2, 0o644),
        ];
        SortConfig::default().apply(&mut v);
        assert_eq!(names(&v), vec!["..", "alpha", "beta.txt", "zeta.txt"]);
    }

    #[test]
    fn reverse_keeps_dirs_grouped() {
        let cfg = SortConfig {
            reverse: true,
            ..Default::default()
        };
        let mut v = vec![
            ent("a.txt", VfsKind::File, 1, 0o644),
            ent("dir", VfsKind::Dir, 0, 0o755),
            ent("b.txt", VfsKind::File, 2, 0o644),
        ];
        cfg.apply(&mut v);
        assert_eq!(names(&v), vec!["dir", "b.txt", "a.txt"]);
    }

    #[test]
    fn exec_first() {
        let cfg = SortConfig {
            exec_first: true,
            ..Default::default()
        };
        let mut v = vec![
            ent("data.txt", VfsKind::File, 1, 0o644),
            ent("run.sh", VfsKind::File, 2, 0o755),
        ];
        cfg.apply(&mut v);
        assert_eq!(names(&v), vec!["run.sh", "data.txt"]);
    }

    #[test]
    fn by_size() {
        let cfg = SortConfig {
            key: SortKey::Size,
            dirs_first: false,
            ..Default::default()
        };
        let mut v = vec![
            ent("big", VfsKind::File, 100, 0o644),
            ent("small", VfsKind::File, 5, 0o644),
        ];
        cfg.apply(&mut v);
        assert_eq!(names(&v), vec!["small", "big"]);
    }
}
