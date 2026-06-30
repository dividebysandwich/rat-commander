//! "Find duplicates": mark files identical between the two panel directories.
//!
//! The comparison runs as a cancellable background task — content reads and
//! remote backends can be slow — reporting progress and delivering the names to
//! mark via [`AppEvent::DuplicatesFound`].

use super::*;
use std::sync::Arc;
use std::time::SystemTime;

/// A file's identity attributes, snapshotted from the panel listing.
struct FileMeta {
    name: String,
    size: u64,
    mtime: Option<SystemTime>,
}

impl AppState {
    /// Start a background scan that marks files present in both panels and
    /// identical per `crit`, behind a cancellable progress dialog.
    pub(in crate::app::state) fn start_find_duplicates(&mut self, crit: DupCriteria) {
        if self.panels[0].is_panelized() || self.panels[1].is_panelized() {
            return self.show_error("Cannot compare search-result panels");
        }
        let collect = |p: &Panel| -> Vec<FileMeta> {
            p.entries
                .iter()
                .filter(|e| e.kind == VfsKind::File && e.name != "..")
                .map(|e| FileMeta { name: e.name.clone(), size: e.size, mtime: e.mtime })
                .collect()
        };
        let left = collect(&self.panels[0]);
        let right = collect(&self.panels[1]);

        let ba = self.panels[0].backend.clone();
        let ca = self.panels[0].cwd.clone();
        let bb = self.panels[1].backend.clone();
        let cb = self.panels[1].cwd.clone();

        let id = self.next_task_id;
        self.next_task_id += 1;
        let cancel = CancelToken::new();
        // Find tasks never prompt for overwrite; the reply channel is unused.
        let (reply, _reply_rx) = tokio::sync::mpsc::channel(1);
        self.tasks.insert(id, TaskHandle { id, cancel: cancel.clone(), reply });
        self.dialog =
            Some(Dialog::Progress(ProgressDialog::scan(id, "Find duplicates", "duplicates")));

        let tx = self.tx.clone();
        tokio::spawn(async move {
            let (left_marks, right_marks) =
                find_duplicates(&ba, &ca, &left, &bb, &cb, &right, crit, &cancel, id, &tx).await;
            let _ = tx
                .send(AppEvent::DuplicatesFound { id, left: left_marks, right: right_marks })
                .await;
        });
    }

    /// Mark the duplicate files reported by the background task in both panels.
    pub(in crate::app::state) fn mark_duplicates(&mut self, left: Vec<String>, right: Vec<String>) {
        self.panels[0].selection.clear();
        self.panels[1].selection.clear();
        for n in &left {
            self.panels[0].selection.mark(n);
        }
        for n in &right {
            self.panels[1].selection.mark(n);
        }
        if left.is_empty() && right.is_empty() {
            self.show_info("Find duplicates", "No duplicate files found.");
        }
    }
}

/// Compare the two file lists, returning the names to mark in `(left, right)`.
/// Reports progress and stops early when `cancel` fires (partial results kept).
#[allow(clippy::too_many_arguments)]
async fn find_duplicates(
    ba: &Arc<dyn Vfs>,
    ca: &VfsPath,
    left: &[FileMeta],
    bb: &Arc<dyn Vfs>,
    cb: &VfsPath,
    right: &[FileMeta],
    crit: DupCriteria,
    cancel: &CancelToken,
    id: TaskId,
    tx: &AppSender,
) -> (Vec<String>, Vec<String>) {
    let key = |name: &str| {
        if crit.case_sensitive {
            name.to_string()
        } else {
            name.to_lowercase()
        }
    };
    // Index the right panel's files by their match key (name, or lower-cased
    // name for a case-insensitive comparison).
    let mut right_by: HashMap<String, Vec<&FileMeta>> = HashMap::new();
    for r in right {
        right_by.entry(key(&r.name)).or_default().push(r);
    }

    let mut left_marks: HashSet<String> = HashSet::new();
    let mut right_marks: HashSet<String> = HashSet::new();
    for l in left {
        if cancel.is_cancelled() {
            break;
        }
        let Some(candidates) = right_by.get(&key(&l.name)) else {
            continue; // no same-named file on the other side
        };
        // Surface the file being checked before the (possibly slow) comparison.
        let _ = tx.try_send(AppEvent::Progress(ProgressUpdate {
            id,
            verb: "Find duplicates",
            current_name: l.name.clone(),
            file_done: 0,
            file_total: 0,
            total_done: 0,
            total_total: 0,
            files_done: left_marks.len() as u64,
            files_total: 0,
        }));
        let mut matched = false;
        for &r in candidates {
            if matches_criteria(l, r, crit, ba, ca, bb, cb).await {
                right_marks.insert(r.name.clone());
                matched = true;
            }
        }
        if matched {
            left_marks.insert(l.name.clone());
        }
    }

    (left_marks.into_iter().collect(), right_marks.into_iter().collect())
}

/// Whether two same-named files satisfy every enabled criterion. With none of
/// size/date/content enabled this is always true (name match only).
async fn matches_criteria(
    l: &FileMeta,
    r: &FileMeta,
    crit: DupCriteria,
    ba: &Arc<dyn Vfs>,
    ca: &VfsPath,
    bb: &Arc<dyn Vfs>,
    cb: &VfsPath,
) -> bool {
    if crit.size && l.size != r.size {
        return false;
    }
    if crit.date && !matches!((l.mtime, r.mtime), (Some(a), Some(b)) if a == b) {
        return false;
    }
    if crit.content {
        // Different sizes ⇒ different content; no need to read the bytes.
        if l.size != r.size {
            return false;
        }
        if files_differ(ba, &ca.join(&l.name), bb, &cb.join(&r.name)).await {
            return false;
        }
    }
    true
}
