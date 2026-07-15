//! Git/VCS-aware panels: kicks off a background `git status` scan when a panel's
//! (local) directory changes, applies the result, and runs the stage/unstage and
//! "diff against HEAD" actions.

use super::*;
use crate::app::event::GitInfoForm;
use crate::git::ops;

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

    // -- The Git menu (File → Git, or Alt-G) --------------------------------

    /// Run every Git-menu action. Actions split three ways: those that need an
    /// existing repository, those that set one up (init/clone), and those that
    /// first read the repo's branches to populate a guided dialog.
    pub(in crate::app::state) async fn git_action(&mut self, action: MenuAction) {
        use MenuAction as M;
        // init/clone are the only ones that make sense outside a work tree; they
        // just need a local directory to act in.
        let Some(dir) = self.git_local_dir() else {
            return self.show_error("Git actions need a local directory");
        };
        match action {
            M::GitInit => {
                if self.panels[self.active].git.is_some() {
                    return self.show_error("This directory is already in a Git repository");
                }
                self.dialog =
                    Some(Dialog::Confirm(ConfirmDialog::git_init(&dir.to_string_lossy())));
                return;
            }
            M::GitClone => {
                self.dialog = Some(Dialog::Form(FormDialog::git_clone()));
                return;
            }
            _ => {}
        }
        // Everything below needs a work tree.
        if self.panels[self.active].git.is_none() {
            return self.show_error("Not a git repository");
        }
        match action {
            M::GitStatus => self.spawn_git("status", dir, ops::status_args()),
            M::GitLog => self.spawn_git("log", dir, ops::log_args()),
            M::GitDiff => self.open_git_diff().await,
            M::GitStage => self.git_stage_toggle().await,
            M::GitCommit => self.dialog = Some(Dialog::Form(FormDialog::git_commit())),
            M::GitPull => self.dialog = Some(Dialog::Form(FormDialog::git_pull())),
            M::GitSync => self.git_sync(dir),
            // These need the repo's branches/remotes before they can be shown.
            M::GitFetch => self.spawn_git_info(GitInfoForm::Fetch, dir),
            M::GitPush => self.spawn_git_info(GitInfoForm::Push, dir),
            M::GitCheckout => self.spawn_git_info(GitInfoForm::Checkout, dir),
            M::GitReset => self.dialog = Some(Dialog::Form(FormDialog::git_reset())),
            // File-scoped actions: act on the selection, or the cursor.
            M::GitAdd | M::GitUnstage | M::GitRemove | M::GitRestore => {
                let names = self.git_action_targets(self.active);
                if names.is_empty() {
                    return self.show_error("No file under the cursor");
                }
                match action {
                    M::GitAdd => self.spawn_git("add", dir, ops::add_args(&names)),
                    M::GitUnstage => self.spawn_git("restore", dir, ops::unstage_args(&names)),
                    // Both throw work away, so both ask first.
                    M::GitRemove => {
                        let args = ops::remove_args(&names, false);
                        self.dialog =
                            Some(Dialog::Confirm(ConfirmDialog::git_remove(&names, args)));
                    }
                    M::GitRestore => {
                        let args = ops::restore_args(&names, true);
                        self.dialog =
                            Some(Dialog::Confirm(ConfirmDialog::git_restore(&names, args)));
                    }
                    _ => unreachable!("outer match limits this set"),
                }
            }
            _ => {}
        }
    }

    /// Sync = pull then push, so the common "catch up and publish" round trip is
    /// one keystroke. Run as a single task; a failed pull skips the push (pushing
    /// on top of a failed merge would only add noise).
    fn git_sync(&mut self, dir: PathBuf) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let mut out = ops::run_text(&dir, &ops::pull_args(false)).await;
            if out.ok {
                let push = ops::run_text(&dir, &ops::push_args("", "", false, false, false)).await;
                if !push.text.trim().is_empty() {
                    if !out.text.trim().is_empty() {
                        out.text.push('\n');
                    }
                    out.text.push_str(&push.text);
                }
                out.ok = push.ok;
            }
            let _ = tx.send(AppEvent::GitDone { title: "sync".into(), out }).await;
        });
        self.busy_git("sync");
    }

    /// Run `git <args>` in `dir` on a background task — the network commands can
    /// take seconds and must not stall the UI — showing a spinner meanwhile.
    pub(in crate::app::state) fn spawn_git(
        &mut self,
        title: impl Into<String>,
        dir: PathBuf,
        args: Vec<String>,
    ) {
        let title = title.into();
        let tx = self.tx.clone();
        let t = title.clone();
        tokio::spawn(async move {
            let out = ops::run_text(&dir, &args).await;
            let _ = tx.send(AppEvent::GitDone { title: t, out }).await;
        });
        self.busy_git(&title);
    }

    fn busy_git(&mut self, title: &str) {
        self.dialog = Some(Dialog::Busy(BusyDialog::new("Git", format!("Running git {title}…"))));
    }

    /// Read the repository's branches/remotes in the background, then open the
    /// guided dialog that needs them.
    fn spawn_git_info(&mut self, form: GitInfoForm, dir: PathBuf) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let info = Box::new(ops::repo_info(&dir).await);
            let _ = tx.send(AppEvent::GitInfo { form, info }).await;
        });
        self.busy_git("branches");
    }

    /// A finished Git command: show what git said, then re-read the panels.
    pub(in crate::app::state) async fn on_git_done(
        &mut self,
        title: String,
        out: crate::git::ops::GitOutput,
    ) {
        // A command that succeeded silently (`add`, `restore`, …) has nothing to
        // report — just close the spinner rather than pop an empty box.
        self.dialog = if out.ok && out.text.trim().is_empty() {
            None
        } else {
            Some(Dialog::GitOutput(GitOutputDialog::new(title, out.ok, &out.text)))
        };
        // Almost every git action changes the tree, the index, or the branch.
        self.invalidate_git();
        self.reload_all().await;
    }

    /// The repository's branches arrived: open the dialog that asked for them.
    pub(in crate::app::state) fn on_git_info(
        &mut self,
        form: GitInfoForm,
        info: crate::git::ops::RepoInfo,
    ) {
        let dialog = match form {
            GitInfoForm::Checkout => {
                let choices = info.checkout_choices();
                if choices.is_empty() {
                    // A repo with no commits has no branches to switch to yet.
                    return self.show_error("This repository has no branches yet");
                }
                FormDialog::git_checkout(choices)
            }
            GitInfoForm::Push => FormDialog::git_push(info.remotes, info.current),
            GitInfoForm::Fetch => FormDialog::git_fetch(info.remotes),
        };
        self.dialog = Some(Dialog::Form(dialog));
    }

    /// Alt-G: open the menu bar straight into the File menu's Git submenu.
    pub(in crate::app::state) fn open_git_menu(&mut self) {
        self.menu = Some(MenuBarState::new_git(&self.session_list(), self.side_remote()));
        self.alt_hint = false;
    }

    /// The active panel's directory when it is on the local filesystem.
    fn git_local_dir(&self) -> Option<PathBuf> {
        let cwd = &self.panels[self.active].cwd;
        (cwd.scheme == "file" && cwd.container.is_none()).then(|| cwd.path.clone())
    }

    /// Where a `Submit::GitRun` should run: the active panel's local directory.
    pub(in crate::app::state) fn git_run_dir(&self) -> Option<PathBuf> {
        self.git_local_dir()
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
