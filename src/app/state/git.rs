//! Git/VCS-aware panels: kicks off a background `git status` scan when a panel's
//! (local) directory changes, applies the result, and runs the stage/unstage and
//! "diff against HEAD" actions.

use super::*;

impl AppState {
    /// Refresh both panels' Git status. Called once per loop iteration (like
    /// [`AppState::update_details`]): starts a background scan when a panel's local
    /// directory changes, and clears the status on a non-local panel.
    pub fn update_git(&mut self) {
        for side in 0..2 {
            let key = if self.panels[side].cwd.scheme == "file" {
                self.panels[side].cwd.path.to_string_lossy().into_owned()
            } else {
                String::new()
            };
            if key == self.git_key[side] {
                continue;
            }
            self.git_key[side] = key.clone();
            if key.is_empty() {
                // Remote/archive panel: no VCS info.
                self.panels[side].git = None;
                self.git_gen[side] = self.git_gen[side].wrapping_add(1);
            } else {
                self.start_git_scan(side);
            }
        }
    }

    /// Force a re-scan of both panels' Git status on the next loop iteration —
    /// called after operations that may have changed the working tree.
    pub(in crate::app::state) fn invalidate_git(&mut self) {
        self.git_key = [String::new(), String::new()];
    }

    /// Spawn a background `git status` scan for panel `side`, guarded by a
    /// generation counter so a stale result is dropped.
    fn start_git_scan(&mut self, side: usize) {
        self.git_gen[side] = self.git_gen[side].wrapping_add(1);
        let generation = self.git_gen[side];
        let dir = self.panels[side].cwd.path.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let status = crate::git::status(&dir).await.map(Box::new);
            let _ = tx.send(AppEvent::GitStatusScanned { side, generation, status }).await;
        });
    }

    /// Apply a completed scan (ignored if a newer scan has since started).
    pub(in crate::app::state) fn apply_git_status(
        &mut self,
        side: usize,
        generation: u64,
        status: Option<crate::git::GitStatus>,
    ) {
        if self.git_gen[side] != generation {
            return;
        }
        self.panels[side].git = status;
    }

    /// Ctrl-G: stage or unstage the entry (or selection) under the cursor in the
    /// active panel. Staged entries are unstaged; everything else is staged.
    pub(in crate::app::state) async fn git_stage_toggle(&mut self) {
        let side = self.active;
        let Some(git) = self.panels[side].git.as_ref() else {
            return self.show_error("Not a git repository");
        };
        // Decide direction from the entry under the cursor: if it is staged,
        // unstage the whole set; otherwise stage it.
        let cursor_staged = self
            .panels[side]
            .current_entry()
            .and_then(|e| git.state_of(&e.name))
            .is_some_and(|s| s.is_staged());

        let names = self.git_action_targets(side);
        if names.is_empty() {
            return self.show_error("No file under the cursor");
        }
        let dir = self.panels[side].cwd.path.clone();
        let mut err = None;
        for name in &names {
            let res = if cursor_staged {
                crate::git::unstage(&dir, name).await
            } else {
                crate::git::stage(&dir, name).await
            };
            if let Err(e) = res {
                err = Some(e);
                break;
            }
        }
        match err {
            Some(e) => self.show_error(format!("git: {e}")),
            None => self.start_git_scan(side), // refresh the glyphs
        }
    }

    /// Alt-G: open the side-by-side diff of the file under the cursor against its
    /// committed (`HEAD`) version, reusing the file-comparison view.
    pub(in crate::app::state) async fn open_git_diff(&mut self) {
        let side = self.active;
        let Some(git) = self.panels[side].git.as_ref() else {
            return self.show_error("Not a git repository");
        };
        let root = git.root.clone();
        let Some(entry) = self
            .panels[side]
            .current_entry()
            .filter(|e| e.kind == VfsKind::File && e.name != "..")
            .cloned()
        else {
            return self.show_error("Put the cursor on a file to diff against HEAD");
        };
        let name = entry.name.clone();
        let work_path = self.panels[side].cwd.join(&name);
        // Path of the file relative to the repo root (for `git show HEAD:<rel>`).
        let rel = self
            .panels[side]
            .cwd
            .path
            .join(&name)
            .strip_prefix(&root)
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|_| std::path::PathBuf::from(&name));

        let backend = self.panels[side].backend.clone();
        let work_data = match load_file(&backend, &work_path).await {
            Ok(d) => d,
            Err(e) => return self.show_error(format!("cannot read {name}: {e}")),
        };
        // The committed version (empty for a new/untracked file).
        let head_data = crate::git::head_blob(&root, &rel).await.unwrap_or_default();

        // Left = HEAD (read-only: a non-`file` scheme so an accidental save can't
        // overwrite anything); right = the working file.
        let head_path = VfsPath { scheme: "git-head".into(), path: rel.clone(), container: None };
        self.diffview = Some(DiffView::new(
            format!("{name}  (HEAD)"),
            head_path,
            &head_data,
            name,
            work_path,
            &work_data,
        ));
    }

    /// The entry names the git actions should act on: the marked set if any,
    /// otherwise the file under the cursor (never `..`).
    fn git_action_targets(&self, side: usize) -> Vec<String> {
        let p = &self.panels[side];
        if !p.selection.is_empty() {
            p.selection
                .marked_names(&p.entries)
                .into_iter()
                .map(|n| n.to_string())
                .filter(|n| n != "..")
                .collect()
        } else {
            p.current_entry()
                .filter(|e| e.name != "..")
                .map(|e| vec![e.name.clone()])
                .unwrap_or_default()
        }
    }
}
