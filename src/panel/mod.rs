//! A single file panel: current directory, listing, cursor, selection, view.

pub mod render;
pub mod selection;
pub mod sort;

use crate::util::Result;
use crate::vfs::{Vfs, VfsEntry, VfsKind, VfsPath};
use selection::Selection;
use sort::SortConfig;
use std::sync::Arc;

/// How the listing columns are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewFormat {
    /// Name, size, mtime columns (single column of rows).
    Full,
    /// Just names, in multiple columns.
    Brief,
}

impl ViewFormat {
    pub fn toggle(self) -> Self {
        match self {
            ViewFormat::Full => ViewFormat::Brief,
            ViewFormat::Brief => ViewFormat::Full,
        }
    }
}

/// One of the two panels.
pub struct Panel {
    pub cwd: VfsPath,
    pub backend: Arc<dyn Vfs>,
    pub entries: Vec<VfsEntry>,
    pub cursor: usize,
    /// First visible row (scroll offset), maintained by the renderer.
    pub offset: usize,
    pub selection: Selection,
    pub format: ViewFormat,
    pub sort: SortConfig,
    /// Last load error, shown in place of the listing.
    pub error: Option<String>,
}

impl Panel {
    pub fn new(backend: Arc<dyn Vfs>, cwd: VfsPath) -> Self {
        Panel {
            cwd,
            backend,
            entries: Vec::new(),
            cursor: 0,
            offset: 0,
            selection: Selection::new(),
            format: ViewFormat::Full,
            sort: SortConfig::default(),
            error: None,
        }
    }

    /// Reload the current directory, preserving the cursor on a named entry
    /// when possible. Adds a synthetic `..` entry unless at the backend root.
    pub async fn reload(&mut self) -> Result<()> {
        self.reload_keeping(None).await
    }

    /// Reload, then try to place the cursor on `focus_name` (e.g. the directory
    /// we just came up out of).
    pub async fn reload_keeping(&mut self, focus_name: Option<&str>) -> Result<()> {
        let prev_name = focus_name
            .map(str::to_string)
            .or_else(|| self.current_entry().map(|e| e.name.clone()));

        match self.backend.read_dir(&self.cwd).await {
            Ok(mut entries) => {
                if self.cwd.parent().is_some() {
                    entries.push(parent_entry());
                }
                self.sort.apply(&mut entries);
                self.selection.retain_existing(&entries);
                self.entries = entries;
                self.error = None;
            }
            Err(e) => {
                self.entries.clear();
                self.error = Some(e.to_string());
            }
        }

        // Restore cursor.
        self.cursor = prev_name
            .and_then(|n| self.entries.iter().position(|e| e.name == n))
            .unwrap_or(0);
        self.clamp_cursor();
        Ok(())
    }

    pub fn current_entry(&self) -> Option<&VfsEntry> {
        self.entries.get(self.cursor)
    }

    fn clamp_cursor(&mut self) {
        if self.entries.is_empty() {
            self.cursor = 0;
        } else if self.cursor >= self.entries.len() {
            self.cursor = self.entries.len() - 1;
        }
    }

    pub fn move_cursor(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let max = self.entries.len() as isize - 1;
        let next = (self.cursor as isize + delta).clamp(0, max);
        self.cursor = next as usize;
    }

    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    pub fn move_end(&mut self) {
        if !self.entries.is_empty() {
            self.cursor = self.entries.len() - 1;
        }
    }

    /// Re-apply the sort config to the existing listing (no I/O).
    pub fn resort(&mut self) {
        let name = self.current_entry().map(|e| e.name.clone());
        self.sort.apply(&mut self.entries);
        self.cursor = name
            .and_then(|n| self.entries.iter().position(|e| e.name == n))
            .unwrap_or(0);
        self.clamp_cursor();
    }

    /// Toggle the mark on the entry under the cursor and advance (mc Insert).
    pub fn toggle_mark_and_advance(&mut self) {
        if let Some(e) = self.current_entry()
            && e.name != ".."
        {
            let name = e.name.clone();
            self.selection.toggle(&name);
        }
        self.move_cursor(1);
    }

    /// The paths an operation should act on: the marked set if non-empty,
    /// otherwise the entry under the cursor (never `..`).
    pub fn operation_targets(&self) -> Vec<VfsPath> {
        if !self.selection.is_empty() {
            self.selection
                .marked_names(&self.entries)
                .into_iter()
                .map(|n| self.cwd.join(n))
                .collect()
        } else if let Some(e) = self.current_entry() {
            if e.name != ".." {
                vec![self.cwd.join(&e.name)]
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        }
    }

    /// Enter the directory (or follow `..`) under the cursor. Returns true if
    /// navigation happened (caller should reload).
    pub fn target_dir_under_cursor(&self) -> Option<(VfsPath, Option<String>)> {
        let e = self.current_entry()?;
        if e.name == ".." {
            let from = self.cwd.file_name();
            self.cwd.parent().map(|p| (p, Some(from)))
        } else if e.kind == VfsKind::Dir {
            Some((self.cwd.join(&e.name), None))
        } else {
            None
        }
    }
}

/// The synthetic `..` entry appended to non-root listings.
fn parent_entry() -> VfsEntry {
    VfsEntry {
        name: "..".to_string(),
        kind: VfsKind::Dir,
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
