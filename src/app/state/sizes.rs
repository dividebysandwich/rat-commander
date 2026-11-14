//! Scheduling for the shared directory-size cache.
//!
//! The crawler is steered, never restarted: everything that wants sizes calls
//! [`AppState::size_focus`] with the directory it is showing, and the crawler
//! puts that subtree at the front of its queue. A directory that was already
//! walked — as part of a parent's crawl, say — is not walked again, which is
//! what makes descending and going back up instant instead of a rescan.

use super::*;

impl AppState {
    /// Point the size crawler at `dir`, starting it if this is the first thing
    /// to ask for sizes this session.
    pub(in crate::app::state) fn size_focus(&mut self, dir: &Path) {
        match self.sizes.as_ref() {
            Some(c) => c.focus(dir),
            None => self.sizes = Some(crate::sizes::crawl::Crawler::spawn(dir.to_path_buf())),
        }
    }

    /// Whether a size crawl is in flight — drives the "scanning…" readout and
    /// keeps the animation tick alive while boxes are still growing.
    pub fn sizes_running(&self) -> bool {
        self.sizes.as_ref().is_some_and(|c| c.is_running())
    }

    /// Point each 3D panel at the *other* panel's directory, the way
    /// `update_details` points a Details panel at the other panel's cursor.
    /// Called once per loop iteration; cheap when nothing has moved.
    pub fn update_space3d(&mut self) {
        for viewer in 0..2 {
            if self.panels[viewer].format != crate::panel::ViewFormat::Space3d {
                continue;
            }
            // Before the crawlability check, so switching the setting re-forms
            // the scene even on a panel whose source is currently remote.
            let style = self.config.space3d_style;
            if let Some(sp) = self.panels[viewer].space3d.as_mut() {
                sp.set_style(style);
            }
            // While a timeline owns this panel, the scene is about a revision,
            // not about wherever the other panel's cursor has wandered to.
            if self.timeline.as_ref().is_some_and(|t| t.side == viewer) {
                continue;
            }
            let source = 1 - viewer;
            // Only a real filesystem can be crawled, so a remote or archive
            // panel leaves the view showing whatever it last had.
            if !crate::space3d::is_crawlable(&self.panels[source].cwd) {
                continue;
            }
            let dir = self.panels[source].cwd.path.clone();
            // The entry that panel's cursor is on, when it is a real
            // subdirectory. The test mirrors the crawler's own rule — a real
            // directory, never a symlink — so the highlight can only ever point
            // at a box that exists. `..` leads out of the tree entirely.
            let under_cursor = self.panels[source].current_entry().and_then(|e| {
                let real_dir = e.kind == crate::vfs::VfsKind::Dir && e.symlink_target.is_none();
                (real_dir && e.name != ".." && e.name != ".").then(|| dir.join(&e.name))
            });
            if let Some(sp) = self.panels[viewer].space3d.as_mut() {
                sp.set_focus(&dir);
                sp.set_cursor(under_cursor.as_deref());
            }
        }
    }

    /// Re-point the crawler at whatever currently needs sizing, then refresh the
    /// views from the cache. Called once per loop iteration, like
    /// `update_details` and `update_git`; cheap when nothing has moved.
    pub fn update_sizes(&mut self) {
        // Whoever is in front gets the crawler's attention: the disk explorer
        // when it is open, otherwise a 3D panel — the active one for preference,
        // but *either* will do. A 3D view is normally on the panel you are not
        // driving (it describes the other one), so keying this off the active
        // panel alone would stop it updating the moment you switched panels.
        let want = match self.diskview.as_ref() {
            Some(d) => Some(d.cwd.clone()),
            None => [self.active, 1 - self.active]
                .iter()
                // A timeline's panel has its sizes from git, not from the disk;
                // crawling for it would be pure waste.
                .filter(|&&i| self.timeline.as_ref().map(|t| t.side) != Some(i))
                .find_map(|&i| self.panels[i].space3d.as_ref().map(|sp| sp.focus.clone())),
        };
        if let Some(dir) = want
            && self.sizes_focus.as_deref() != Some(dir.as_path())
        {
            self.size_focus(&dir);
            self.sizes_focus = Some(dir);
        }
        // Always re-project: this is guarded on the crawler's own counter, so it
        // costs nothing when nothing has moved.
        self.refresh_size_views();
    }

    /// Re-project the size-backed views from the cache, but only when the
    /// crawler has actually seen something new since last time — otherwise an
    /// idle view would rebuild its box list on every frame.
    pub(in crate::app::state) fn refresh_size_views(&mut self) {
        let timeline_side = self.timeline.as_ref().map(|t| t.side);
        let Some(crawler) = self.sizes.as_ref() else {
            return self.refresh_timeline_view();
        };
        crawler.with_tree(|tree| {
            if let Some(dv) = self.diskview.as_mut()
                && tree.dirs_seen != dv.synced_at
            {
                dv.synced_at = tree.dirs_seen;
                dv.sync_from(tree);
            }
            for (i, p) in self.panels.iter_mut().enumerate() {
                // A panel under a timeline is drawn from that revision's tree,
                // applied below; the crawler's has nothing to say about it.
                if timeline_side == Some(i) {
                    continue;
                }
                if let Some(sp) = p.space3d.as_mut()
                    && tree.dirs_seen != sp.synced_at
                {
                    sp.synced_at = tree.dirs_seen;
                    sp.sync_from(tree);
                }
            }
        });
        self.refresh_timeline_view();
    }

    /// Draw a timeline's panel from the revision it is showing. The same
    /// `dirs_seen`/`synced_at` guard as the crawler's, against the epoch each
    /// delivered tree is stamped with.
    pub(in crate::app::state) fn refresh_timeline_view(&mut self) {
        let Some(tl) = self.timeline.as_ref() else { return };
        let (side, Some(tree)) = (tl.side, tl.tree().cloned()) else { return };
        if let Some(sp) = self.panels[side].space3d.as_mut()
            && tree.dirs_seen != sp.synced_at
        {
            sp.synced_at = tree.dirs_seen;
            sp.sync_from(&tree);
        }
    }
}

// -- The 3D time machine ----------------------------------------------------

impl AppState {
    /// Toggle the time machine on the active 3D panel.
    ///
    /// The 3D view describes the *other* panel, so the history scrubbed is the
    /// one belonging to the directory that panel is showing.
    pub(in crate::app::state) async fn timeline_toggle(&mut self) {
        if self.timeline.take().is_some() {
            // Leaving: the crawler takes the panel back, and the scene returns
            // to the live filesystem on the next pass.
            self.sizes_focus = None;
            return;
        }
        let viewer = self.active;
        if !self.panels[viewer].is_space3d() {
            return self.show_error(crate::l10n::tr("The time machine needs the 3D view"));
        }
        let source = 1 - viewer;
        let dir = self.panels[source].cwd.clone();
        if !crate::space3d::is_scrubbable(&dir) {
            return self.show_error(crate::l10n::tr("The time machine needs a local directory"));
        }
        let Some(toplevel) = crate::vfs::git::toplevel_of(&dir.path).await else {
            return self.show_error(crate::l10n::tr("Not a git repository"));
        };
        let limit = crate::config::DEFAULT_GIT_REV_LIMIT;
        let revs = match crate::vfs::git::rev_list(&toplevel, limit).await {
            Ok(r) => r,
            Err(e) => return self.show_error(format!("{e}")),
        };
        if revs.is_empty() {
            return self.show_error(crate::l10n::tr("This repository has no commits yet"));
        }
        // The scene is about the repository root from here on, whatever the
        // source panel was pointing at — a revision's sizes are the whole tree's.
        if let Some(sp) = self.panels[viewer].space3d.as_mut() {
            sp.set_focus(&toplevel);
            sp.set_cursor(None);
        }
        self.timeline = Some(crate::sizes::timeline::Timeline::new(viewer, toplevel, revs));
    }

    /// Step the timeline, if one is running.
    pub(in crate::app::state) fn timeline_step(&mut self, by: i64) {
        if let Some(t) = self.timeline.as_mut() {
            t.step(by);
        }
    }

    /// Jump to the oldest (`false`) or newest (`true`) revision.
    pub(in crate::app::state) fn timeline_end(&mut self, newest: bool) {
        if let Some(t) = self.timeline.as_mut() {
            let to = if newest { 0 } else { t.revs.len().saturating_sub(1) };
            t.seek(to);
        }
    }

    /// Start whatever fetch the timeline is owed, and keep the scrub row's
    /// readout in step. Called once per loop tick.
    pub fn update_timeline(&mut self) {
        use crate::sizes::timeline::Fetch;
        let snapshot = self.timeline.as_ref().map(|t| {
            let (index, total) = t.position();
            (t.side, crate::panel::ScrubRow { label: t.label(), index, total })
        });
        for (i, panel) in self.panels.iter_mut().enumerate() {
            panel.scrub = match &snapshot {
                Some((side, row)) if *side == i => Some(row.clone()),
                _ => None,
            };
        }
        let Some(t) = self.timeline.as_mut() else { return };
        let (root, want) = (t.root.clone(), t.poll(std::time::Instant::now()));
        let Fetch::Want { oid, generation } = want else { return };
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = crate::vfs::git::tree_sizes(&root, &oid).await.map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::TimelineTree { oid, generation, result }).await;
        });
    }

    /// Take delivery of a revision's sizes.
    pub(in crate::app::state) fn apply_timeline_tree(
        &mut self,
        oid: String,
        _generation: u64,
        result: Result<Vec<(String, u64)>, String>,
    ) {
        let Some(t) = self.timeline.as_mut() else { return };
        match result {
            // A reply for a revision already scrubbed past is still kept — it
            // cost a `git` call — but `deliver` only shows it if it is current.
            Ok(entries) => t.deliver(oid, entries),
            Err(e) => {
                t.fail(&oid);
                self.show_error(e);
            }
        }
    }
}
