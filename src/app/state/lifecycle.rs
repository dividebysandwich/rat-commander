//! Construction, the tick/event-loop plumbing, and small panel helpers.

use super::*;

impl AppState {
    pub fn new(tx: AppSender) -> Self {
        let registry = Registry::new();
        let local = registry.local();
        let cwd = VfsPath::local_cwd();
        let mut left = Panel::new(local.clone(), cwd.clone());
        let mut right = Panel::new(local, cwd.clone());
        let config = Config::load();
        // Restore each panel's remembered listing format and sort order.
        left.format = config.panels[0].format;
        left.sort = config.panels[0].sort;
        right.format = config.panels[1].format;
        right.sort = config.panels[1].sort;
        // Restored layout scalars (captured before `config` is moved below).
        let restored_active = config.active_panel.min(1);
        // The initially-active panel opens at the current directory (where rc was
        // launched), so you land where you were working; the *other* panel restores
        // its last local directory (if it still exists). Each panel's persistent
        // filter is restored either way. `init()` reloads the listings afterward.
        for (i, panel) in [&mut left, &mut right].into_iter().enumerate() {
            if i != restored_active {
                let dir = &config.panel_dirs[i];
                if !dir.is_empty() && std::path::Path::new(dir).is_dir() {
                    panel.cwd = VfsPath::local(dir);
                }
            }
            let filter = &config.panel_filters[i];
            if !filter.is_empty() {
                panel.filter = Some(filter.clone());
            }

            // Restore the saved tabs, dropping any whose directory has gone.
            // The *active* panel keeps its launch directory in the front tab
            // (as above), so only its background tabs come back.
            let saved = &config.panel_tabs[i];
            if saved.len() > 1 {
                let mut tabs: Vec<crate::panel::tabs::TabState> = saved
                    .iter()
                    .filter(|r| !r.dir.is_empty() && std::path::Path::new(&r.dir).is_dir())
                    .map(|r| {
                        let mut t = crate::panel::tabs::TabState::new(
                            VfsPath::local(&r.dir),
                            r.view.format,
                            r.view.sort,
                        );
                        t.filter = (!r.filter.is_empty()).then(|| r.filter.clone());
                        t
                    })
                    .collect();
                if !tabs.is_empty() {
                    let active_tab = config.panel_tab_active[i].min(tabs.len() - 1);
                    // The panel itself is showing `panel.cwd`, so the front tab
                    // has to agree with it or the first switch would jump.
                    tabs[active_tab] = crate::panel::tabs::TabState::new(
                        panel.cwd.clone(),
                        panel.format,
                        panel.sort,
                    );
                    tabs[active_tab].filter = panel.filter.clone();
                    panel.tabs = tabs;
                    panel.tab = active_tab;
                }
            }
        }
        let restored_split =
            if config.split_horizontal { SplitDir::Horizontal } else { SplitDir::Vertical };
        let restored_hidden = config.panel_hidden;
        let restored_half = config.half_height;
        // "Go local" returns each panel to where it started — the active panel's
        // current directory, the other's restored directory.
        let last_local = [left.cwd.clone(), right.cwd.clone()];
        let truecolor = config.truecolor.unwrap_or_else(detect_truecolor);
        let theme = Theme::by_name(&config.theme, truecolor);
        // Started from within another instance's Ctrl-O subshell? This instance
        // can't provide its own subshell, so it's disabled. The warning dialog is
        // raised later (see `warn_nested_subshell`), once the UI language loads.
        let subshell_disabled = crate::shell::in_subshell();
        // Restore the persistent command history (capped at the configured max).
        let mut cmd = CommandLine::new();
        cmd.history_max = config.command_history_max;
        cmd.history = crate::config::load_command_history(config.command_history_max);
        AppState {
            panels: [left, right],
            active: restored_active,
            split: restored_split,
            panel_hidden: restored_hidden,
            half_height: restored_half,
            // Sized to a sane default; resized to the backdrop area on each draw.
            console: crate::console::Console::new(24, 80),
            cmd,
            dialog: None,
            viewer: None,
            editor: None,
            editor_stack: Vec::new(),
            menu: None,
            force_clear: false,
            procview: None,
            diskview: None,
            sizes: None,
            timeline: None,
            sizes_focus: None,
            diffview: None,
            hexdiff: None,
            mountview: None,
            netview: None,
            theme_editor: None,
            pending_sudo: None,
            pending_connect: None,
            pending_priv_answer: None,
            pending_flash: None,
            pending_image: None,
            flash_tasks: HashMap::new(),
            theme,
            config,
            registry,
            sessions: Vec::new(),
            last_local_cwd: last_local,
            tasks: HashMap::new(),
            task_progress: HashMap::new(),
            op_source: HashMap::new(),
            next_task_id: 1,
            next_session_id: 0,
            tx,
            truecolor,
            anim_phase: 0,
            tick_count: 0,
            sampler: crate::util::sysinfo::SysSampler::new(),
            theme_backup: None,
            lang_backup: None,
            reshape_backup: None,
            gfx: None,
            trim: Default::default(),
            graphics_backup: None,
            space3d_backup: None,
            settings_tab: SettingsTab::default(),
            user_menu: usermenu::load_or_create(),
            ext_rules: crate::ext::ExtRules::load_or_create(),
            pending_run: None,
            pending_run_fg: None,
            pending_menu: None,
            pending_quit: false,
            alt_hint: false,
            pending_esc: None,
            quick_search: None,
            stashed_progress: None,
            last_area: Rect::new(0, 0, 0, 0),
            frame_at: Instant::now(),
            paint_last: None,
            drag_orbit: None,
            last_click: None,
            details: Default::default(),
            git_key: [String::new(), String::new()],
            git_gen: [0, 0],
            pending_focus: None,
            search_memory: Default::default(),
            find_hit_lines: HashMap::new(),
            watcher: None,
            watch_key: [String::new(), String::new()],
            watch_armed: Default::default(),
            watch_refused: Default::default(),
            fs_inbox: Default::default(),
            thumb_slots: std::sync::Arc::new(tokio::sync::Semaphore::new(crate::thumbs::PARALLEL)),
            watch_dirty: [None, None],
            edit_only: false,
            kbd_enhanced: false,
            subshell_disabled,
            send_server: None,
            receive_server: None,
            busy_task: None,
            last_panelize: String::new(),
            blame_task: None,
            blame_gen: 0,
            geo_gen: 0,
            rename_gen: 0,
            activity_cache: std::collections::VecDeque::new(),
            activity_task: [None, None],
            audio_out: crate::audio::AudioOut::new(),
            last_input: Instant::now(),
            saver: None,
            saver_dialog: None,
        }
    }

    /// Copy the current session layout — each panel's view/sort, its local
    /// directory and filter, plus the split, hidden/half-height flags and the
    /// active panel — into the config and persist it, so the next run opens where
    /// this one left off. Called on exit.
    pub fn persist_panel_views(&mut self) {
        self.capture_session();
        let _ = self.config.save();
    }

    /// Fold the live session layout into `self.config` (without saving), so a
    /// following `save()` persists it. Split out from [`persist_panel_views`] so
    /// it is testable without writing to the real config file.
    pub(in crate::app::state) fn capture_session(&mut self) {
        for i in 0..2 {
            self.config.panels[i] = crate::config::PanelView {
                format: self.panels[i].format,
                sort: self.panels[i].sort,
            };
            // Only a local directory can be restored; a remote/archive location
            // needs credentials we don't keep, so it isn't saved.
            let cwd = &self.panels[i].cwd;
            self.config.panel_dirs[i] = if cwd.is_plain_local() {
                cwd.path.to_string_lossy().into_owned()
            } else {
                String::new()
            };
            self.config.panel_filters[i] = self.panels[i].filter.clone().unwrap_or_default();

            // Tabs, on the same "only what can be restored" rule as panel_dirs:
            // a remote or in-archive tab needs state we deliberately don't keep.
            self.panels[i].sync_active_tab();
            let mut records = Vec::new();
            let mut active = 0;
            for (idx, tab) in self.panels[i].tabs.iter().enumerate() {
                if !tab.cwd.is_plain_local() {
                    continue;
                }
                if idx == self.panels[i].tab {
                    active = records.len();
                }
                records.push(crate::config::TabRecord {
                    dir: tab.cwd.path.to_string_lossy().into_owned(),
                    filter: tab.filter.clone().unwrap_or_default(),
                    view: crate::config::PanelView { format: tab.format, sort: tab.sort },
                });
            }
            // One tab is the ordinary case and is already covered by
            // `panel_dirs`; saving it too would just be duplication.
            self.config.panel_tabs[i] = if records.len() > 1 { records } else { Vec::new() };
            self.config.panel_tab_active[i] = active;
        }
        self.config.split_horizontal = matches!(self.split, SplitDir::Horizontal);
        self.config.panel_hidden = self.panel_hidden;
        self.config.half_height = self.half_height;
        self.config.active_panel = self.active;
    }

    /// The directory a calling shell should `cd` to after we quit, for
    /// `rc --print-last-dir`.
    ///
    /// Only a real local directory is useful to a shell, so the two non-local
    /// cases fall back: inside an **archive** (or extfs) we hand back the
    /// directory *holding* the archive, and on a **remote** panel we hand back
    /// the local directory that panel last showed. Anything that still isn't a
    /// directory (a deleted cwd, say) degrades to the process's own cwd.
    pub(in crate::app::state) fn last_dir_for_shell(&self) -> std::path::PathBuf {
        let cwd = &self.panels[self.active].cwd;
        let candidate = if let Some(container) = &cwd.container {
            // Archive/extfs: the container is a real file on local disk.
            container.parent().map(std::path::Path::to_path_buf)
        } else if cwd.is_remote() {
            Some(self.last_local_cwd[self.active].path.clone())
        } else {
            Some(cwd.path.clone())
        };
        match candidate {
            Some(p) if p.is_dir() => p,
            _ => std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
        }
    }

    /// Write [`last_dir_for_shell`](Self::last_dir_for_shell) to `file` for the
    /// calling shell to read. Best effort: a failure here must not stop the exit
    /// path, and the wrapper function simply doesn't `cd`.
    ///
    /// The raw OS bytes are written rather than a lossy string, so a directory
    /// whose name isn't valid UTF-8 still round-trips into the shell.
    pub fn write_last_dir(&self, file: &std::path::Path) {
        let dir = self.last_dir_for_shell();
        #[cfg(unix)]
        let bytes = {
            use std::os::unix::ffi::OsStrExt;
            dir.as_os_str().as_bytes().to_vec()
        };
        #[cfg(not(unix))]
        let bytes = dir.to_string_lossy().into_owned().into_bytes();
        let mut out = bytes;
        out.push(b'\n');
        let _ = std::fs::write(file, out);
    }

    /// Persist the command-line history to disk (capped at the configured max),
    /// so recent commands survive across sessions. Called on exit.
    pub fn persist_command_history(&self) {
        crate::config::save_command_history(&self.cmd.history, self.config.command_history_max);
    }

    /// Periodic tick (~100 ms): advances animation and samples system stats.
    /// Returns true when something visible changed (so the loop can redraw).
    pub fn on_tick(&mut self) -> bool {
        let mut dirty = false;
        self.tick_count = self.tick_count.wrapping_add(1);
        // Animate gradients when truecolor is on and either animations are
        // enabled, the (always-animated) process explorer is open, or a file
        // operation is running (so the progress bars pulse).
        let scanning_disk = self.diskview.as_ref().is_some_and(|d| d.scanning);
        let animate = self.truecolor
            && (self.config.animation
                || self.procview.is_some()
                || !self.tasks.is_empty()
                || scanning_disk);
        if animate {
            self.anim_phase = self.anim_phase.wrapping_add(1);
            dirty = true;
        }
        if self.config.system_status && self.tick_count.is_multiple_of(5) {
            // Sample roughly every 500 ms.
            self.sampler.sample();
            dirty = true;
        }
        // Refresh the process explorer on its (user-adjustable) interval.
        if let Some(pv) = self.procview.as_mut() {
            if pv.tick_due() {
                pv.refresh();
            }
            dirty = true;
        }
        // Keep the disk-mounter lists fresh (~every 500 ms), unless a dialog is
        // open over it (e.g. entering a path), to avoid the lists shifting.
        if self.dialog.is_none()
            && self.tick_count.is_multiple_of(5)
            && let Some(mv) = self.mountview.as_mut()
        {
            mv.refresh();
            dirty = true;
        }
        // The screensaver animates on this tick, and makes way for a dialog that
        // appeared while it was up.
        if self.saver.is_some() {
            if self.dialog.as_ref().map(std::mem::discriminant) != self.saver_dialog {
                self.stop_saver();
            } else if let Some(s) = self.saver.as_mut() {
                s.step();
            }
            dirty = true;
        }
        // Follow mode: pick up whatever was appended to the viewed file.
        if let Some(v) = self.viewer.as_mut()
            && v.poll_follow()
        {
            dirty = true;
        }
        // Binary mode: show an analysis that has finished in the background.
        if let Some(v) = self.viewer.as_mut()
            && v.poll_binary()
        {
            dirty = true;
        }
        // Audio: the picture filling in, and the play position moving on.
        if let Some(v) = self.viewer.as_mut()
            && v.poll_audio()
        {
            dirty = true;
        }
        if self.poll_details_audio() {
            dirty = true;
        }
        // The editor's JSON check: start one once typing pauses, show its result.
        if let Some(ed) = self.editor.as_mut()
            && ed.poll_check(Instant::now())
        {
            dirty = true;
        }
        // The hex editor's binary template: its run finishing, or rerunning
        // after an edit.
        if let Some(ed) = self.editor.as_mut()
            && ed.poll_template(Instant::now())
        {
            dirty = true;
        }
        // A binary compare's scan moving on, or a step that waited for it.
        if let Some(hd) = self.hexdiff.as_mut()
            && hd.poll()
        {
            dirty = true;
        }
        // Spin the "working…" dialog while a privileged op runs.
        if let Some(Dialog::Busy(b)) = self.dialog.as_mut() {
            b.tick();
            dirty = true;
        }
        // Periodically re-scan the network explorer (live traffic counts), but not
        // while a dialog is open over it (e.g. the password prompt).
        if self.dialog.is_none() {
            let due = self.netview.as_mut().is_some_and(|nv| nv.tick_due());
            if due {
                self.start_network_scan();
            }
            if self.netview.is_some() {
                dirty = true;
            }
        }
        dirty
    }

    /// Whether the loop needs periodic ticks at all (animation or stats on).
    pub fn wants_ticks(&self) -> bool {
        (self.config.animation && self.truecolor)
            || self.config.system_status
            || self.pending_esc.is_some()
            || self.procview.is_some()
            || self.mountview.is_some()
            || self.netview.is_some()
            || !self.tasks.is_empty()
            || matches!(self.dialog, Some(Dialog::Busy(_)))
            || self.sizes_running()
            // The screensaver animates on the tick.
            || self.saver.is_some()
            // An Activity log's ages and event rate move on by themselves.
            || self.activity_shown()
            // A followed file is polled for growth on the tick, and a binary's
            // background analysis for its result.
            || self.viewer.as_ref().is_some_and(|v| v.following() || v.analyzing() || v.audio_busy())
            // An edited JSON file is due a syntax check, or one is running.
            || self.editor.as_ref().is_some_and(|e| e.check_pending())
            // A binary template is running over the hex editor's file, or due to.
            || self.editor.as_ref().is_some_and(|e| e.template_pending())
            // A binary compare is still scanning for differences.
            || self.hexdiff.as_ref().is_some_and(|h| h.busy())
            // A Details view's audio is still being analysed, or is playing.
            || self.details.iter().any(|d| d.audio.as_ref().is_some_and(|a| a.busy()))
            // A debounced panel reload is still waiting to fire.
            || self.watch_pending()
            // A scrubbed revision is still waiting to be fetched. Without this
            // the debounce would never come round and the scene would sit on
            // the commit before the one you asked for.
            || self.timeline.as_ref().is_some_and(|t| t.pending())
    }

    /// Whether the ~30 fps frame ticker should run: only while a 3D panel has
    /// something actually moving. A settled camera over a finished crawl returns
    /// false, so an idle 3D panel costs no more CPU than an idle Full view.
    pub fn wants_frames(&self) -> bool {
        // Nothing behind the screensaver is on screen to animate.
        self.saver.is_none()
            && self.panels.iter().any(|p| p.space3d.as_ref().is_some_and(|s| s.needs_frames()))
    }

    /// Whether the screensaver is waiting to start: it is turned on, and not
    /// already up.
    pub fn saver_armed(&self) -> bool {
        self.config.screensaver_minutes > 0 && self.saver.is_none()
    }

    /// When the screensaver starts if nothing is pressed before then.
    pub fn saver_deadline(&self) -> Instant {
        self.last_input + Duration::from_secs(u64::from(self.config.screensaver_minutes) * 60)
    }

    /// Put the screensaver up (idle timeout, or the palette's "Start
    /// screensaver").
    pub fn start_saver(&mut self) {
        self.saver = Some(crate::saver::Saver::new(
            self.config.screensaver,
            crate::util::rng::Rng::seeded(),
        ));
        self.saver_dialog = self.dialog.as_ref().map(std::mem::discriminant);
        // A full repaint on the way in and out: Sixel and iTerm2 pictures stay
        // on screen until their cells are actually rewritten.
        self.force_clear = true;
    }

    pub fn stop_saver(&mut self) {
        if self.saver.take().is_some() {
            self.force_clear = true;
        }
        self.last_input = Instant::now();
    }

    /// Advance the 3D views' camera and box-size animations.
    pub fn on_frame(&mut self) {
        let now = std::time::Instant::now();
        for p in self.panels.iter_mut() {
            if let Some(sp) = p.space3d.as_mut() {
                sp.advance(now);
            }
        }
    }

    /// Load both panels' directories.
    pub async fn init(&mut self) {
        let _ = self.panels[0].reload().await;
        let _ = self.panels[1].reload().await;
        // Rebuild the directory tree for any panel restored into Tree view.
        for i in 0..2 {
            if self.panels[i].is_tree() {
                self.panels[i].build_tree().await;
            }
            if self.panels[i].is_space3d() {
                self.panels[i].build_space3d(self.config.space3d_style);
            }
        }
    }

    pub(in crate::app::state) fn active_panel(&mut self) -> &mut Panel {
        &mut self.panels[self.active]
    }

    pub(in crate::app::state) fn other_index(&self) -> usize {
        1 - self.active
    }

    /// The directory shown on the command-line prompt (and used as the cwd for a
    /// typed shell command). Normally the active panel's directory, but in Tree
    /// view it is the directory last committed with Enter (which also moves the
    /// other panel) — so it changes only on Enter, not as the cursor browses.
    pub(crate) fn console_cwd(&self) -> VfsPath {
        let p = &self.panels[self.active];
        if p.format == ViewFormat::Tree
            && let Some(tree) = p.tree.as_ref()
        {
            return tree.current.clone();
        }
        p.cwd.clone()
    }

    /// Whether the panels draw Nerd Font type glyphs instead of the `ls -F`
    /// classify characters. While the Settings dialog is open this follows its
    /// live checkbox, so the listings behind it update as the box is ticked and
    /// snap back when the dialog is cancelled (nothing is stored until submit).
    pub(crate) fn nerd_font_active(&self) -> bool {
        match &self.dialog {
            Some(Dialog::Form(fd)) => {
                fd.check_value("Nerd Font symbols").unwrap_or(self.config.nerd_font)
            }
            _ => self.config.nerd_font,
        }
    }

    /// Whether the active UI theme has a dark background (picks a fitting syntax
    /// highlighting theme).
    pub(in crate::app::state) fn dark_ui(&self) -> bool {
        if let ratatui::style::Color::Rgb(r, g, b) = self.theme.panel_bg {
            let luma = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
            luma < 128.0
        } else {
            true
        }
    }

    /// Reload both panels after a filesystem-changing operation.
    pub async fn reload_all(&mut self) {
        for p in self.panels.iter_mut() {
            let _ = p.refresh().await;
        }
        // The working tree may have changed (delete/copy/move/save): re-scan git.
        self.invalidate_git();
    }

    /// Aggregate progress of the **background** transfers (those not currently
    /// shown as the foreground progress dialog): `(bytes done, bytes total,
    /// count)`, or `None` when nothing is running in the background.
    pub(crate) fn background_summary(&self) -> Option<(u64, u64, usize)> {
        let foreground = match &self.dialog {
            Some(Dialog::Progress(p)) => Some(p.id),
            _ => None,
        };
        let (mut done, mut total, mut count) = (0u64, 0u64, 0usize);
        for (id, t) in &self.task_progress {
            if Some(*id) == foreground {
                continue;
            }
            count += 1;
            if let Some(u) = &t.update {
                done += u.total_done;
                total += u.total_total;
            }
        }
        (count > 0).then_some((done, total, count))
    }

    /// The menu-bar rect for the mini background-progress bar (left of the
    /// system-status widget), or `None` when nothing runs in the background.
    /// Shared by the renderer and mouse hit-testing so they stay in sync.
    pub(crate) fn menu_progress_rect(&self, menubar_row: Rect) -> Option<Rect> {
        self.background_summary()?;
        let mini_w = 24u16.min(menubar_row.width);
        let status_shown =
            self.config.system_status && menubar_row.width >= crate::ui::menubar::STATUS_MIN_WIDTH;
        let right_edge = if status_shown {
            menubar_row.x + menubar_row.width.saturating_sub(crate::ui::menubar::STATUS_WIDTH)
        } else {
            menubar_row.x + menubar_row.width
        };
        let x = right_edge.saturating_sub(mini_w).max(menubar_row.x);
        Some(Rect { x, y: menubar_row.y, width: mini_w, height: 1 })
    }

    // -- Event handling ----------------------------------------------------

    /// A clone of the app-event sender, handed to the console subshell's reader
    /// thread so it can nudge the loop to repaint the backdrop.
    pub(crate) fn event_sender(&self) -> AppSender {
        self.tx.clone()
    }

    pub async fn apply_event(&mut self, ev: AppEvent) {
        match ev {
            // Console output already landed in the shared emulator; receiving the
            // event is enough to trigger the next repaint (loop top redraws).
            AppEvent::ConsoleOutput => {}
            AppEvent::FsActivity => self.drain_fs_inbox(),
            AppEvent::Thumbnail { side, key, thumb } => {
                if let Some(cache) = self.panels.get_mut(side).and_then(|p| p.thumbs.as_mut()) {
                    cache.finish(key, thumb);
                }
            }
            AppEvent::Progress(u) => {
                // Fold the speed sample into the *task's* history first. It records
                // whether or not anyone is watching, so a transfer sent to the
                // background keeps its chart growing and the dialog that picks it
                // up later shows the whole run — rather than the history dying with
                // the dismissed dialog.
                if let Some(t) = self.task_progress.get_mut(&u.id) {
                    t.chart.record(u.total_done);
                }
                if let Some(Dialog::Progress(p)) = &mut self.dialog
                    && p.id == u.id
                {
                    p.update(&u);
                    // Draw the task's history. (A modal op — find / checksum /
                    // archive — has no task entry and keeps its own in `update`.)
                    if let Some(t) = self.task_progress.get(&u.id) {
                        p.chart.clone_from(&t.chart);
                    }
                }
                // Keep the background snapshot current even with no visible dialog
                // (drives the menu-bar mini bar and the Background-operations list).
                if let Some(t) = self.task_progress.get_mut(&u.id) {
                    t.update = Some(u);
                }
                // Advance the open "Background operations" list live.
                self.refresh_background_ops();
            }
            AppEvent::Conflict(info) => {
                // The engine is paused awaiting a decision. Bring the conflicting
                // transfer to the foreground (rebuild its progress dialog from the
                // latest snapshot) and raise the overwrite prompt over it; the
                // Overwrite reply restores the stashed progress dialog. This works
                // whether the task was foreground or in the background.
                self.stashed_progress = Some(self.progress_dialog_for(info.id));
                self.dialog = Some(Dialog::Overwrite(OverwriteDialog::new(info)));
            }
            AppEvent::PermissionDenied(info) => {
                // Same shape as a conflict: the engine is paused, so foreground
                // the task's progress dialog and raise the prompt over it. The
                // answer restores the stashed progress dialog.
                self.stashed_progress = Some(self.progress_dialog_for(info.id));
                self.dialog = Some(Dialog::Confirm(ConfirmDialog::permission_denied(&info)));
            }
            AppEvent::ArchiveAddChecked { conflicts, request } => {
                // Drop the "checking…" spinner; whatever comes next replaces it.
                if matches!(self.dialog, Some(Dialog::Busy(_))) {
                    self.dialog = None;
                }
                match conflicts {
                    Err(e) => self.show_error(format!("Cannot read the archive: {e}")),
                    Ok(names) if names.is_empty() => self.run_archive_add(*request),
                    Ok(names) => {
                        self.dialog = Some(Dialog::Confirm(ConfirmDialog::archive_overwrite(
                            &names, request,
                        )));
                    }
                }
            }
            AppEvent::TaskDone { id, outcome } => {
                self.tasks.remove(&id);
                self.task_progress.remove(&id);
                if let Some(Dialog::Progress(p)) = &self.dialog
                    && p.id == id
                {
                    self.dialog = None;
                }
                // Drop the finished task from an open "Background operations" list.
                self.refresh_background_ops();
                if let TaskOutcome::Failed(msg) = outcome {
                    self.dialog = Some(Dialog::Message(MessageDialog::error(msg)));
                }
                // Drop only the selection that was just operated on: the marked
                // set on the panel the op's sources came from. A selection sitting
                // on the *other* (inactive) panel is unrelated to this op and is
                // left alone, along with its cursor (the reload keeps it in place).
                if let Some(src) = self.op_source.remove(&id)
                    && let Some(p) = self.panels.get_mut(src)
                {
                    p.selection.clear();
                }
                self.reload_all().await;
                // Land the cursor on a remembered entry: the file after a delete,
                // or the just-renamed/moved item on its destination panel.
                if let Some((idx, name)) = self.pending_focus.take()
                    && let Some(p) = self.panels.get_mut(idx)
                    && let Some(i) = p.entries.iter().position(|e| e.name == name)
                {
                    p.cursor = i;
                }
            }
            AppEvent::PrivilegedDone { ok_msg, result } => {
                // Dismiss the busy spinner, then report on the manager's status.
                if matches!(self.dialog, Some(Dialog::Busy(_))) {
                    self.dialog = None;
                }
                self.finish_privileged(result, ok_msg);
            }
            AppEvent::FlashDone { id, outcome } => {
                self.flash_tasks.remove(&id);
                self.stashed_progress = None;
                // Report the outcome (this replaces the progress / abort dialog).
                match outcome {
                    TaskOutcome::Done => {
                        self.show_info("Flash complete", "The image was written successfully.")
                    }
                    TaskOutcome::Cancelled => self.show_info(
                        "Flash aborted",
                        "Flashing was aborted; the device is only partially written.",
                    ),
                    TaskOutcome::Failed(e) => self.show_error(format!("Flashing failed: {e}")),
                }
                // The target's contents changed — refresh the disk manager.
                if let Some(mv) = self.mountview.as_mut() {
                    mv.refresh();
                }
            }
            AppEvent::ImageDone { id, outcome } => {
                self.flash_tasks.remove(&id);
                self.stashed_progress = None;
                match outcome {
                    TaskOutcome::Done => self
                        .show_info("Image created", "The device image was written successfully."),
                    TaskOutcome::Cancelled => self.show_info(
                        "Imaging aborted",
                        "Imaging was aborted; the partial image file was removed.",
                    ),
                    TaskOutcome::Failed(e) => self.show_error(format!("Imaging failed: {e}")),
                }
            }
            AppEvent::ChecksumDone { id, result } => {
                self.tasks.remove(&id);
                if let Some(Dialog::Progress(p)) = &self.dialog
                    && p.id == id
                {
                    self.dialog = None;
                }
                match result {
                    Ok(report) => {
                        self.dialog =
                            Some(Dialog::ChecksumResult(ChecksumResultDialog::new(report)));
                    }
                    Err(Some(msg)) => self.show_error(msg),
                    Err(None) => {} // aborted: the progress dialog was closed above
                }
            }
            AppEvent::PanelizeDone { result } => self.on_panelize_done(result),
            AppEvent::GitStashes { stashes } => self.on_git_stashes(stashes),
            AppEvent::FindDone { id, results } => {
                self.tasks.remove(&id);
                if let Some(Dialog::Progress(p)) = &self.dialog
                    && p.id == id
                {
                    self.dialog = None;
                }
                self.panelize_results(results);
            }
            AppEvent::DuplicatesFound { id, left, right } => {
                self.tasks.remove(&id);
                if let Some(Dialog::Progress(p)) = &self.dialog
                    && p.id == id
                {
                    self.dialog = None;
                }
                self.mark_duplicates(left, right);
            }
            AppEvent::DetailsTally { viewer, generation, total, files, dirs, done } => {
                self.apply_details_tally(viewer, generation, total, files, dirs, done);
            }
            AppEvent::GitStatusScanned { side, generation, status } => {
                self.apply_git_status(side, generation, status.map(|b| *b));
            }
            AppEvent::BlameLoaded { generation, result } => {
                self.apply_blame(generation, result);
            }
            AppEvent::GeoJsonRead { generation, doc } => self.apply_geojson(generation, *doc),
            AppEvent::RenameMetaRead { generation, meta } => {
                self.apply_rename_meta(generation, meta);
            }
            AppEvent::TimelineTree { oid, generation, result } => {
                self.apply_timeline_tree(oid, generation, result);
            }
            AppEvent::DetailsPreview { viewer, generation, preview } => {
                self.apply_details_preview(viewer, generation, *preview);
            }
            AppEvent::DetailsActivity { viewer, key, activity } => {
                self.apply_details_activity(viewer, key, activity);
            }
            AppEvent::NetworkScanned { generation, result } => {
                self.apply_network_scanned(generation, result);
            }
            AppEvent::ReverseDnsResolved { ip, host } => {
                self.apply_reverse_dns(ip, host);
            }
            AppEvent::SendPrepared { name, result } => {
                self.on_send_prepared(name, result);
            }
            AppEvent::ReceiveProgress { name, received, total } => {
                if let Some(Dialog::Receive(d)) = &mut self.dialog {
                    d.on_progress(name, received, total);
                }
            }
            AppEvent::FileReceived { name, bytes } => self.on_file_received(name, bytes).await,
            AppEvent::ReceiveFailed { name, error } => {
                if let Some(Dialog::Receive(d)) = &mut self.dialog {
                    d.on_failed(name, error);
                }
            }
            AppEvent::FileSent => {
                self.on_file_sent();
            }
            AppEvent::SyncPlanned { result } => {
                self.on_sync_planned(result.map(|p| *p));
            }
            AppEvent::GitDone { title, out } => {
                self.on_git_done(title, out).await;
            }
            AppEvent::GitInfo { form, info } => {
                self.on_git_info(form, *info);
            }
            AppEvent::FileFetched { id, kind, name, orig_path, temp } => {
                self.tasks.remove(&id);
                if let Some(Dialog::Progress(p)) = &self.dialog
                    && p.id == id
                {
                    self.dialog = None;
                }
                match kind {
                    FetchKind::View => {
                        // Page the downloaded copy from disk; it's deleted on close.
                        let dark = self.dark_ui();
                        let t = temp.clone();
                        let scanned =
                            tokio::task::spawn_blocking(move || crate::viewer::scan_file(&t)).await;
                        match scanned {
                            Ok(Ok((file, len, line_starts, scanned))) => {
                                let mut v = ViewerState::from_scanned(
                                    name,
                                    file,
                                    len,
                                    line_starts,
                                    scanned,
                                    Some(temp.clone()),
                                );
                                v.enable_syntax(dark);
                                v.set_search_seed(self.search_memory.viewer_query.clone());
                                // Show a supported image fullscreen (from the
                                // fetched temp copy); falls back to raw on failure.
                                if crate::util::img::is_image_name(&v.name)
                                    && let Some(iv) = load_view_image(&temp).await
                                {
                                    v.set_image(iv);
                                }
                                // An executable from an archive or a remote
                                // host opens in Binary mode, as a local one does.
                                open_binary(&mut v, &temp).await;
                                open_certs(&mut v, &temp).await;
                                // So does a document, read from the temp copy
                                // under its original name.
                                self.attach_doc(&mut v, &temp).await;
                                // Audio from an archive or a remote host is
                                // drawn and played from the temp copy.
                                if let Some(av) = load_view_audio(
                                    &temp,
                                    &v.name,
                                    &self.config,
                                    self.audio_out.clone(),
                                )
                                .await
                                {
                                    v.set_audio(av);
                                }
                                self.viewer = Some(v);
                            }
                            Ok(Err(e)) => {
                                let _ = std::fs::remove_file(&temp);
                                self.show_error(format!("Cannot open file: {e}"));
                            }
                            Err(_) => {
                                let _ = std::fs::remove_file(&temp);
                                self.show_error("Viewer failed to open file");
                            }
                        }
                    }
                    FetchKind::Edit => {
                        // The editor edits in memory; read the temp then drop it.
                        // Saving still targets the original (remote) path.
                        match std::fs::read(&temp) {
                            Ok(bytes) => {
                                let text = String::from_utf8_lossy(&bytes).into_owned();
                                let mut ed = EditorState::new(name, orig_path, &text);
                                self.prepare_editor(&mut ed);
                                // No-op for remote paths (keys aren't stable), but
                                // keeps every editor-open site uniform.
                                Self::restore_editor_position(&mut ed);
                                self.editor = Some(ed);
                            }
                            Err(e) => self.show_error(format!("Cannot open file: {e}")),
                        }
                        let _ = std::fs::remove_file(&temp);
                    }
                }
            }
        }
    }

    // -- Key handling ------------------------------------------------------

    pub(in crate::app::state) fn show_error(&mut self, msg: impl Into<String>) {
        self.dialog = Some(Dialog::Message(MessageDialog::error(msg)));
    }

    /// Put `text` on the system clipboard, reporting only a refusal.
    ///
    /// Success is deliberately silent: OSC 52 gets no reply, so we cannot tell a
    /// terminal that took the text from one that ignored the sequence, and a
    /// modal dialog on every copy would be worse than no dialog at all.
    pub(in crate::app::state) fn copy_to_system_clipboard(&mut self, text: &str) {
        use crate::util::clipboard::{self, ClipError};
        if let Err(ClipError::TooLarge(n)) = clipboard::copy(text) {
            self.show_error(format!(
                "Too much text for the terminal clipboard: {} bytes, the limit is {}",
                n,
                clipboard::MAX_CLIP_BYTES
            ));
        }
    }

    /// The text [`Self::copy_paths_to_clipboard`] would put on the clipboard, or
    /// empty when there is nothing to copy. Split out so the shapes can be tested
    /// without writing an escape sequence to the terminal.
    pub(in crate::app::state) fn clipboard_text(
        &self,
        what: crate::ui::menu::ClipTarget,
    ) -> String {
        use crate::ui::menu::ClipTarget;
        // `operation_targets` resolves marks, the cursor entry and find-file
        // panelization the same way every file operation does, so what gets
        // copied is exactly what F5 would act on.
        let targets = self.panels[self.active].operation_targets();
        match what {
            ClipTarget::Selection => {
                targets.iter().map(|t| t.display()).collect::<Vec<_>>().join("\n")
            }
            ClipTarget::FullPath => targets.first().map(|t| t.display()).unwrap_or_default(),
            ClipTarget::Name => targets.first().map(|t| t.file_name()).unwrap_or_default(),
        }
    }

    /// Copy the active panel's paths to the system clipboard (Alt-C, the File
    /// menu, and the command palette).
    pub(in crate::app::state) fn copy_paths_to_clipboard(
        &mut self,
        what: crate::ui::menu::ClipTarget,
    ) {
        let text = self.clipboard_text(what);
        if text.is_empty() {
            // Nothing under the cursor but `..`, or an empty listing.
            return self.show_error("Nothing to copy");
        }
        self.copy_to_system_clipboard(&text);
    }

    pub(in crate::app::state) fn show_info(&mut self, title: &str, msg: impl Into<String>) {
        self.dialog = Some(Dialog::Message(MessageDialog {
            title: title.to_string(),
            message: msg.into(),
            is_error: false,
        }));
    }

    /// If this instance is nested inside another Rat Commander's subshell, raise
    /// the warning dialog. Called from startup *after* the UI language is loaded
    /// so the (construction-time translated) dialog is in the right language.
    pub fn warn_nested_subshell(&mut self) {
        if self.subshell_disabled {
            self.dialog = Some(Dialog::Confirm(ConfirmDialog::subshell_nested()));
        }
    }

    /// Quit, prompting for confirmation only when `confirm_exit` is enabled.
    pub(in crate::app::state) fn request_quit(&mut self) -> Flow {
        if self.config.confirm_exit {
            self.dialog = Some(Dialog::Confirm(ConfirmDialog::quit()));
            Flow::Continue
        } else {
            Flow::Quit
        }
    }
}
