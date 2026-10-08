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
            let cwd = &self.panels[side].cwd;
            let key = match cwd.scheme.as_str() {
                "file" => cwd.path.to_string_lossy().into_owned(),
                // A git mount labels itself with the revision it is showing.
                // Keyed on the whole path so walking between revisions relabels.
                "git" => format!("git\u{1}{}", cwd.posix_path()),
                _ => String::new(),
            };
            if key == self.git_key[side] {
                continue;
            }
            self.git_key[side] = key.clone();
            self.git_gen[side] = self.git_gen[side].wrapping_add(1);
            if cwd.scheme == "git" {
                self.panels[side].git = git_mount_label(cwd);
            } else if key.is_empty() {
                // Remote/archive panel: no VCS info.
                self.panels[side].git = None;
            } else {
                self.start_git_scan(side);
            }
        }
    }

    /// Force a re-scan of both panels' Git status on the next loop iteration —
    /// called after operations that may have changed the working tree.
    pub(in crate::app::state) fn invalidate_git(&mut self) {
        self.git_key = [String::new(), String::new()];
        // History may have moved on too (a commit, a pull): count again.
        self.activity_cache.clear();
        for d in &mut self.details {
            d.activity_key.clear();
        }
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
        let cursor_staged = self.panels[side]
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
        let Some(entry) = self.panels[side]
            .current_entry()
            .filter(|e| e.kind == VfsKind::File && e.name != "..")
            .cloned()
        else {
            return self.show_error("Put the cursor on a file to diff against HEAD");
        };
        let name = entry.name.clone();
        let work_path = self.panels[side].cwd.join(&name);
        // Path of the file relative to the repo root (for `git show HEAD:<rel>`).
        let rel = self.panels[side]
            .cwd
            .path
            .join(&name)
            .strip_prefix(&root)
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|_| std::path::PathBuf::from(&name));

        let backend = self.panels[side].backend.clone();
        let work_data = match load_file(&backend, &work_path).await {
            Ok(d) => d,
            Err(e) => return self.show_error(format!("Cannot read {name}: {e}")),
        };
        // The committed version (empty for a new/untracked file).
        let head_data = crate::git::head_blob(&root, &rel).await.unwrap_or_default();

        // Left = HEAD (read-only: a non-`file` scheme so an accidental save can't
        // overwrite anything); right = the working file.
        let head_path = VfsPath { scheme: "git-head".into(), path: rel.clone(), container: None };
        let mut view = DiffView::new(
            format!("{name}  (HEAD)"),
            head_path,
            &head_data,
            name,
            work_path,
            &work_data,
        );
        // The staging keys work off `git diff`, not off the view's own deltas —
        // see `crate::git::hunks` for the four reasons those cannot be turned
        // into an appliable patch. A file git cannot diff (untracked, say) just
        // leaves the view without hunks, still perfectly readable.
        if let Ok(unstaged) = crate::git::hunks::diff_file(&root, &rel, false).await {
            let staged_dirty = crate::git::hunks::diff_file(&root, &rel, true)
                .await
                .map(|d| !d.hunks.is_empty() || d.binary)
                .unwrap_or(false);
            view.git = Some(crate::diff::GitCtx { root, rel, unstaged, staged_dirty });
        }
        self.diffview = Some(view);
    }

    /// Point the active panel at this repository's history.
    ///
    /// The mount's root *is* the revision list, so this needs no picker of its
    /// own: you arrive at the commits and walk into one like any directory.
    pub(in crate::app::state) async fn git_browse_revisions(&mut self, dir: PathBuf) {
        let Some(toplevel) = crate::vfs::git::toplevel_of(&dir).await else {
            return self.show_error(crate::l10n::tr("Not a git repository"));
        };
        let target = VfsPath::git(crate::vfs::git::container_for(&toplevel), "/");
        let backend = match self.registry.resolve(&target) {
            Ok(b) => b,
            Err(e) => return self.show_error(format!("Cannot open location: {e}")),
        };
        let side = self.active;
        self.panels[side].try_enter(target, backend, None).await;
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
            M::GitBrowseRev => self.git_browse_revisions(dir).await,
            M::GitStatus => self.spawn_git("status", dir, ops::status_args()),
            M::GitLog => self.spawn_git("log", dir, ops::log_args()),
            M::GitDiff => self.open_git_diff().await,
            M::GitStage => self.git_stage_toggle().await,
            M::GitCommit => self.dialog = Some(Dialog::Form(FormDialog::git_commit())),
            M::GitStash => self.dialog = Some(Dialog::Form(FormDialog::git_stash())),
            M::GitStashList => self.spawn_git_stashes(dir),
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
        let handle = tokio::spawn(async move {
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
        self.busy_git("sync", handle);
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
        let handle = tokio::spawn(async move {
            let out = ops::run_text(&dir, &args).await;
            let _ = tx.send(AppEvent::GitDone { title: t, out }).await;
        });
        self.busy_git(&title, handle);
    }

    /// Show the cancellable "running git…" spinner and remember `handle` so Esc on
    /// it can abort a hung network op (see [`AppState::busy_task`]).
    fn busy_git(&mut self, title: &str, handle: tokio::task::JoinHandle<()>) {
        self.busy_task = Some(handle);
        self.dialog = Some(Dialog::Busy(
            BusyDialog::new("Git", format!("Running git {title}…")).cancellable(),
        ));
    }

    // -- Hunk staging from the Alt-D diff ----------------------------------

    /// The patch for the hunk under the cursor, or why there isn't one.
    fn hunk_patch(&self) -> std::result::Result<(PathBuf, String), &'static str> {
        let Some(view) = self.diffview.as_ref() else { return Err("No diff is open") };
        let Some(git) = view.git.as_ref() else {
            return Err("Hunk staging needs the Git diff (Alt-D)");
        };
        if git.unstaged.binary {
            return Err("This file is binary");
        }
        if git.unstaged.hunks.is_empty() {
            return Err("Nothing is unstaged in this file");
        }
        let Some(i) = view.hunk_at_cursor() else { return Err("No hunk under the cursor") };
        let patch = crate::git::hunks::single_hunk_patch(&git.unstaged, &git.unstaged.hunks[i]);
        Ok((git.root.clone(), patch))
    }

    /// Stage just the hunk under the cursor.
    pub(in crate::app::state) async fn stage_hunk_under_cursor(&mut self) {
        let (root, patch) = match self.hunk_patch() {
            Ok(v) => v,
            Err(e) => return self.show_error(e),
        };
        self.run_patch(root, patch, true, false, "stage hunk").await;
    }

    /// Discarding throws away work that was never committed, so it asks first.
    pub(in crate::app::state) fn confirm_discard_hunk(&mut self) {
        if let Err(e) = self.hunk_patch() {
            return self.show_error(e);
        }
        self.dialog = Some(Dialog::Confirm(ConfirmDialog::discard_hunk()));
    }

    /// The confirmation came back yes: reverse the hunk out of the worktree.
    pub(in crate::app::state) async fn discard_hunk_confirmed(&mut self) {
        let (root, patch) = match self.hunk_patch() {
            Ok(v) => v,
            Err(e) => return self.show_error(e),
        };
        self.run_patch(root, patch, false, true, "discard hunk").await;
    }

    /// Unstage the whole file.
    ///
    /// Whole-file rather than per-hunk on purpose: unstaging one hunk needs the
    /// hunks of `git diff --cached` (index against HEAD), and this view has no
    /// row that corresponds to an index line — its panes are HEAD and the
    /// worktree. A third pane would be needed to put a cursor on one.
    pub(in crate::app::state) fn unstage_diff_file(&mut self) {
        let Some(view) = self.diffview.as_ref() else { return };
        let Some(git) = view.git.as_ref() else {
            return self.show_error("Unstaging needs the Git diff (Alt-D)");
        };
        let names = vec![git.rel.to_string_lossy().into_owned()];
        let (root, args) = (git.root.clone(), ops::unstage_args(&names));
        self.spawn_git("restore", root, args);
    }

    /// Feed a patch to `git apply`, then refresh what the user is looking at.
    async fn run_patch(
        &mut self,
        root: PathBuf,
        patch: String,
        cached: bool,
        reverse: bool,
        what: &str,
    ) {
        let out = ops::apply_patch(&root, &patch, cached, reverse).await;
        if !out.ok {
            let why = if out.text.trim().is_empty() { "git apply failed".into() } else { out.text };
            return self.show_error(format!("Cannot {what}: {why}"));
        }
        self.invalidate_git();
        self.refresh_git_diff().await;
        self.reload_all().await;
        if let Some(v) = self.diffview.as_mut() {
            // Two literal calls rather than one over a conditional, so the
            // catalog audit can see both keys.
            v.set_status(if reverse {
                crate::l10n::trd("Hunk discarded")
            } else {
                crate::l10n::trd("Hunk staged")
            });
        }
    }

    /// Re-read the open git diff in place, keeping the cursor where it was.
    ///
    /// Called after *any* git action while the diff is open, not just a staging
    /// one — a commit or a checkout changes what the diff should show just as
    /// much.
    pub(in crate::app::state) async fn refresh_git_diff(&mut self) {
        let Some(view) = self.diffview.as_ref() else { return };
        let Some(git) = view.git.as_ref() else { return };
        let (root, rel) = (git.root.clone(), git.rel.clone());
        let unstaged = match crate::git::hunks::diff_file(&root, &rel, false).await {
            Ok(d) => d,
            Err(_) => return, // the file may have gone; leave the view as it is
        };
        let staged_dirty = crate::git::hunks::diff_file(&root, &rel, true)
            .await
            .map(|d| !d.hunks.is_empty() || d.binary)
            .unwrap_or(false);
        if let Some(v) = self.diffview.as_mut()
            && let Some(g) = v.git.as_mut()
        {
            g.unstaged = unstaged;
            g.staged_dirty = staged_dirty;
        }
    }

    /// Read the repository's stashes in the background, then open the picker.
    fn spawn_git_stashes(&mut self, dir: PathBuf) {
        let tx = self.tx.clone();
        let handle = tokio::spawn(async move {
            let stashes = ops::stash_list(&dir).await;
            let _ = tx.send(AppEvent::GitStashes { stashes }).await;
        });
        self.busy_git("stash list", handle);
    }

    /// The stashes arrived: show them, or say there are none rather than
    /// opening an empty box with nothing to pick.
    pub(in crate::app::state) fn on_git_stashes(
        &mut self,
        stashes: Vec<crate::git::ops::StashEntry>,
    ) {
        self.busy_task = None; // the task delivered its result
        if stashes.is_empty() {
            return self.show_error("This repository has no stashes");
        }
        self.dialog = Some(Dialog::Stash(StashDialog::new(stashes)));
    }

    /// Read the repository's branches/remotes in the background, then open the
    /// guided dialog that needs them.
    fn spawn_git_info(&mut self, form: GitInfoForm, dir: PathBuf) {
        let tx = self.tx.clone();
        let handle = tokio::spawn(async move {
            let info = Box::new(ops::repo_info(&dir).await);
            let _ = tx.send(AppEvent::GitInfo { form, info }).await;
        });
        self.busy_git("branches", handle);
    }

    /// A finished Git command: show what git said, then re-read the panels.
    pub(in crate::app::state) async fn on_git_done(
        &mut self,
        title: String,
        out: crate::git::ops::GitOutput,
    ) {
        self.busy_task = None; // the task delivered its result
        // A command that succeeded silently (`add`, `restore`, …) has nothing to
        // report — just close the spinner rather than pop an empty box.
        self.dialog = if out.ok && out.text.trim().is_empty() {
            None
        } else {
            Some(Dialog::GitOutput(GitOutputDialog::new(title, out.ok, &out.text)))
        };
        // Almost every git action changes the tree, the index, or the branch.
        self.invalidate_git();
        // A commit or a checkout changes what an open diff should show just as
        // much as staging does, so refresh it here rather than at each caller.
        self.refresh_git_diff().await;
        self.reload_all().await;
    }

    /// The repository's branches arrived: open the dialog that asked for them.
    pub(in crate::app::state) fn on_git_info(
        &mut self,
        form: GitInfoForm,
        info: crate::git::ops::RepoInfo,
    ) {
        self.busy_task = None; // the task delivered its result
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
        self.menu = Some(
            MenuBarState::new_git(&self.session_list(), self.side_remote())
                .with_sort(self.panel_sorts()),
        );
        self.alt_hint = false;
    }

    /// The active panel's directory when it is on the local filesystem.
    fn git_local_dir(&self) -> Option<PathBuf> {
        let cwd = &self.panels[self.active].cwd;
        cwd.is_plain_local().then(|| cwd.path.clone())
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

/// The border label for a `git://` panel: the revision it is showing.
///
/// A committed tree has no working-tree state, so `files` is empty and no entry
/// gets a status glyph — which is correct, not a shortcut. Reusing `GitStatus`
/// means the existing `⎇` renderer draws it with no new code.
fn git_mount_label(cwd: &VfsPath) -> Option<crate::git::GitStatus> {
    let root = cwd.container.as_ref()?.parent()?.to_path_buf();
    let inner = cwd.posix_path();
    let rev = inner.trim_start_matches('/').split('/').next().unwrap_or("");
    // At the mount root the panel is listing revisions, not standing in one.
    let branch = if rev.is_empty() {
        crate::l10n::tr("history").to_string()
    } else {
        let mut fields = rev.splitn(4, '_');
        let short = fields.nth(2).unwrap_or(rev);
        let subject = fields.next().unwrap_or("").replace('-', " ");
        if subject.is_empty() { short.to_string() } else { format!("{short} {subject}") }
    };
    Some(crate::git::GitStatus { branch, ahead: 0, behind: 0, files: HashMap::new(), root })
}
