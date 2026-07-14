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
            active: 0,
            split: SplitDir::Vertical,
            panel_hidden: [false, false],
            half_height: false,
            // Sized to a sane default; resized to the backdrop area on each draw.
            console: crate::console::Console::new(24, 80),
            cmd,
            dialog: None,
            viewer: None,
            editor: None,
            menu: None,
            procview: None,
            diskview: None,
            diffview: None,
            mountview: None,
            netview: None,
            theme_editor: None,
            pending_sudo: None,
            pending_flash: None,
            pending_image: None,
            flash_tasks: HashMap::new(),
            theme,
            config,
            registry,
            sessions: Vec::new(),
            last_local_cwd: [cwd.clone(), cwd],
            tasks: HashMap::new(),
            task_progress: HashMap::new(),
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
            graphics_backup: None,
            user_menu: usermenu::load_or_create(),
            ext_rules: crate::ext::ExtRules::load_or_create(),
            pending_run: None,
            pending_quit: false,
            alt_hint: false,
            pending_esc: None,
            quick_search: None,
            stashed_progress: None,
            last_area: Rect::new(0, 0, 0, 0),
            paint_last: None,
            last_click: None,
            details: Default::default(),
            git_key: [String::new(), String::new()],
            git_gen: [0, 0],
            pending_focus: None,
            search_memory: Default::default(),
            edit_only: false,
            kbd_enhanced: false,
            subshell_disabled,
        }
    }

    /// Copy each panel's current listing format and sort order into the config
    /// and persist it, so they are restored on the next run. Called on exit.
    pub fn persist_panel_views(&mut self) {
        for i in 0..2 {
            self.config.panels[i] = crate::config::PanelView {
                format: self.panels[i].format,
                sort: self.panels[i].sort,
            };
        }
        let _ = self.config.save();
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
            || self.diskview.as_ref().is_some_and(|d| d.scanning)
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
            let _ = p.reload().await;
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
        let status_shown = self.config.system_status
            && menubar_row.width >= crate::ui::menubar::STATUS_MIN_WIDTH;
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
            AppEvent::Progress(u) => {
                if let Some(Dialog::Progress(p)) = &mut self.dialog
                    && p.id == u.id
                {
                    p.update(&u);
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
                // Drop selections that were just operated on, then refresh.
                for p in self.panels.iter_mut() {
                    p.selection.clear();
                }
                self.reload_all().await;
                // Land the cursor on a remembered entry: the file above a delete,
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
                    TaskOutcome::Done => {
                        self.show_info("Image created", "The device image was written successfully.")
                    }
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
            AppEvent::DiskScanProgress { generation, done, total } => {
                if let Some(dv) = self.diskview.as_mut()
                    && dv.generation == generation
                    && dv.scanning
                {
                    dv.scan_done = done;
                    dv.scan_total = total;
                }
            }
            AppEvent::NetworkScanned { generation, result } => {
                self.apply_network_scanned(generation, result);
            }
            AppEvent::ReverseDnsResolved { ip, host } => {
                self.apply_reverse_dns(ip, host);
            }
            AppEvent::DiskScanned { generation, entries } => {
                if let Some(dv) = self.diskview.as_mut()
                    && dv.generation == generation
                {
                    dv.entries = entries;
                    dv.scanning = false;
                    dv.selected = 0;
                }
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
                                self.viewer = Some(v);
                            }
                            Ok(Err(e)) => {
                                let _ = std::fs::remove_file(&temp);
                                self.show_error(format!("cannot open file: {e}"));
                            }
                            Err(_) => {
                                let _ = std::fs::remove_file(&temp);
                                self.show_error("viewer failed to open file");
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
                                ed.enable_syntax(self.dark_ui());
                                // No-op for remote paths (keys aren't stable), but
                                // keeps every editor-open site uniform.
                                Self::restore_editor_position(&mut ed);
                                self.editor = Some(ed);
                            }
                            Err(e) => self.show_error(format!("cannot open file: {e}")),
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
