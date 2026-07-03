//! Find-file and editor search/replace.

use super::*;

impl AppState {
    pub(in crate::app::state) fn open_find_dialog(&mut self) {
        // Prefill the backend-relative path (no "scheme://"): for a remote panel
        // it's the remote start directory, interpreted on that backend.
        let start = self.panels[self.active].cwd.path.to_string_lossy().into_owned();
        self.dialog = Some(Dialog::Find(FindDialog::new(start)));
    }

    /// Build the editor's search/replace dialog, prefilled (and marked) with the
    /// last-used search and replacement terms — in Hex mode when the editor is.
    pub(in crate::app::state) fn editor_search_dialog(&self, replace: bool) -> SearchReplaceDialog {
        match self.editor.as_ref() {
            Some(ed) if ed.is_hex() => {
                SearchReplaceDialog::new_hex(replace, ed.last_hex_search(), ed.last_replacement())
            }
            Some(ed) => {
                SearchReplaceDialog::new(replace, ed.last_search_pattern(), ed.last_replacement())
            }
            None => SearchReplaceDialog::new(replace, String::new(), String::new()),
        }
    }

    pub(in crate::app::state) fn apply_search_replace(&mut self, p: SearchReplaceParams) {
        if let Some(ed) = self.editor.as_mut() {
            if ed.is_hex() {
                ed.apply_hex_search_replace(p.replace, &p.search, &p.replacement, p.hex, p.backwards);
                return;
            }
            ed.apply_search_replace(
                p.replace,
                &p.search,
                &p.replacement,
                p.regex,
                p.case_sensitive,
                p.whole_words,
                p.backwards,
            );
        }
    }

    /// Launch a cancellable find-file search; a progress dialog shows the
    /// current path and lets the user abort. Results arrive via `FindDone`.
    pub(in crate::app::state) fn start_find(&mut self, p: FindParams) {
        let matcher =
            match crate::panel::selection::NameMatcher::build(&p.file_name, p.case_sensitive, p.shell) {
                Ok(m) => m,
                Err(e) => return self.show_error(format!("invalid pattern: {e}")),
            };
        let cwd = self.panels[self.active].cwd.clone();
        let backend = self.panels[self.active].backend.clone();
        // Non-local backends (remote, archives) are searched by name only via the
        // VFS — content search isn't reasonable over the network.
        let on_vfs = cwd.scheme != "file";

        let id = self.next_task_id;
        self.next_task_id += 1;
        let cancel = CancelToken::new();
        // Find tasks never prompt for overwrite; an unused reply channel keeps
        // the handle shape uniform.
        let (reply, _reply_rx) = tokio::sync::mpsc::channel(1);
        self.tasks.insert(
            id,
            TaskHandle {
                id,
                cancel: cancel.clone(),
                reply,
            },
        );
        self.dialog = Some(Dialog::Progress(ProgressDialog::find(id)));

        let progress = move |tx2: AppSender, cur: String, found: usize| {
            let _ = tx2.try_send(AppEvent::Progress(ProgressUpdate {
                id,
                verb: "Searching",
                current_name: cur,
                file_done: 0,
                file_total: 0,
                total_done: 0,
                total_total: 0,
                files_done: found as u64,
                files_total: 0,
            }));
        };

        let tx = self.tx.clone();
        if on_vfs {
            // Remote / archive: walk the backend by name only.
            let start = if p.start_at.trim().is_empty() {
                cwd.clone()
            } else {
                VfsPath {
                    scheme: cwd.scheme.clone(),
                    path: PathBuf::from(p.start_at.trim()),
                    container: cwd.container.clone(),
                }
            };
            let (recursive, skip_hidden) = (p.recursive, p.skip_hidden);
            tokio::spawn(async move {
                let tx2 = tx.clone();
                let results = find_files_vfs(
                    &backend,
                    start,
                    &matcher,
                    recursive,
                    skip_hidden,
                    &cancel,
                    |cur, found| progress(tx2.clone(), cur, found),
                )
                .await;
                let _ = tx.send(AppEvent::FindDone { id, results }).await;
            });
        } else {
            // Local: the existing blocking walker (supports content search).
            let start = if p.start_at.trim().is_empty() {
                cwd.path.clone()
            } else {
                PathBuf::from(&p.start_at)
            };
            tokio::spawn(async move {
                let tx2 = tx.clone();
                let results = tokio::task::spawn_blocking(move || {
                    find_files(&start, &p, &matcher, &cancel, |cur, found| {
                        progress(tx2.clone(), cur, found)
                    })
                    .into_iter()
                    .map(|path| {
                        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                        (VfsPath::local(path), size)
                    })
                    .collect::<Vec<_>>()
                })
                .await
                .unwrap_or_default();
                let _ = tx.send(AppEvent::FindDone { id, results }).await;
            });
        }
    }

    /// Panelize find-file results (with a `..` entry that returns to browsing).
    /// Results may be local or remote; the panel keeps the backend the matches
    /// live on so navigating into a result — or back out via `..` — works.
    pub(in crate::app::state) fn panelize_results(&mut self, results: Vec<(VfsPath, u64)>) {
        if results.is_empty() {
            return self.show_error("No files found");
        }
        let cwd = self.panels[self.active].cwd.clone();
        let mut entries = vec![VfsEntry {
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
        }];
        let mut vpaths = vec![cwd]; // dummy path paired with ".."
        for (path, size) in results {
            entries.push(VfsEntry {
                name: path.path.to_string_lossy().into_owned(),
                kind: VfsKind::File,
                size,
                mtime: None,
                atime: None,
                ctime: None,
                inode: None,
                mode: None,
                uid: None,
                gid: None,
                symlink_target: None,
                symlink_broken: false,
            });
            vpaths.push(path);
        }
        // Resolve the backend the results live on (local or the remote session).
        let backend = vpaths
            .get(1)
            .and_then(|p| self.registry.resolve(p).ok())
            .unwrap_or_else(|| self.registry.local());
        let p = &mut self.panels[self.active];
        p.backend = backend;
        p.set_results(entries, vpaths);
    }

}
