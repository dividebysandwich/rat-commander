//! Sort keys, toggles, and the comparator used to order a directory listing.

use crate::vfs::VfsEntry;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// The field a listing is ordered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SortKey {
    Unsorted,
    #[default]
    Name,
    Extension,
    Size,
    ModifyTime,
    AccessTime,
    ChangeTime,
    /// Birth (creation) time; entries without one sort as the oldest.
    BirthTime,
    Inode,
}

/// The full sort configuration: key plus the modifier toggles.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
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
            let a_dir = a.is_dir_like();
            let b_dir = b.is_dir_like();
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
            SortKey::ModifyTime => {
                a.mtime.cmp(&b.mtime).then_with(|| self.cmp_name(&a.name, &b.name))
            }
            SortKey::AccessTime => {
                a.atime.cmp(&b.atime).then_with(|| self.cmp_name(&a.name, &b.name))
            }
            SortKey::ChangeTime => {
                a.ctime.cmp(&b.ctime).then_with(|| self.cmp_name(&a.name, &b.name))
            }
            SortKey::BirthTime => {
                a.btime.cmp(&b.btime).then_with(|| self.cmp_name(&a.name, &b.name))
            }
            SortKey::Inode => a.inode.cmp(&b.inode).then_with(|| self.cmp_name(&a.name, &b.name)),
        }
    }

    fn cmp_name(&self, a: &str, b: &str) -> Ordering {
        if self.case_sensitive { a.cmp(b) } else { a.to_lowercase().cmp(&b.to_lowercase()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::VfsKind;
    use std::time::{Duration, UNIX_EPOCH};

    fn ent(name: &str, kind: VfsKind, size: u64, mode: u32) -> VfsEntry {
        VfsEntry {
            name: name.to_string(),
            kind,
            size,
            mtime: Some(UNIX_EPOCH + Duration::from_secs(size)),
            atime: None,
            ctime: None,
            btime: None,
            inode: Some(size),
            mode: Some(mode),
            uid: None,
            gid: None,
            symlink_target: None,
            symlink_broken: false,
            symlink_dir: false,
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
    fn symlinked_dirs_group_with_dirs() {
        let link = |name: &str, to_dir: bool| VfsEntry {
            symlink_dir: to_dir,
            ..ent(name, VfsKind::Symlink, 0, 0o777)
        };
        let mut v = vec![
            ent("b.txt", VfsKind::File, 1, 0o644),
            link("a-file-link", false),
            ent("dir", VfsKind::Dir, 0, 0o755),
            link("z-dir-link", true),
        ];
        SortConfig::default().apply(&mut v);
        assert_eq!(names(&v), vec!["dir", "z-dir-link", "a-file-link", "b.txt"]);
    }

    #[test]
    fn reverse_keeps_dirs_grouped() {
        let cfg = SortConfig { reverse: true, ..Default::default() };
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
        let cfg = SortConfig { exec_first: true, ..Default::default() };
        let mut v =
            vec![ent("data.txt", VfsKind::File, 1, 0o644), ent("run.sh", VfsKind::File, 2, 0o755)];
        cfg.apply(&mut v);
        assert_eq!(names(&v), vec!["run.sh", "data.txt"]);
    }

    #[test]
    fn by_birth_time_without_dirs_first() {
        let cfg = SortConfig { key: SortKey::BirthTime, dirs_first: false, ..Default::default() };
        let born = |name: &str, kind: VfsKind, secs: u64| VfsEntry {
            btime: Some(UNIX_EPOCH + Duration::from_secs(secs)),
            ..ent(name, kind, 0, 0o644)
        };
        let mut v = vec![
            born("new.txt", VfsKind::File, 30),
            born("dir", VfsKind::Dir, 20),
            born("old.txt", VfsKind::File, 10),
            born("..", VfsKind::Dir, 99),
        ];
        cfg.apply(&mut v);
        assert_eq!(names(&v), vec!["..", "old.txt", "dir", "new.txt"]);
    }

    #[test]
    fn by_size() {
        let cfg = SortConfig { key: SortKey::Size, dirs_first: false, ..Default::default() };
        let mut v =
            vec![ent("big", VfsKind::File, 100, 0o644), ent("small", VfsKind::File, 5, 0o644)];
        cfg.apply(&mut v);
        assert_eq!(names(&v), vec!["small", "big"]);
    }
}
