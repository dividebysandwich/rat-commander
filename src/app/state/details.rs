//! Drives the "Details" panel view: figures out what the *other* panel points
//! at and, for a directory or multi-item selection, walks it in the background
//! (so remote/large trees stay responsive) to tally its recursive size.

use super::*;
use crate::details::{DetailsData, DetailsKind, FileInfo, Tally};
use std::sync::Arc;

/// What a Details panel should display, derived from the source panel.
enum Plan {
    Empty,
    File(VfsEntry),
    /// One or more roots to recursively size; `(name, kind, size)`.
    Tally { label: String, roots: Vec<(String, VfsKind, u64)> },
}

impl AppState {
    /// Refresh both panels' Details state. Cheap when nothing changed; called
    /// once per loop iteration (after every key/mouse/tick/event). Restarts the
    /// background size scan when the source panel's cursor/selection changes.
    pub fn update_details(&mut self) {
        for viewer in 0..2 {
            if self.panels[viewer].format != ViewFormat::Details {
                // Left Details mode: cancel any scan and forget the state.
                if self.details[viewer].cancel.is_some() || !self.details[viewer].key.is_empty() {
                    if let Some(c) = self.details[viewer].cancel.take() {
                        c.cancel();
                    }
                    self.details[viewer] = DetailsData::default();
                }
                continue;
            }
            let source = 1 - viewer;
            let key = self.details_key(source);
            if key != self.details[viewer].key {
                self.details[viewer].key = key;
                self.start_details(viewer);
            }
        }
    }

    /// A signature of what the source panel currently points at, so a change
    /// (navigation, cursor move, or selection edit) triggers a recompute.
    fn details_key(&self, source: usize) -> String {
        let p = &self.panels[source];
        if p.format == ViewFormat::Details {
            return "\u{0}details".to_string(); // source isn't a normal listing
        }
        let cursor = p.current_entry().map(|e| e.name.as_str()).unwrap_or("");
        format!("{}\u{0}{}\u{0}{}", p.cwd.display(), cursor, p.selection.signature())
    }

    /// (Re)build `details[viewer]` from the source panel, starting a background
    /// size scan when a directory or selection needs one.
    fn start_details(&mut self, viewer: usize) {
        let source = 1 - viewer;
        if let Some(c) = self.details[viewer].cancel.take() {
            c.cancel();
        }
        self.details[viewer].generation = self.details[viewer].generation.wrapping_add(1);
        let generation = self.details[viewer].generation;

        // Decide what to show (immutable borrow of the source panel only).
        let (cwd, backend, plan) = {
            let p = &self.panels[source];
            let plan = if p.format == ViewFormat::Details {
                Plan::Empty
            } else if !p.selection.is_empty() {
                let roots: Vec<(String, VfsKind, u64)> = p
                    .entries
                    .iter()
                    .filter(|e| e.name != ".." && p.selection.is_marked(&e.name))
                    .map(|e| (e.name.clone(), e.kind, e.size))
                    .collect();
                let label = format!(
                    "{} item{} selected",
                    roots.len(),
                    if roots.len() == 1 { "" } else { "s" }
                );
                Plan::Tally { label, roots }
            } else if let Some(e) = p.current_entry().filter(|e| e.name != "..") {
                if e.kind == VfsKind::Dir {
                    Plan::Tally {
                        label: format!("{}/", e.name),
                        roots: vec![(e.name.clone(), e.kind, e.size)],
                    }
                } else {
                    Plan::File(e.clone())
                }
            } else {
                Plan::Empty
            };
            (p.cwd.clone(), p.backend.clone(), plan)
        };

        match plan {
            Plan::Empty => self.details[viewer].kind = DetailsKind::Empty,
            Plan::File(e) => {
                let fi = self.file_info(&cwd, &e);
                self.details[viewer].kind = DetailsKind::File(fi);
            }
            Plan::Tally { label, roots } => {
                // Seed the immediate counts (files are sized straight away); only
                // real directories need the recursive background walk.
                let (mut total, mut files, mut dirs, mut has_dirs) = (0u64, 0u64, 0u64, false);
                for (_, kind, size) in &roots {
                    if *kind == VfsKind::Dir {
                        dirs += 1;
                        has_dirs = true;
                    } else {
                        files += 1;
                        total += size;
                    }
                }
                self.details[viewer].kind =
                    DetailsKind::Tally(Tally { label, total, files, dirs, scanning: has_dirs });
                if has_dirs {
                    let cancel = CancelToken::new();
                    self.details[viewer].cancel = Some(cancel.clone());
                    let roots: Vec<(VfsPath, VfsKind, u64)> =
                        roots.into_iter().map(|(n, k, s)| (cwd.join(&n), k, s)).collect();
                    let tx = self.tx.clone();
                    tokio::spawn(async move {
                        scan_tally(backend, roots, viewer, generation, cancel, tx).await;
                    });
                }
            }
        }
    }

    /// Build the render-ready file overview, resolving owner/group names here
    /// (where the lookups live) so the renderer just formats strings.
    fn file_info(&self, cwd: &VfsPath, e: &VfsEntry) -> FileInfo {
        let owner = e
            .uid
            .map(|u| uid_name(u).unwrap_or_else(|| u.to_string()))
            .unwrap_or_else(|| "—".to_string());
        let group = e
            .gid
            .map(|g| gid_name(g).unwrap_or_else(|| g.to_string()))
            .unwrap_or_else(|| "—".to_string());
        FileInfo {
            name: e.name.clone(),
            dir: cwd.display(),
            kind: e.kind,
            size: e.size,
            mode: e.mode,
            owner,
            group,
            mtime: e.mtime,
            atime: e.atime,
            ctime: e.ctime,
            inode: e.inode,
            symlink_target: e.symlink_target.clone(),
        }
    }

    /// Apply a tally update from a background scan (ignored if stale).
    pub(in crate::app::state) fn apply_details_tally(
        &mut self,
        viewer: usize,
        generation: u64,
        total: u64,
        files: u64,
        dirs: u64,
        done: bool,
    ) {
        let Some(d) = self.details.get_mut(viewer) else {
            return;
        };
        if d.generation != generation {
            return;
        }
        if let DetailsKind::Tally(t) = &mut d.kind {
            t.total = total;
            t.files = files;
            t.dirs = dirs;
            t.scanning = !done;
        }
    }
}

/// Recursively walk `roots` (never following symlinks), accumulating the total
/// file size plus file/dir counts, sending throttled [`AppEvent::DetailsTally`]
/// updates until done or cancelled.
async fn scan_tally(
    backend: Arc<dyn Vfs>,
    roots: Vec<(VfsPath, VfsKind, u64)>,
    viewer: usize,
    generation: u64,
    cancel: CancelToken,
    tx: AppSender,
) {
    let (mut total, mut files, mut dirs) = (0u64, 0u64, 0u64);
    let mut stack: Vec<VfsPath> = Vec::new();
    for (path, kind, size) in roots {
        if kind == VfsKind::Dir {
            dirs += 1;
            stack.push(path);
        } else {
            files += 1;
            total += size;
        }
    }

    let mut last = std::time::Instant::now();
    while let Some(dir) = stack.pop() {
        if cancel.is_cancelled() {
            return; // a newer scan superseded this one; drop it silently
        }
        let entries = backend.read_dir(&dir).await.unwrap_or_default();
        for e in entries {
            if e.name == ".." || e.name == "." {
                continue;
            }
            if e.kind == VfsKind::Dir && e.symlink_target.is_none() {
                dirs += 1;
                stack.push(dir.join(&e.name));
            } else {
                files += 1;
                total += e.size;
            }
        }
        // Throttle to ~12 updates/sec so a deep local tree can't flood the loop.
        if last.elapsed() >= std::time::Duration::from_millis(80) {
            let _ = tx.try_send(AppEvent::DetailsTally {
                viewer,
                generation,
                total,
                files,
                dirs,
                done: false,
            });
            last = std::time::Instant::now();
        }
    }
    let _ = tx
        .send(AppEvent::DetailsTally { viewer, generation, total, files, dirs, done: true })
        .await;
}
