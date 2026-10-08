//! A single file panel: current directory, listing, cursor, selection, view.

pub mod icons;
pub mod render;
pub mod selection;
pub mod sort;
pub mod tabs;
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
    #[allow(dead_code)] // Brief-grid column count; the click map now uses `rows`
    pub columns: usize,
    /// Column height in the Brief grid (entries per screen column); used for the
    /// column-major click mapping.
    pub rows: usize,
    pub cell_w: u16,
    /// The thumbnail grid's layout, `(columns, cell width, cell height)`: it
    /// fills row by row, unlike the Brief grid.
    pub grid: Option<(usize, u16, u16)>,
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
        let idx = if let Some((cols, cw, ch)) = self.grid {
            // Row-major: each grid row holds `cols` entries, left to right.
            let c = ((col - self.body.x) / cw.max(1)) as usize;
            if c >= cols {
                return None;
            }
            self.offset + (r / ch.max(1) as usize) * cols + c
        } else if self.brief {
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
    /// A 3D view of the subdirectories, each drawn as a box whose volume
    /// corresponds to its total size on disk. Local directories only — the
    /// sizes come from a filesystem crawl.
    Space3d,
    /// The listing as a grid of thumbnails: images and 3D models as pictures,
    /// everything else by its type.
    Thumbs,
    /// No own listing: a live log of what changes anywhere under the *other*
    /// panel's directory. Chosen from the menu only — not part of the Alt-T
    /// cycle, since it is a tool to reach for rather than a way to list files.
    Activity,
}

impl ViewFormat {
    /// Whether the panel shows a listing of its own directory — not the Details
    /// view or the Activity log, which describe the other panel instead (so
    /// their hidden listing's cursor points at nothing anyone can see).
    pub fn has_listing(self) -> bool {
        !matches!(self, ViewFormat::Details | ViewFormat::Activity)
    }

    /// Cycle Full → Brief → Details → Tree → 3D → Thumbnails → Full (Alt-T). The Activity log
    /// steps back into the cycle at the start.
    pub fn toggle(self) -> Self {
        match self {
            ViewFormat::Full => ViewFormat::Brief,
            ViewFormat::Brief => ViewFormat::Details,
            ViewFormat::Details => ViewFormat::Tree,
            ViewFormat::Tree => ViewFormat::Space3d,
            ViewFormat::Space3d => ViewFormat::Thumbs,
            ViewFormat::Thumbs | ViewFormat::Activity => ViewFormat::Full,
        }
    }
}

/// One of the two panels.
/// What the time machine's row says: where you are in the history, and which
/// commit that is.
#[derive(Debug, Clone)]
pub struct ScrubRow {
    /// The commit, as `<short oid> <subject>`.
    pub label: String,
    /// Position counted oldest-first, so it reads the way the row is drawn.
    pub index: usize,
    pub total: usize,
}

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
    /// Visited directories behind the current one (most-recent last) and ahead of
    /// it, for back/forward navigation. `cwd` itself is never on either stack.
    pub back: Vec<VfsPath>,
    pub forward: Vec<VfsPath>,
    /// Set by the back/forward navigation while it drives [`Panel::try_enter`], so
    /// that move does not itself get recorded onto the history stacks.
    pub in_history_nav: bool,
    /// A persistent listing filter (a shell glob like `*.rs`, or plain text
    /// matched as a substring). When set, only matching entries (plus `..`) are
    /// listed. Distinct from the quick-search cursor jump. `None` = show all.
    pub filter: Option<String>,
    /// Screen rects of the clickable `◀` / `▶` history arrows on the top border,
    /// recorded at render time for mouse hit-testing (`None` when not drawn).
    pub back_arrow: Option<Rect>,
    pub fwd_arrow: Option<Rect>,
    /// The directory's Git status (branch + per-file states), gathered in the
    /// background. `None` when the directory is not in a git work tree (or is
    /// remote/archive). Read by the renderer to glyph/colour entries and label
    /// the border.
    pub git: Option<crate::git::GitStatus>,
    /// When this panel is in Details view and its preview is an image to be drawn
    /// with pixel graphics, the renderer records the target rect here so the root
    /// draw can composite the image (via `Gfx`) after the panels are laid out.
    pub preview_image_area: Option<Rect>,
    /// 3D view state, built on entering that format (mirrors `tree`).
    pub space3d: Option<crate::space3d::Space3d>,
    /// The Activity log, while this panel shows one.
    pub activity: Option<crate::activity::ActivityLog>,
    /// The thumbnail grid's pictures, while this panel shows one.
    pub thumbs: Option<crate::thumbs::ThumbCache>,
    /// Where the renderer wants each ready thumbnail composited with pixel
    /// graphics, handed to the root layer (which owns `Gfx`), like `scene_area`.
    pub thumb_cells: Vec<(Rect, std::sync::Arc<crate::thumbs::Thumb>)>,
    /// Where the 3D view wants its pixel image composited, when the terminal has
    /// graphics. Same deferred handoff as `preview_image_area`, because the
    /// panel renderer has no access to `Gfx`.
    pub scene_area: Option<Rect>,
    /// The time machine's readout, when this panel is under one. Filled by
    /// `AppState::update_timeline` in the same way `git` is filled by
    /// `update_git`, so the renderer never reaches outside the panel.
    pub scrub: Option<ScrubRow>,
    /// Where the scrub row was drawn, for click-to-seek.
    pub scrub_area: Option<Rect>,
    /// Saved positions for this panel's other tabs. Always non-empty: entry
    /// `tab` is *this* panel's own position, kept in step on every switch, so
    /// the list can be rendered without special-casing the active one.
    pub tabs: Vec<crate::panel::tabs::TabState>,
    /// Index into `tabs` of the position currently being shown.
    pub tab: usize,
    /// Screen rects of the tab strip's clickable labels, recorded at render time
    /// (`None`/empty when the strip is not drawn).
    pub tab_hits: Vec<(Rect, usize)>,
}

/// Largest number of directories kept on a panel's back/forward history stacks.
const HISTORY_MAX: usize = 128;

impl Panel {
    pub fn new(backend: Arc<dyn Vfs>, cwd: VfsPath) -> Self {
        let first_tab =
            crate::panel::tabs::TabState::new(cwd.clone(), ViewFormat::Full, SortConfig::default());
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
            back: Vec::new(),
            forward: Vec::new(),
            in_history_nav: false,
            filter: None,
            back_arrow: None,
            fwd_arrow: None,
            git: None,
            preview_image_area: None,
            space3d: None,
            activity: None,
            thumbs: None,
            thumb_cells: Vec::new(),
            scene_area: None,
            scrub: None,
            scrub_area: None,
            tabs: vec![first_tab],
            tab: 0,
            tab_hits: Vec::new(),
        }
    }

    // -- Tabs --------------------------------------------------------------

    /// This panel's current position, as a saved tab.
    pub fn capture_tab(&self) -> crate::panel::tabs::TabState {
        crate::panel::tabs::TabState {
            cwd: self.cwd.clone(),
            format: self.format,
            sort: self.sort,
            filter: self.filter.clone(),
            selection: self.selection.clone(),
            cursor: self.cursor,
            cursor_name: self.current_entry().map(|e| e.name.clone()),
            offset: self.offset,
            back: self.back.clone(),
            forward: self.forward.clone(),
        }
    }

    /// Restore a saved position. The caller is responsible for setting
    /// `backend` (resolved from `cwd`) and reloading the listing — everything
    /// derived from the directory is deliberately *not* carried in a tab.
    pub fn apply_tab(&mut self, tab: &crate::panel::tabs::TabState) {
        self.cwd = tab.cwd.clone();
        self.format = tab.format;
        self.sort = tab.sort;
        self.filter = tab.filter.clone();
        self.selection = tab.selection.clone();
        self.cursor = tab.cursor;
        self.offset = tab.offset;
        self.back = tab.back.clone();
        self.forward = tab.forward.clone();
        // Derived state that belongs to the directory we just left.
        self.entries.clear();
        self.error = None;
        self.result_paths = None;
        self.tree = None;
        self.space3d = None;
        self.scene_area = None;
        self.scrub_area = None;
        self.git = None;
        self.disk = None;
    }

    /// Fold the live position back into the tab list, so the entry for the
    /// active tab is never stale.
    pub fn sync_active_tab(&mut self) {
        let current = self.capture_tab();
        if let Some(slot) = self.tabs.get_mut(self.tab) {
            *slot = current;
        }
    }

    /// Whether there is anywhere to go back / forward (drives the arrow styling).
    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }
    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
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
    /// Whether this panel is showing the 3D view.
    pub fn is_space3d(&self) -> bool {
        self.format == ViewFormat::Space3d
    }

    /// Create the 3D view's state if it is missing.
    ///
    /// Which directory it *shows* is not this panel's own cwd — like the Details
    /// and Tree formats, the 3D view describes the other panel, and
    /// `AppState::update_space3d` points it there every loop iteration.
    pub fn build_space3d(&mut self, style: crate::config::Space3dStyle) {
        if self.space3d.is_none() {
            self.space3d = Some(crate::space3d::Space3d::new(self.cwd.path.clone()));
        }
        // Applied here as well as from `update_space3d`, so the very first frame
        // is drawn in the configured style rather than flipping to it a loop
        // iteration later.
        if let Some(sp) = self.space3d.as_mut() {
            sp.set_style(style);
        }
    }

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
    /// we just came up out of), falling back to whatever the cursor sits on now
    /// so an in-place refresh does not move it.
    pub async fn reload_keeping(&mut self, focus_name: Option<&str>) -> Result<()> {
        let prev_name =
            focus_name.map(str::to_string).or_else(|| self.current_entry().map(|e| e.name.clone()));
        self.reload_focusing(prev_name.as_deref(), 0).await
    }

    /// Reload the directory the panel is already showing. The cursor stays on
    /// its entry, and when that entry has gone (deleted, trashed or moved away)
    /// it keeps its row instead, landing on the entry that followed — so to the
    /// user the cursor does not move. Only for an unchanged directory: in a
    /// different listing the old row would be an arbitrary spot.
    pub async fn refresh(&mut self) -> Result<()> {
        let name = self.current_entry().map(|e| e.name.clone());
        self.reload_focusing(name.as_deref(), self.cursor).await
    }

    /// Reload, placing the cursor on `focus_name` if that entry exists and on
    /// row `fallback_row` (clamped) otherwise. Unlike [`Panel::reload_keeping`]
    /// there is no fallback to the current cursor: when the panel changes
    /// directory, the name it was sitting on says nothing about where the cursor
    /// belongs in the new listing (a file that happens to share the directory's
    /// name would steal it).
    async fn reload_focusing(
        &mut self,
        focus_name: Option<&str>,
        fallback_row: usize,
    ) -> Result<()> {
        // Reloading leaves any find-file panelization.
        self.result_paths = None;

        match self.backend.read_dir(&self.cwd).await {
            Ok(mut entries) => {
                if self.cwd.parent().is_some() {
                    entries.push(parent_entry());
                }
                self.sort.apply(&mut entries);
                // Prune the selection against the full (unfiltered) listing so a
                // mark on a file the filter currently hides survives toggling it.
                self.selection.retain_existing(&entries);
                // Apply the persistent listing filter (keeping `..` always).
                if let Some(pattern) = &self.filter {
                    let m = FilterMatch::new(pattern);
                    entries.retain(|e| e.name == ".." || m.matches(&e.name));
                }
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
        self.cursor = focus_name
            .and_then(|n| self.entries.iter().position(|e| e.name == n))
            .unwrap_or(fallback_row);
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
        let came_from = self.cwd.clone();
        let prev_cwd = std::mem::replace(&mut self.cwd, newcwd);
        let prev_backend = std::mem::replace(&mut self.backend, backend);
        let prev_selection = std::mem::replace(&mut self.selection, Selection::new());

        let _ = self.reload_focusing(focus_name, 0).await;
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
        // Record the directory we left onto the back stack (unless this move is
        // itself a back/forward step, which manages the stacks directly). A new
        // forward move invalidates the forward stack, browser-style.
        if ok && !self.in_history_nav && came_from != self.cwd {
            self.back.push(came_from);
            if self.back.len() > HISTORY_MAX {
                self.back.remove(0);
            }
            self.forward.clear();
        }
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
        if self.format == ViewFormat::Space3d {
            if let Some(sp) = self.space3d.as_mut() {
                sp.step(0.0, delta.signum() as f32);
            }
            return;
        }
        if self.format == ViewFormat::Tree {
            if let Some(tree) = self.tree.as_mut() {
                tree.move_cursor(delta);
            }
            return;
        }
        if let Some(log) = self.activity.as_mut() {
            log.move_cursor(delta);
            return;
        }
        if self.entries.is_empty() {
            return;
        }
        let max = self.entries.len() as isize - 1;
        let next = (self.cursor as isize + delta).clamp(0, max);
        self.cursor = next as usize;
    }

    /// How far Up/Down move the cursor: a whole row of the thumbnail grid, one
    /// entry anywhere else.
    pub fn vertical_step(&self) -> isize {
        if self.format == ViewFormat::Thumbs { self.cols.max(1) as isize } else { 1 }
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
        if let Some(log) = self.activity.as_mut() {
            log.cursor = 0;
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
        if let Some(log) = self.activity.as_mut() {
            log.move_end();
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
        self.cursor = name.and_then(|n| self.entries.iter().position(|e| e.name == n)).unwrap_or(0);
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
            if e.name != ".." { vec![self.cwd.join(&e.name)] } else { Vec::new() }
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
        } else if e.is_dir_like() {
            Some((self.cwd.join(&e.name), None))
        } else {
            None
        }
    }
}

/// A compiled panel-filter matcher: a case-insensitive shell glob when the
/// pattern uses glob metacharacters (`*?[`), otherwise a case-insensitive
/// substring match (so typing `test` shows every name containing `test`).
pub(crate) enum FilterMatch {
    Glob(globset::GlobMatcher),
    Substr(String),
}

impl FilterMatch {
    pub(crate) fn new(pattern: &str) -> Self {
        let has_meta = pattern.contains(['*', '?', '[']);
        // A plain word is matched anywhere in the name (`*word*`); a pattern with
        // metacharacters is used as written.
        let candidate = if has_meta { pattern.to_string() } else { format!("*{pattern}*") };
        match globset::GlobBuilder::new(&candidate).case_insensitive(true).build() {
            Ok(g) => FilterMatch::Glob(g.compile_matcher()),
            // A malformed glob (e.g. an unclosed `[`) falls back to substring.
            Err(_) => FilterMatch::Substr(pattern.to_lowercase()),
        }
    }

    pub(crate) fn matches(&self, name: &str) -> bool {
        match self {
            FilterMatch::Glob(g) => g.is_match(name),
            FilterMatch::Substr(s) => name.to_lowercase().contains(s.as_str()),
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
        btime: None,
        inode: None,
        mode: None,
        uid: None,
        gid: None,
        symlink_target: None,
        symlink_broken: false,
        symlink_dir: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Entering a directory starts at the top of its listing: a file inside that
    /// happens to share the directory's name must not inherit the cursor.
    #[tokio::test]
    async fn entering_a_directory_does_not_land_on_a_same_named_file() {
        let nanos =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("rc-enter-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(root.join("foo")).unwrap();
        std::fs::write(root.join("foo").join("aaa"), b"").unwrap();
        std::fs::write(root.join("foo").join("foo"), b"").unwrap();

        let backend = crate::vfs::registry::Registry::default().local();
        let mut panel = Panel::new(backend.clone(), VfsPath::local(&root));
        panel.reload().await.unwrap();
        // Put the cursor on the "foo" directory, as pressing Enter would.
        panel.cursor = panel.entries.iter().position(|e| e.name == "foo").unwrap();

        let (target, focus) = panel.target_dir_under_cursor().unwrap();
        assert!(panel.try_enter(target, backend, focus.as_deref()).await);
        assert_eq!(panel.cursor, 0, "cursor starts at the top of the new listing");
        assert_ne!(
            panel.current_entry().map(|e| e.name.as_str()),
            Some("foo"),
            "the same-named file did not steal the cursor"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// Stepping out of a directory still focuses the directory just left.
    #[tokio::test]
    async fn leaving_a_directory_focuses_the_directory_just_left() {
        let nanos =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("rc-leave-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(root.join("aaa")).unwrap();
        std::fs::create_dir_all(root.join("zzz")).unwrap();

        let backend = crate::vfs::registry::Registry::default().local();
        let mut panel = Panel::new(backend.clone(), VfsPath::local(root.join("zzz")));
        panel.reload().await.unwrap();
        panel.cursor = panel.entries.iter().position(|e| e.name == "..").unwrap();

        let (target, focus) = panel.target_dir_under_cursor().unwrap();
        assert!(panel.try_enter(target, backend, focus.as_deref()).await);
        assert_eq!(panel.current_entry().map(|e| e.name.as_str()), Some("zzz"));

        std::fs::remove_dir_all(&root).ok();
    }

    /// Enter on a symlink to a directory descends into it through the link's
    /// own path, so ".." comes back to the directory holding the link.
    #[cfg(unix)]
    #[tokio::test]
    async fn entering_a_symlinked_directory_follows_the_link() {
        let nanos =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("rc-symdir-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(root.join("real")).unwrap();
        std::fs::write(root.join("real").join("inside"), b"").unwrap();
        std::fs::write(root.join("afile"), b"").unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();

        let backend = crate::vfs::registry::Registry::default().local();
        let mut panel = Panel::new(backend.clone(), VfsPath::local(&root));
        panel.reload().await.unwrap();
        let link = panel.entries.iter().position(|e| e.name == "link").unwrap();
        let file = panel.entries.iter().position(|e| e.name == "afile").unwrap();
        assert!(link < file, "the directory link sorts with the directories");
        panel.cursor = link;

        let (target, focus) = panel.target_dir_under_cursor().unwrap();
        assert_eq!(target, VfsPath::local(root.join("link")));
        assert!(panel.try_enter(target, backend, focus.as_deref()).await);
        assert!(panel.entries.iter().any(|e| e.name == "inside"));

        std::fs::remove_dir_all(&root).ok();
    }
}
