//! Per-panel back/forward directory history, the directory hotlist (Ctrl-\), and
//! the persistent listing filter.

use super::*;

impl AppState {
    /// Step panel `side` back to the previous directory in its history. The move
    /// is recorded manually here (with `in_history_nav` set so `try_enter` does
    /// not also push it), moving the current directory onto the forward stack.
    pub(in crate::app::state) async fn go_back(&mut self, side: usize) {
        let Some(target) = self.panels[side].back.last().cloned() else {
            return;
        };
        if !self.history_target_allowed(side, &target) {
            return;
        }
        let backend = match self.registry.resolve(&target) {
            Ok(b) => b,
            Err(e) => return self.show_error(format!("Cannot open location: {e}")),
        };
        let current = self.panels[side].cwd.clone();
        self.panels[side].in_history_nav = true;
        let ok = self.panels[side].try_enter(target, backend, None).await;
        self.panels[side].in_history_nav = false;
        if ok {
            self.panels[side].back.pop();
            self.panels[side].forward.push(current);
        }
    }

    /// Step panel `side` forward to the next directory in its history.
    pub(in crate::app::state) async fn go_forward(&mut self, side: usize) {
        let Some(target) = self.panels[side].forward.last().cloned() else {
            return;
        };
        if !self.history_target_allowed(side, &target) {
            return;
        }
        let backend = match self.registry.resolve(&target) {
            Ok(b) => b,
            Err(e) => return self.show_error(format!("Cannot open location: {e}")),
        };
        let current = self.panels[side].cwd.clone();
        self.panels[side].in_history_nav = true;
        let ok = self.panels[side].try_enter(target, backend, None).await;
        self.panels[side].in_history_nav = false;
        if ok {
            self.panels[side].forward.pop();
            self.panels[side].back.push(current);
        }
    }

    /// Whether stepping panel `side` to `target` in history is allowed. Retracing
    /// into a *remote* directory is refused when the other panel is already
    /// remote, upholding the "one panel stays local" invariant (which keeps
    /// copies from crossing directly between two servers).
    fn history_target_allowed(&mut self, side: usize, target: &VfsPath) -> bool {
        if target.is_remote() && self.other_panel_is_remote(side) {
            self.show_error(
                "The other panel is already remote — one panel must stay local.".to_string(),
            );
            return false;
        }
        true
    }

    /// Alt-H: the active panel's visited directories as a pickable list — the
    /// same history `Alt-Y` / `Alt-U` step through, but jumpable.
    pub(in crate::app::state) fn open_dir_history(&mut self) {
        let p = &self.panels[self.active];
        self.dialog =
            Some(Dialog::DirHistory(DirHistoryDialog::new(&p.back, &p.cwd, &p.forward)));
    }

    /// Jump the active panel to a directory picked from the history list. The
    /// move is an ordinary navigation, so it lands on the history stacks itself
    /// (retracing then stays consistent with where you actually went).
    pub(in crate::app::state) async fn goto_dir(&mut self, target: VfsPath) {
        let side = self.active;
        if self.panels[side].cwd == target {
            return;
        }
        self.enter_on(side, target).await;
    }

    /// Alt-I: point the *other* panel at the active panel's directory, so both
    /// show the same place (Midnight Commander's "panel sync").
    pub(in crate::app::state) async fn sync_other_panel(&mut self) {
        let target = self.panels[self.active].cwd.clone();
        let other = self.other_index();
        if self.panels[other].cwd == target {
            return; // already there: nothing to do, and no history entry
        }
        self.enter_on(other, target).await;
    }

    /// Alt-O: show the cursor's directory on the *other* panel and step down one
    /// entry, so holding Alt-O walks a directory list while the other panel keeps
    /// pace. With the cursor on a file, the other panel gets this directory
    /// instead — the useful reading of "show me where I am".
    pub(in crate::app::state) async fn chdir_other_panel(&mut self) {
        let side = self.active;
        let entry = self.panels[side].current_entry().cloned();
        let target = match entry {
            // `..` names the parent, and entering it is what the user means.
            Some(e) if e.kind == VfsKind::Dir => self.panels[side].cwd.join(&e.name),
            // A file (or an empty listing): the other panel follows this one.
            _ => self.panels[side].cwd.clone(),
        };
        let other = self.other_index();
        if self.panels[other].cwd != target {
            self.enter_on(other, target).await;
        }
        // Advance regardless, so repeating the key walks the listing either way.
        self.panels[side].move_cursor(1);
    }

    /// Move panel `side` to `target`, resolving its backend and reporting any
    /// error. The move records history like an ordinary navigation.
    async fn enter_on(&mut self, side: usize, target: VfsPath) {
        if !self.history_target_allowed(side, &target) {
            return;
        }
        let backend = match self.registry.resolve(&target) {
            Ok(b) => b,
            Err(e) => return self.show_error(format!("Cannot open location: {e}")),
        };
        self.panels[side].try_enter(target, backend, None).await;
    }

    /// If `(col, row)` falls on a panel's `◀`/`▶` history arrow, return
    /// `(side, is_back)`. Used by the mouse handler to run back/forward.
    pub(in crate::app::state) fn history_arrow_at(&self, col: u16, row: u16) -> Option<(usize, bool)> {
        let on = |r: Option<Rect>| r.is_some_and(|r| col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height);
        for side in 0..2 {
            if self.panel_hidden[side] {
                continue;
            }
            if on(self.panels[side].back_arrow) {
                return Some((side, true));
            }
            if on(self.panels[side].fwd_arrow) {
                return Some((side, false));
            }
        }
        None
    }

    // -- Directory hotlist (bookmarks) -------------------------------------

    /// Open the directory hotlist over the active panel's bookmarks.
    pub(in crate::app::state) fn open_hotlist(&mut self) {
        let current = {
            let p = &self.panels[self.active];
            (p.cwd.scheme == "file").then(|| p.cwd.path.to_string_lossy().to_string())
        };
        self.dialog = Some(Dialog::Hotlist(HotlistDialog::new(
            self.config.bookmarks.clone(),
            current,
        )));
    }

    /// Apply the hotlist's result: persist any edited bookmark list, and jump the
    /// active panel to the chosen directory.
    pub(in crate::app::state) async fn apply_hotlist_outcome(&mut self, outcome: HotlistOutcome) {
        match outcome {
            HotlistOutcome::Save(bookmarks) => {
                self.config.bookmarks = bookmarks;
                self.save_config_reporting();
            }
            HotlistOutcome::Jump { path, bookmarks } => {
                self.config.bookmarks = bookmarks;
                self.save_config_reporting();
                self.jump_to_local_dir(path).await;
            }
        }
    }

    // -- Persistent listing filter -----------------------------------------

    /// Prompt for a listing filter for the active panel (prefilled with the
    /// current one; a blank entry clears it).
    pub(in crate::app::state) fn open_panel_filter(&mut self) {
        let side = self.active;
        let initial = self.panels[side].filter.clone().unwrap_or_default();
        self.dialog = Some(Dialog::Input(InputDialog::new(
            "Panel filter",
            "Filter pattern (blank clears)",
            initial,
            InputPurpose::PanelFilter(side),
        )));
    }

    /// Set (or clear) panel `side`'s persistent listing filter and reload so it
    /// takes effect.
    pub(in crate::app::state) async fn apply_panel_filter(&mut self, side: usize, pattern: String) {
        self.panels[side].filter = if pattern.trim().is_empty() {
            None
        } else {
            Some(pattern)
        };
        let _ = self.panels[side].reload().await;
    }
}
