//! Directory synchronisation (Command → Synchronize directories): collect the
//! mode, plan the work in the background, show it for confirmation, then hand the
//! plan to the ops engine.
//!
//! The active panel is always the source and the other panel the destination, so
//! the direction follows the same rule as F5/F6 and is spelled out in both
//! dialogs. Planning walks both trees over the VFS, which can be slow on a remote
//! or an archive, so it runs as a task behind a spinner rather than blocking the
//! UI.

use super::*;
use crate::ops::sync::{self, SyncMode, SyncPlan};

impl AppState {
    /// Open the sync options dialog for the two panels.
    pub(in crate::app::state) fn open_sync(&mut self) {
        // A panelized (search-result) listing isn't a directory, so there is
        // nothing coherent to mirror.
        if self.panels[0].is_panelized() || self.panels[1].is_panelized() {
            return self.show_error("Cannot synchronize search-result panels");
        }
        let (src, dst) = (self.sync_label(self.active), self.sync_label(self.other_index()));
        if src == dst {
            return self.show_error("Both panels show the same directory");
        }
        // An archive can be read from but not written into file by file (its
        // mutations are whole-archive rebuilds), so it can never be a sync
        // destination. Say so now rather than failing on the first copy.
        if !self.panels[self.other_index()].backend.capabilities().writable {
            return self.show_error(
                "The destination panel is read-only; an archive cannot be synchronized into",
            );
        }
        self.dialog = Some(Dialog::Form(FormDialog::sync(&src, &dst)));
    }

    /// A short display label for a panel's directory (used in the dialogs).
    fn sync_label(&self, side: usize) -> String {
        let cwd = &self.panels[side].cwd;
        if cwd.scheme == "file" && cwd.container.is_none() {
            cwd.path.to_string_lossy().into_owned()
        } else {
            cwd.display()
        }
    }

    /// Walk both trees in the background and plan the sync; the result arrives as
    /// [`AppEvent::SyncPlanned`].
    pub(in crate::app::state) fn start_sync_plan(&mut self, mode: SyncMode) {
        let (a, b) = (self.active, self.other_index());
        // Two-way writes to both sides, so both must accept writes.
        if mode.two_way() && !self.panels[a].backend.capabilities().writable {
            return self
                .show_error("A two-way sync must write to both panels, and this one is read-only");
        }
        let (fs_a, root_a) = (self.panels[a].backend.clone(), self.panels[a].cwd.clone());
        let (fs_b, root_b) = (self.panels[b].backend.clone(), self.panels[b].cwd.clone());
        let roots = [self.sync_label(a), self.sync_label(b)];
        let tx = self.tx.clone();
        let handle = tokio::spawn(async move {
            let result = async {
                let ta = sync::walk(&fs_a, &root_a).await?;
                let tb = sync::walk(&fs_b, &root_b).await?;
                // "Newer wins" needs timestamps, and some backends (FTP, SCP)
                // report none. Refuse rather than quietly turning a two-way sync
                // into a one-way one.
                if mode.two_way() && (sync::lacks_times(&ta) || sync::lacks_times(&tb)) {
                    return Err(crate::util::Error::other(
                        "this connection reports no file times, so \"newer wins\" cannot be \
                         decided — use a one-way mode, which compares sizes",
                    ));
                }
                let steps = sync::plan(&ta, &tb, [&root_a, &root_b], mode);
                Ok::<_, crate::util::Error>(SyncPlan { steps, mode, roots })
            }
            .await
            .map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::SyncPlanned { result: result.map(Box::new) }).await;
        });
        self.busy_task = Some(handle);
        self.dialog = Some(Dialog::Busy(
            BusyDialog::new("Synchronize", "Comparing directories…".to_string()).cancellable(),
        ));
    }

    /// The plan is ready: show it. Nothing has been touched yet.
    pub(in crate::app::state) fn on_sync_planned(&mut self, result: Result<SyncPlan, String>) {
        self.busy_task = None; // the walk delivered its result
        match result {
            Ok(plan) => self.dialog = Some(Dialog::SyncPreview(SyncPreviewDialog::new(plan))),
            Err(e) => self.show_error(format!("Cannot compare the directories: {e}")),
        }
    }

    /// Execute a confirmed plan as an ordinary transfer task: one progress
    /// dialog, abortable, and sendable to the background like any copy.
    pub(in crate::app::state) fn start_sync(&mut self, plan: SyncPlan) {
        if plan.is_empty() {
            return;
        }
        let (a, b) = (self.active, self.other_index());
        let id = self.next_task_id;
        self.next_task_id += 1;
        // Remote schemes this touches, so "To background" can reopen a browsing
        // connection for FTP (which blocks while transferring).
        let mut schemes: Vec<String> = Vec::new();
        for side in [a, b] {
            let s = self.panels[side].cwd.scheme.clone();
            if s != "file" && !schemes.contains(&s) {
                schemes.push(s);
            }
        }
        let req = OpRequest {
            kind: OpKind::Sync,
            // Side 0 is the active panel, side 1 the other — the same indices the
            // plan's steps were built with.
            src_fs: self.panels[a].backend.clone(),
            sources: Vec::new(),
            dst_fs: Some(self.panels[b].backend.clone()),
            dst_dir: None,
            dst_name: None,
            // The plan already decided what wins; re-asking per file would defeat
            // the preview the user just approved.
            overwrite_all: true,
            steps: plan.steps,
        };
        let handle = spawn_op(id, req, self.tx.clone());
        self.tasks.insert(id, handle);
        self.task_progress
            .insert(id, BgTransfer { verb: "Synchronizing", update: None, schemes, chart: SpeedChart::default() });
        let mut dialog = ProgressDialog::new(id, "Synchronizing");
        dialog.backgroundable = true;
        self.dialog = Some(Dialog::Progress(dialog));
    }
}
