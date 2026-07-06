//! A single file panel: current directory, listing, cursor, selection, view.

pub mod render;
pub mod selection;
pub mod sort;
pub mod tree;

use crate::util::Result;
use crate::vfs::{DiskUsage, Vfs, VfsEntry, VfsKind, VfsPath};
use ratatui::layout::Rect;
use selection::Selection;
use sort::SortConfig;
use std::sync::Arc;
use tree::TreeState;

/// Geometry recorded at render time so mouse clicks can be mapped to entries.
#[derive(Clone, Copy)]
pub struct PanelHit {
    /// The whole panel rect (including border) — identifies which panel a click
    /// landed in.
    pub area: Rect,
    /// The region in which entry rows are drawn.
    pub body: Rect,
    pub brief: bool,
    pub offset: usize,
    pub columns: usize,
    /// Column height in the Brief grid (entries per screen column); used for the
    /// column-major click mapping.
    pub rows: usize,
    pub cell_w: u16,
}

impl PanelHit {
    fn contains(rect: Rect, col: u16, row: u16) -> bool {
        col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
    }

    /// True if (col,row) is anywhere within the panel.
    pub fn in_panel(&self, col: u16, row: u16) -> bool {
        Self::contains(self.area, col, row)
    }

    /// The entry index at screen (col,row), bounded by `len`, if it lands on a row.
    pub fn index_at(&self, col: u16, row: u16, len: usize) -> Option<usize> {
        if !Self::contains(self.body, col, row) {
            return None;
        }
        let r = (row - self.body.y) as usize;
        let idx = if self.brief {
            // Column-major: each screen column holds `rows` consecutive entries.
            let cw = self.cell_w.max(1);
            let c = ((col - self.body.x) / cw) as usize;
            self.offset + c * self.rows.max(1) + r
        } else {
            self.offset + r
        };
        (idx < len).then_some(idx)
    }
}

/// How the listing columns are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ViewFormat {
    /// Name, size, mtime columns (single column of rows).
    #[default]
    Full,
    /// Just names, in multiple columns.
    Brief,
    /// No own listing: shows details about the item under the *other* panel's
    /// cursor (file stats, or a directory/selection size tallied in background).
    Details,
    /// A navigable directory tree rooted at the backend/drive root; Enter opens
    /// a branch and points the *other* panel at the selected directory.
    Tree,
}

impl ViewFormat {
    /// Cycle Full → Brief → Details → Tree → Full (Ctrl-W).
    pub fn toggle(self) -> Self {
        match self {
            ViewFormat::Full => ViewFormat::Brief,
            ViewFormat::Brief => ViewFormat::Details,
            ViewFormat::Details => ViewFormat::Tree,
            ViewFormat::Tree => ViewFormat::Full,
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
    /// When set, the panel shows find-file results (full paths) instead of a
    /// directory listing. Parallel to `entries`.
    pub result_paths: Option<Vec<VfsPath>>,
    /// Capacity of the volume holding `cwd`, shown on the bottom border. Updated
    /// on each reload; `None` for backends that can't report it.
    pub disk: Option<DiskUsage>,
    /// Layout geometry from the last render, for mapping mouse clicks to entries.
    pub hit: Option<PanelHit>,
    /// Number of entries visible on screen, set by the renderer; drives PgUp/PgDn.
    pub page: usize,
    /// Brief-view grid geometry from the last render, for column-major arrow
    /// navigation: `cols` columns of `brief_rows` entries each (entries fill
    /// top-to-bottom, column by column). `cols` is 1 in the Full/Details views.
    pub cols: usize,
    pub brief_rows: usize,
    /// The directory tree shown when `format == ViewFormat::Tree`. Built lazily
    pub tree: Option<TreeState>,
    /// Caret screen position of the quick-search input when this panel is
    /// rendering an active quick search (set by the renderer, read by the root
    /// draw to place the terminal cursor). `None` otherwise.
    pub quick_caret: Option<ratatui::layout::Position>,
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
            result_paths: None,
            disk: None,
            hit: None,
            page: 1,
            cols: 1,
            brief_rows: 1,
            tree: None,
            quick_caret: None,
        }
    }

    /// (Re)build the directory tree for the current directory. Called when the
    /// panel switches to Tree view, and when a Tree-view panel changes directory.
    pub async fn build_tree(&mut self) {
        self.tree = Some(TreeState::build(&self.backend, &self.cwd).await);
    }

    /// Toggle (expand/collapse) the tree node under the cursor and return its
    /// path, so the caller can point the other panel at it. `None` when not in
    /// Tree view or the tree is empty.
    pub async fn tree_toggle(&mut self) -> Option<VfsPath> {
        if self.format != ViewFormat::Tree {
            return None;
        }
        // Clone the backend handle to avoid borrowing `self` twice.
        let backend = self.backend.clone();
        let tree = self.tree.as_mut()?;
        let path = tree.selected_path()?;
        tree.toggle(&backend).await;
        Some(path)
    }

    /// Whether the panel is currently showing the directory tree.
    pub fn is_tree(&self) -> bool {
        self.format == ViewFormat::Tree
    }

    /// Show an explicit list of result entries (find-file panelization).
    pub fn set_results(&mut self, entries: Vec<VfsEntry>, paths: Vec<VfsPath>) {
        self.entries = entries;
        self.result_paths = Some(paths);
        self.cursor = 0;
        self.offset = 0;
        self.selection.clear();
        self.error = None;
    }

    pub fn is_panelized(&self) -> bool {
        self.result_paths.is_some()
    }

    /// Reload the current directory, preserving the cursor on a named entry
    /// when possible. Adds a synthetic `..` entry unless at the backend root.
    pub async fn reload(&mut self) -> Result<()> {
        self.reload_keeping(None).await
    }

    /// Reload, then try to place the cursor on `focus_name` (e.g. the directory
    /// we just came up out of).
    pub async fn reload_keeping(&mut self, focus_name: Option<&str>) -> Result<()> {
        // Reloading leaves any find-file panelization.
        self.result_paths = None;
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

        // Refresh the volume's capacity for the bottom-border readout.
        self.disk = self.backend.disk_usage(&self.cwd).await.ok().flatten();

        // Restore cursor.
        self.cursor = prev_name
            .and_then(|n| self.entries.iter().position(|e| e.name == n))
            .unwrap_or(0);
        self.clamp_cursor();
        Ok(())
    }

    /// Navigate to `newcwd` (on `backend`), focusing `focus_name` once the
    /// listing loads. The move is atomic: if the target can't be read (e.g. a
    /// directory the user has no permission to list), the panel reverts to where
    /// it was and returns `false` — so navigation into an unusable directory
    /// simply does not happen.
    pub async fn try_enter(
        &mut self,
        newcwd: VfsPath,
        backend: Arc<dyn Vfs>,
        focus_name: Option<&str>,
    ) -> bool {
        let prev_cwd = std::mem::replace(&mut self.cwd, newcwd);
        let prev_backend = std::mem::replace(&mut self.backend, backend);
        let prev_selection = std::mem::replace(&mut self.selection, Selection::new());

        let _ = self.reload_keeping(focus_name).await;
        let ok = if self.error.is_some() {
            // Couldn't list the target: undo the move and stay put.
            self.cwd = prev_cwd;
            self.backend = prev_backend;
            self.selection = prev_selection;
            self.error = None;
            let _ = self.reload().await;
            false
        } else {
            true
        };
        // A Tree-view panel that changed directory (e.g. via `cd`) re-roots its
        // tree so it keeps reflecting where the panel is.
        if self.format == ViewFormat::Tree {
            self.build_tree().await;
        }
        ok
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
        if self.format == ViewFormat::Tree {
            if let Some(tree) = self.tree.as_mut() {
                tree.move_cursor(delta);
            }
            return;
        }
        if self.entries.is_empty() {
            return;
        }
        let max = self.entries.len() as isize - 1;
        let next = (self.cursor as isize + delta).clamp(0, max);
        self.cursor = next as usize;
    }

    /// Whether arrow Left/Right should move between Brief-view columns (rather
    /// than editing the command line): true only in a multi-column Brief view.
    pub fn brief_grid(&self) -> bool {
        self.format == ViewFormat::Brief && self.cols > 1
    }

    pub fn move_home(&mut self) {
        if self.format == ViewFormat::Tree {
            if let Some(tree) = self.tree.as_mut() {
                tree.move_home();
            }
            return;
        }
        self.cursor = 0;
    }

    pub fn move_end(&mut self) {
        if self.format == ViewFormat::Tree {
            if let Some(tree) = self.tree.as_mut() {
                tree.move_end();
            }
            return;
        }
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
        // Find-file results: operate on the stored full paths (never "..").
        if let Some(paths) = &self.result_paths {
            if !self.selection.is_empty() {
                return self
                    .entries
                    .iter()
                    .zip(paths)
                    .filter(|(e, _)| e.name != ".." && self.selection.is_marked(&e.name))
                    .map(|(_, p)| p.clone())
                    .collect();
            }
            return match self.current_entry() {
                Some(e) if e.name != ".." => paths.get(self.cursor).cloned().into_iter().collect(),
                _ => Vec::new(),
            };
        }
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
        // In find-file results: ".." leaves the result view back to normal
        // browsing; any other entry jumps to that file's directory.
        if let Some(paths) = &self.result_paths {
            let e = self.current_entry()?;
            if e.name == ".." {
                return Some((self.cwd.clone(), None));
            }
            let path = paths.get(self.cursor)?;
            let parent = path.parent()?;
            return Some((parent, Some(path.file_name())));
        }
        let e = self.current_entry()?;
        if e.name == ".." {
            // When stepping out of an archive root, focus the archive file.
            let from = if self.cwd.is_archive_root() {
                self.cwd.container_name().unwrap_or_default()
            } else {
                self.cwd.file_name()
            };
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
        symlink_broken: false,
    }
}
