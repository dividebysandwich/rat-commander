//! Per-panel marked-file set and wildcard select/unselect-group support.

use crate::vfs::VfsEntry;
use globset::{Glob, GlobSetBuilder};
use std::collections::HashSet;

/// The set of marked (tagged) file names within a single directory listing.
/// Keyed by name because the listing is re-sorted/reloaded frequently.
#[derive(Debug, Default)]
pub struct Selection {
    marked: HashSet<String>,
}

impl Selection {
    pub fn new() -> Self {
        Selection::default()
    }

    pub fn clear(&mut self) {
        self.marked.clear();
    }

    pub fn is_marked(&self, name: &str) -> bool {
        self.marked.contains(name)
    }

    pub fn toggle(&mut self, name: &str) {
        if !self.marked.remove(name) {
            self.marked.insert(name.to_string());
        }
    }

    pub fn count(&self) -> usize {
        self.marked.len()
    }

    pub fn is_empty(&self) -> bool {
        self.marked.is_empty()
    }

    /// Names that are both marked and still present in `entries`.
    pub fn marked_names<'a>(&self, entries: &'a [VfsEntry]) -> Vec<&'a str> {
        entries
            .iter()
            .filter(|e| e.name != ".." && self.marked.contains(&e.name))
            .map(|e| e.name.as_str())
            .collect()
    }

    /// Drop marks whose names no longer appear in the listing.
    pub fn retain_existing(&mut self, entries: &[VfsEntry]) {
        let present: HashSet<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        self.marked.retain(|m| present.contains(m.as_str()));
    }

    /// Mark every file (not directories, not `..`) matching `pattern`.
    /// Returns the number newly marked, or an error if the pattern is invalid.
    pub fn select_group(
        &mut self,
        entries: &[VfsEntry],
        pattern: &str,
        files_only: bool,
    ) -> Result<usize, String> {
        let set = build_globset(pattern)?;
        let mut n = 0;
        for e in entries {
            if e.name == ".." {
                continue;
            }
            if files_only && e.kind.is_dir() {
                continue;
            }
            if set.is_match(&e.name) && self.marked.insert(e.name.clone()) {
                n += 1;
            }
        }
        Ok(n)
    }

    /// Unmark every entry matching `pattern`. Returns the number removed.
    pub fn unselect_group(&mut self, entries: &[VfsEntry], pattern: &str) -> Result<usize, String> {
        let set = build_globset(pattern)?;
        let mut n = 0;
        for e in entries {
            if set.is_match(&e.name) && self.marked.remove(&e.name) {
                n += 1;
            }
        }
        Ok(n)
    }
}

/// Build a matcher from one or more `;`/`,`-separated shell wildcards.
fn build_globset(pattern: &str) -> Result<globset::GlobSet, String> {
    let mut builder = GlobSetBuilder::new();
    let mut any = false;
    for part in pattern.split([';', ',']).map(str::trim).filter(|p| !p.is_empty()) {
        let glob = Glob::new(part).map_err(|e| e.to_string())?;
        builder.add(glob);
        any = true;
    }
    if !any {
        return Err("empty pattern".to_string());
    }
    builder.build().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::VfsKind;

    fn ent(name: &str, kind: VfsKind) -> VfsEntry {
        VfsEntry {
            name: name.to_string(),
            kind,
            size: 0,
            mtime: None,
            atime: None,
            ctime: None,
            inode: None,
            mode: None,
            uid: None,
            gid: None,
            symlink_target: None,
        }
    }

    #[test]
    fn select_and_unselect_group_by_wildcard() {
        let entries = vec![
            ent("..", VfsKind::Dir),
            ent("a.txt", VfsKind::File),
            ent("b.txt", VfsKind::File),
            ent("c.rs", VfsKind::File),
            ent("docs", VfsKind::Dir),
        ];
        let mut sel = Selection::new();

        let n = sel.select_group(&entries, "*.txt", true).unwrap();
        assert_eq!(n, 2);
        assert!(sel.is_marked("a.txt"));
        assert!(sel.is_marked("b.txt"));
        assert!(!sel.is_marked("c.rs"));
        // files_only excludes the directory even if it matched.
        assert!(!sel.is_marked("docs"));

        let removed = sel.unselect_group(&entries, "a.*").unwrap();
        assert_eq!(removed, 1);
        assert!(!sel.is_marked("a.txt"));
        assert!(sel.is_marked("b.txt"));
    }

    #[test]
    fn invalid_pattern_errors() {
        let entries = vec![ent("x", VfsKind::File)];
        let mut sel = Selection::new();
        assert!(sel.select_group(&entries, "[", true).is_err());
    }
}
