//! File operations: transfers, deletes, multi-rename, and archive ops.

use super::*;

impl AppState {
    pub(in crate::app::state) async fn begin_transfer(&mut self, kind: OpKind, sources: Vec<VfsPath>, dest: &str) {
        // The destination defaults to the *other* panel's backend, but a typed
        // `scheme://` prefix (or its absence on a remote panel) can redirect it
        // to any registered backend — letting a local path override a remote one.
        let other = self.other_index();
        let active = self.active;
        let target = dest_vfspath(dest, &self.panels[other].cwd, &self.panels[active].cwd);
        let dst_fs = match self.registry.resolve(&target) {
            Ok(b) => b,
            Err(e) => {
                self.show_error(format!("cannot resolve destination: {e}"));
                return;
            }
        };

        // mc-style disambiguation: a single source whose typed target is neither an
        // existing directory nor slash-terminated is a rename/move-to-name — the
        // last component is the new name and `target`'s parent is the container.
        // Otherwise the target *is* the directory the sources drop into.
        let ends_with_sep = dest.ends_with('/') || dest.ends_with(std::path::MAIN_SEPARATOR);
        let target_is_dir = dst_fs.stat(&target).await.map(|e| e.kind.is_dir()).unwrap_or(false);
        let rename = sources.len() == 1 && !ends_with_sep && !target_is_dir;
        let (dst_dir, dst_name) = match (rename, target.parent()) {
            (true, Some(parent)) => (parent, Some(target.file_name())),
            _ => (target, None),
        };

        // Only the local backend needs (and supports) creating the directory up
        // front; remote/other backends copy into an existing directory.
        if dst_dir.scheme == "file"
            && let Err(e) = tokio::fs::create_dir_all(dst_dir.as_path()).await
        {
            self.show_error(format!("cannot create destination: {e}"));
            return;
        }

        // Land the cursor on the moved/renamed item afterwards. Prefer the active
        // panel (an in-place rename lands there, and both panels may show the same
        // directory), falling back to whichever panel shows the destination.
        if sources.len() == 1 {
            let name = dst_name.clone().unwrap_or_else(|| sources[0].file_name());
            let idx = if self.panels[active].cwd == dst_dir {
                Some(active)
            } else {
                self.panels.iter().position(|p| p.cwd == dst_dir)
            };
            if let Some(idx) = idx {
                self.pending_focus = Some((idx, name));
            }
        }

        self.start_op(kind, sources, Some(dst_fs), Some(dst_dir), dst_name);
    }

    /// The name of the surviving entry the cursor should land on after a delete:
    /// the next entry at or below the cursor (so the cursor moves *down* onto the
    /// following file), falling back to the nearest entry above when the deleted
    /// items were last in the listing. `..` and entries being deleted are skipped.
    pub(in crate::app::state) fn delete_anchor(&self, targets: &[VfsPath]) -> Option<String> {
        let doomed: HashSet<String> = targets.iter().map(|t| t.file_name()).collect();
        let p = &self.panels[self.active];
        let surviving = |i: usize| {
            let name = &p.entries[i].name;
            (name != ".." && !doomed.contains(name)).then(|| name.clone())
        };
        (p.cursor..p.entries.len())
            .find_map(&surviving)
            .or_else(|| (0..p.cursor).rev().find_map(&surviving))
    }

    pub(in crate::app::state) fn start_op(
        &mut self,
        kind: OpKind,
        sources: Vec<VfsPath>,
        dst_fs: Option<std::sync::Arc<dyn crate::vfs::Vfs>>,
        dst_dir: Option<VfsPath>,
        dst_name: Option<String>,
    ) {
        if sources.is_empty() {
            return;
        }
        // For a delete, remember the surviving entry just above the deleted one
        // so the cursor lands there (not at the top) once the listing reloads.
        if kind == OpKind::Delete {
            let active = self.active;
            self.pending_focus = self.delete_anchor(&sources).map(|n| (active, n));
        }
        let id = self.next_task_id;
        self.next_task_id += 1;
        // The sources are the active panel's marked set (or its cursor entry), so
        // that panel's selection is the one to drop once the op finishes.
        self.op_source.insert(id, self.active);
        let verb = match kind {
            OpKind::Copy => "Copying",
            OpKind::Move => "Moving",
            OpKind::Delete => "Deleting",
            OpKind::Sync => "Synchronizing",
        };
        // Remote backend schemes this op touches, so a later "To background" can
        // reopen a browsing connection for FTP (which blocks while transferring).
        let mut schemes: Vec<String> = Vec::new();
        let src_scheme = self.panels[self.active].cwd.scheme.clone();
        if src_scheme != "file" {
            schemes.push(src_scheme);
        }
        if let Some(d) = &dst_dir
            && d.scheme != "file"
            && !schemes.contains(&d.scheme)
        {
            schemes.push(d.scheme.clone());
        }
        let req = OpRequest {
            kind,
            src_fs: self.panels[self.active].backend.clone(),
            sources,
            dst_fs,
            dst_dir,
            dst_name,
            overwrite_all: !self.config.confirm_overwrite,
            steps: Vec::new(), // only OpKind::Sync carries a plan
        };
        let handle = spawn_op(id, req, self.tx.clone());
        self.tasks.insert(id, handle);
        // Track it as a backgroundable transfer (drives the mini bar / list).
        self.task_progress.insert(id, BgTransfer { verb, update: None, schemes, chart: SpeedChart::default() });
        let mut pd = ProgressDialog::new(id, verb);
        pd.backgroundable = true;
        self.dialog = Some(Dialog::Progress(pd));
    }

    /// One list row per tracked transfer, ordered by id (stable across updates).
    fn background_rows(&self) -> Vec<BgRow> {
        let mut rows: Vec<BgRow> = self
            .task_progress
            .iter()
            .map(|(id, t)| {
                let (done, total, name) = t
                    .update
                    .as_ref()
                    .map(|u| (u.total_done, u.total_total, u.current_name.clone()))
                    .unwrap_or((0, 0, String::new()));
                let ratio = if total > 0 { done as f64 / total as f64 } else { 0.0 };
                let label = if name.is_empty() {
                    t.verb.to_string()
                } else {
                    format!("{}  {name}", t.verb)
                };
                BgRow { id: *id, label, ratio }
            })
            .collect();
        rows.sort_by_key(|r| r.id);
        rows
    }

    /// Open the "Background operations" list of running transfers.
    pub(in crate::app::state) fn open_background_ops(&mut self) {
        let rows = self.background_rows();
        if rows.is_empty() {
            self.show_info("Background operations", "No background operations are running.");
            return;
        }
        self.dialog = Some(Dialog::BackgroundOps(BackgroundOpsDialog::new(rows)));
    }

    /// Refresh the open "Background operations" list from the latest progress so
    /// its bars advance live. Closes the list once no transfers remain.
    pub(in crate::app::state) fn refresh_background_ops(&mut self) {
        if !matches!(self.dialog, Some(Dialog::BackgroundOps(_))) {
            return;
        }
        let rows = self.background_rows();
        if rows.is_empty() {
            self.dialog = None;
        } else if let Some(Dialog::BackgroundOps(d)) = &mut self.dialog {
            d.set_rows(rows);
        }
    }

    /// Rebuild a progress dialog for a (possibly backgrounded) transfer from its
    /// latest snapshot — used to foreground it (from the list, or when it hits an
    /// overwrite conflict).
    pub(in crate::app::state) fn progress_dialog_for(&self, id: TaskId) -> ProgressDialog {
        let verb = self.task_progress.get(&id).map(|t| t.verb).unwrap_or("Copying");
        let mut d = ProgressDialog::new(id, verb);
        d.backgroundable = true;
        if let Some(t) = self.task_progress.get(&id) {
            if let Some(u) = t.update.as_ref() {
                d.update(u);
            }
            // Adopt the history the task has been keeping — including whatever it
            // recorded while running in the background — so the chart picks up
            // where it left off rather than starting blank.
            d.chart.clone_from(&t.chart);
        }
        d
    }

    /// Open the multi-rename dialog for the currently *selected* files. Requires
    /// an explicit selection (unlike most ops, it does not fall back to the file
    /// under the cursor).
    pub(in crate::app::state) fn open_multi_rename(&mut self) {
        let p = &self.panels[self.active];
        if p.is_panelized() {
            return self.show_error("Multi rename is not available on search results");
        }
        if p.selection.is_empty() {
            return self.show_error("No files selected. Select files first (Insert, or + to select a group).");
        }
        let sources = p.operation_targets();
        if sources.is_empty() {
            return self.show_error("No files selected.");
        }
        let (date, time) = crate::rename::date_time_now();
        self.dialog = Some(Dialog::MultiRename(MultiRenameDialog::new(sources, date, time)));
    }

    /// Apply a batch rename. Renames are done in two phases (each source to a
    /// unique temporary name, then to its final name) so intra-batch
    /// permutations — swaps, rotations, counter renumberings — can't clobber a
    /// not-yet-renamed sibling. Existing files outside the batch are never
    /// overwritten.
    pub(in crate::app::state) async fn do_multi_rename(&mut self, plan: Vec<(VfsPath, String)>) {
        // Drop no-ops (unchanged names).
        let jobs: Vec<(VfsPath, String)> = plan
            .into_iter()
            .filter(|(src, name)| *name != src.file_name())
            .collect();
        if jobs.is_empty() {
            return;
        }

        // Validate target names.
        for (_, name) in &jobs {
            if name.is_empty() || name.contains('/') || name.contains('\\') {
                return self.show_error(format!("Invalid target name: \"{name}\""));
            }
        }
        // Reject duplicate target names within the batch.
        let mut seen = HashSet::new();
        for (_, name) in &jobs {
            if !seen.insert(name.as_str()) {
                return self.show_error(format!("Two files would be renamed to \"{name}\""));
            }
        }

        let dir = self.panels[self.active].cwd.clone();
        let backend = self.panels[self.active].backend.clone();
        let targets: Vec<VfsPath> = jobs.iter().map(|(_, name)| dir.join(name)).collect();
        let temps: Vec<VfsPath> = (0..jobs.len()).map(|i| dir.join(format!(".rc-rename-tmp-{i}"))).collect();

        // Refuse to overwrite an existing file that isn't itself being renamed
        // away (a final name that matches a source is safe — phase 2 handles it).
        let source_names: HashSet<String> = jobs.iter().map(|(s, _)| s.file_name()).collect();
        for (i, (_, name)) in jobs.iter().enumerate() {
            if !source_names.contains(name) && backend.stat(&targets[i]).await.is_ok() {
                return self.show_error(format!("\"{name}\" already exists"));
            }
        }
        // The temporary names must be free, or phase 1 could clobber them.
        for t in &temps {
            if backend.stat(t).await.is_ok() {
                return self.show_error("Temporary rename name is in use; please retry");
            }
        }

        // Phase 1: source → temp. Roll back everything staged on the first error.
        let mut staged = 0;
        let mut phase1_err = None;
        for (i, (src, _)) in jobs.iter().enumerate() {
            if let Err(e) = backend.rename(src, &temps[i]).await {
                phase1_err = Some(format!("Rename failed: {e}"));
                break;
            }
            staged = i + 1;
        }
        if let Some(e) = phase1_err {
            for i in 0..staged {
                let _ = backend.rename(&temps[i], &jobs[i].0).await;
            }
            let _ = self.panels[self.active].reload().await;
            return self.show_error(e);
        }

        // Phase 2: temp → final. Restore the original name if a final fails.
        let mut errors = Vec::new();
        for (i, (src, name)) in jobs.iter().enumerate() {
            if let Err(e) = backend.rename(&temps[i], &targets[i]).await {
                let _ = backend.rename(&temps[i], src).await;
                errors.push(format!("{name}: {e}"));
            }
        }

        self.panels[self.active].selection.clear();
        let _ = self.panels[self.active].reload().await;
        if !errors.is_empty() {
            let shown: Vec<String> = errors.iter().take(8).cloned().collect();
            let more = errors.len().saturating_sub(shown.len());
            let mut msg = format!("{} rename(s) failed:\n{}", errors.len(), shown.join("\n"));
            if more > 0 {
                msg.push_str(&format!("\n… and {more} more"));
            }
            self.show_error(msg);
        }
    }

    pub(in crate::app::state) fn open_compress(&mut self) {
        let p = &self.panels[self.active];
        if p.cwd.is_archive() {
            return self.show_error("Compress from a local directory");
        }
        let sources = p.operation_targets();
        if sources.is_empty() {
            return;
        }
        self.dialog = Some(Dialog::Input(InputDialog::new(
            "Compress",
            "Archive name (.zip .7z .tar.gz .tar.bz2 .tar.xz):",
            "archive.tar.gz",
            InputPurpose::Compress(sources),
        )));
    }

    pub(in crate::app::state) fn start_compress(&mut self, sources: Vec<VfsPath>, name: String) {
        let format = match ArchiveFormat::from_name(&name) {
            Some(ArchiveFormat::Rar) => return self.show_error("Cannot create RAR archives"),
            Some(f) => f,
            None => {
                return self
                    .show_error("Unknown type (use .zip .7z .tar.gz .tar.bz2 .tar.xz)");
            }
        };
        let dest = self.panels[self.active].cwd.path.join(&name);
        let local: Vec<PathBuf> = sources.iter().map(|s| s.path.clone()).collect();
        self.spawn_archive_op("Compressing", move || {
            archive::create_archive(format, &dest, &local)
        });
    }

    pub(in crate::app::state) fn start_archive_add(&mut self, kind: OpKind, sources: Vec<VfsPath>, dest: VfsPath) {
        let Some(container) = dest.container.clone() else {
            return self.show_error("destination is not an archive");
        };
        let dest_inner = dest.path.to_string_lossy().into_owned();
        let local: Vec<PathBuf> = sources.iter().map(|s| s.path.clone()).collect();
        let is_move = matches!(kind, OpKind::Move);
        self.spawn_archive_op("Updating archive", move || {
            archive::add_to_archive(&container, &dest_inner, &local)?;
            if is_move {
                for s in &local {
                    let _ = remove_local(s);
                }
            }
            Ok(())
        });
    }

    pub(in crate::app::state) fn start_archive_remove(&mut self, targets: Vec<VfsPath>) {
        let Some(container) = targets.first().and_then(|t| t.container.clone()) else {
            return;
        };
        let set: HashSet<String> = targets
            .iter()
            .map(|t| t.path.to_string_lossy().into_owned())
            .collect();
        self.spawn_archive_op("Updating archive", move || {
            archive::remove_from_archive(&container, &set)
        });
    }

    /// Spawn a blocking archive mutation; shows a progress dialog and reloads
    /// panels when it finishes (via the usual `TaskDone` path).
    fn spawn_archive_op<F>(&mut self, verb: &'static str, f: F)
    where
        F: FnOnce() -> crate::util::Result<()> + Send + 'static,
    {
        let id = self.next_task_id;
        self.next_task_id += 1;
        // Compress / archive-add / archive-remove all act on the active panel's
        // marked set, so its selection is the one to drop when the op finishes.
        self.op_source.insert(id, self.active);
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let outcome = match tokio::task::spawn_blocking(f).await {
                Ok(Ok(())) => TaskOutcome::Done,
                Ok(Err(e)) => TaskOutcome::Failed(e.to_string()),
                Err(e) => TaskOutcome::Failed(e.to_string()),
            };
            let _ = tx.send(AppEvent::TaskDone { id, outcome }).await;
        });
        self.dialog = Some(Dialog::Progress(ProgressDialog::new(id, verb)));
    }
}
