//! Directory-tree model for the Tree view.
//!
//! A [`TreeState`] holds a *flattened* list of visible directory nodes (the
//! filesystem tree with some branches expanded). Expanding a node splices its
//! subdirectories in right after it; collapsing removes the whole subtree. The
//! tree is rooted at the backend/drive root of the panel's directory (`/` on
//! Unix, the current drive's root on Windows) and, when built, is opened all the
//! way down to that directory with the cursor resting on it.

use crate::vfs::{Vfs, VfsKind, VfsPath};
use std::sync::Arc;

/// One visible row of the tree — always a directory.
pub struct TreeNode {
    /// The directory this row points at.
    pub path: VfsPath,
    /// Display label: the final path component (the root shows its full path).
    pub label: String,
    /// Indentation level (root = 0).
    pub depth: usize,
    /// Whether this node's children are currently spliced into `rows` below it.
    pub expanded: bool,
}

impl TreeNode {
    fn new(path: VfsPath, depth: usize) -> Self {
        let label = path.file_name();
        TreeNode { path, label, depth, expanded: false }
    }
}

/// The flattened, navigable directory tree shown in a Tree-view panel.
pub struct TreeState {
    /// Visible rows, in display (pre-order) order.
    pub rows: Vec<TreeNode>,
    /// Index of the highlighted row.
    pub cursor: usize,
    /// First visible row (scroll offset), maintained by the renderer.
    pub offset: usize,
    /// The directory last *committed* by pressing Enter (which also points the
    /// other panel here). Drives the command-line prompt, so it changes only on
    /// Enter — not as the cursor merely browses. Starts at the panel's directory.
    pub current: VfsPath,
}

impl TreeState {
    /// Build the tree for `cwd` on `backend`: rooted at the backend/drive root,
    /// expanded down to `cwd`, cursor on `cwd` (which is itself expanded so its
    /// subdirectories are visible).
    pub async fn build(backend: &Arc<dyn Vfs>, cwd: &VfsPath) -> TreeState {
        let chain = ancestor_chain(cwd); // [root, …, cwd]
        let mut rows = vec![TreeNode::new(chain[0].clone(), 0)];
        // Walk down the chain, expanding each level so the path to `cwd` is open.
        let mut idx = 0;
        for next in &chain[1..] {
            expand_at(backend, &mut rows, idx).await;
            match rows[idx + 1..].iter().position(|n| n.path == *next) {
                Some(rel) => idx += 1 + rel,
                None => break, // level unreadable or entry gone: stop descending
            }
        }
        // Reveal the target directory's own subdirectories.
        expand_at(backend, &mut rows, idx).await;
        // Start scrolled so the target sits at the top of the viewport — its
        // just-revealed children are then immediately visible below it, instead
        // of the cursor landing at the bottom amid its siblings (e.g. under a
        // huge `/tmp`). The renderer only scrolls further if the cursor moves.
        TreeState { rows, cursor: idx, offset: idx, current: cwd.clone() }
    }

    /// The directory under the cursor, if any.
    pub fn selected_path(&self) -> Option<VfsPath> {
        self.rows.get(self.cursor).map(|n| n.path.clone())
    }

    pub fn move_cursor(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let max = self.rows.len() as isize - 1;
        self.cursor = (self.cursor as isize + delta).clamp(0, max) as usize;
    }

    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    pub fn move_end(&mut self) {
        if !self.rows.is_empty() {
            self.cursor = self.rows.len() - 1;
        }
    }

    /// Toggle the cursor row: expand a collapsed node (loading its children on
    /// `backend`) or collapse an expanded one. This is the "commit" action
    /// (Enter): the cursor's directory becomes [`current`](Self::current).
    pub async fn toggle(&mut self, backend: &Arc<dyn Vfs>) {
        let idx = self.cursor;
        if idx >= self.rows.len() {
            return;
        }
        // The toggled node keeps its position, so record it as the committed dir.
        self.current = self.rows[idx].path.clone();
        if self.rows[idx].expanded {
            collapse_at(&mut self.rows, idx);
        } else {
            expand_at(backend, &mut self.rows, idx).await;
        }
    }
}

/// The chain of directories from the backend/drive root down to `cwd`
/// (inclusive, root first), staying within the same backend/archive.
fn ancestor_chain(cwd: &VfsPath) -> Vec<VfsPath> {
    let mut chain = vec![cwd.clone()];
    let mut cur = cwd.clone();
    while let Some(parent) = cur.parent() {
        // Stop before crossing out of this backend (e.g. an archive root's
        // parent is the containing local directory).
        if parent.scheme != cwd.scheme || parent.container != cwd.container {
            break;
        }
        chain.push(parent.clone());
        cur = parent;
    }
    chain.reverse();
    chain
}

/// Read `path`'s immediate subdirectories, sorted case-insensitively. Errors
/// (e.g. permission denied) yield an empty list rather than failing the tree.
async fn read_subdirs(backend: &Arc<dyn Vfs>, path: &VfsPath) -> Vec<String> {
    let mut names: Vec<String> = match backend.read_dir(path).await {
        Ok(entries) => entries
            .into_iter()
            .filter(|e| e.kind == VfsKind::Dir)
            .map(|e| e.name)
            .collect(),
        Err(_) => Vec::new(),
    };
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// Expand `rows[idx]`: load its subdirectories and splice them in as new rows
/// directly below it. A no-op if already expanded.
async fn expand_at(backend: &Arc<dyn Vfs>, rows: &mut Vec<TreeNode>, idx: usize) {
    if rows[idx].expanded {
        return;
    }
    let base = rows[idx].path.clone();
    let depth = rows[idx].depth + 1;
    let children = read_subdirs(backend, &base).await;
    let nodes: Vec<TreeNode> =
        children.into_iter().map(|name| TreeNode::new(base.join(&name), depth)).collect();
    rows[idx].expanded = true;
    rows.splice(idx + 1..idx + 1, nodes);
}

/// Collapse `rows[idx]`: drop every deeper row that follows it (its subtree).
fn collapse_at(rows: &mut Vec<TreeNode>, idx: usize) {
    if !rows[idx].expanded {
        return;
    }
    let depth = rows[idx].depth;
    let mut end = idx + 1;
    while end < rows.len() && rows[end].depth > depth {
        end += 1;
    }
    rows.drain(idx + 1..end);
    rows[idx].expanded = false;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::registry::Registry;
    use std::fs;

    /// Build a small on-disk tree in a uniquely-named directory (so parallel
    /// tests don't clobber each other) and return its root path.
    fn scratch_tree(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("rc-tree-{tag}-{}-{nanos}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("alpha/one")).unwrap();
        fs::create_dir_all(base.join("alpha/two")).unwrap();
        fs::create_dir_all(base.join("beta")).unwrap();
        fs::write(base.join("alpha/file.txt"), b"x").unwrap();
        base
    }

    #[tokio::test]
    async fn build_opens_path_to_cwd_and_lists_its_subdirs() {
        let base = scratch_tree("build");
        let backend = Registry::default().local();
        let cwd = VfsPath::local(base.join("alpha"));
        let tree = TreeState::build(&backend, &cwd).await;

        // The cursor rests on the target directory…
        assert_eq!(tree.selected_path().unwrap().path, cwd.path);
        // …which is expanded, so its two subdirectories are visible below it.
        let labels: Vec<&str> = tree.rows.iter().map(|n| n.label.as_str()).collect();
        assert!(labels.contains(&"one"), "alpha/one should be listed: {labels:?}");
        assert!(labels.contains(&"two"), "alpha/two should be listed: {labels:?}");
        // Files are never shown in the tree.
        assert!(!labels.contains(&"file.txt"), "files are excluded: {labels:?}");

        let _ = fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn toggle_collapses_and_reexpands_a_branch() {
        let base = scratch_tree("toggle");
        let backend = Registry::default().local();
        let cwd = VfsPath::local(base.join("alpha"));
        let mut tree = TreeState::build(&backend, &cwd).await;

        let with_children = tree.rows.len();
        // Cursor is on `alpha` (expanded). Toggling collapses its subtree.
        tree.toggle(&backend).await;
        assert!(tree.rows.len() < with_children, "collapsing removes child rows");
        assert!(!tree.rows[tree.cursor].expanded);
        // Toggling again re-expands and restores the children.
        tree.toggle(&backend).await;
        assert_eq!(tree.rows.len(), with_children, "re-expanding restores the subtree");
        assert!(tree.rows[tree.cursor].expanded);

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn ancestor_chain_runs_root_first_to_cwd() {
        let chain = ancestor_chain(&VfsPath::local("/home/user/proj"));
        let tail: Vec<_> = chain.iter().map(|p| p.path.to_string_lossy().into_owned()).collect();
        assert_eq!(tail.first().unwrap(), "/", "root comes first");
        assert_eq!(tail.last().unwrap(), "/home/user/proj", "cwd comes last");
    }
}
