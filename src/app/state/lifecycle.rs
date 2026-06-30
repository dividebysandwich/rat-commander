//! Construction, the tick/event-loop plumbing, and small panel helpers.

use super::*;

impl AppState {
    pub fn new(tx: AppSender) -> Self {
        let registry = Registry::new();
        let local = registry.local();
        let cwd = VfsPath::local_cwd();
        let left = Panel::new(local.clone(), cwd.clone());
        let right = Panel::new(local, cwd);
        let config = Config::load();
        let truecolor = config.truecolor.unwrap_or_else(detect_truecolor);
        let theme = Theme::by_name(&config.theme, truecolor);
        AppState {
            panels: [left, right],
            active: 0,
            split: SplitDir::Vertical,
            cmd: CommandLine::new(),
            dialog: None,
            viewer: None,
            editor: None,
            menu: None,
            procview: None,
            diskview: None,
            diffview: None,
            mountview: None,
            pending_sudo: None,
            pending_flash: None,
            pending_image: None,
            flash_tasks: HashMap::new(),
            theme,
            config,
            registry,
            tasks: HashMap::new(),
            next_task_id: 1,
            next_session_id: 0,
            tx,
            truecolor,
            anim_phase: 0,
            tick_count: 0,
            sampler: crate::util::sysinfo::SysSampler::new(),
            theme_backup: None,
            user_menu: usermenu::load_or_create(),
            pending_run: None,
            pending_quit: false,
            pending_esc: None,
            alt_hint: false,
            stashed_progress: None,
            last_area: Rect::new(0, 0, 0, 0),
            paint_last: None,
            pending_focus: None,
        }
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
        dirty
    }

    /// Whether the loop needs periodic ticks at all (animation or stats on).
    pub fn wants_ticks(&self) -> bool {
        (self.config.animation && self.truecolor)
            || self.config.system_status
            || self.pending_esc.is_some()
            || self.procview.is_some()
            || self.mountview.is_some()
            || !self.tasks.is_empty()
            || matches!(self.dialog, Some(Dialog::Busy(_)))
            || self.diskview.as_ref().is_some_and(|d| d.scanning)
    }

    /// Load both panels' directories.
    pub async fn init(&mut self) {
        let _ = self.panels[0].reload().await;
        let _ = self.panels[1].reload().await;
    }

    pub(in crate::app::state) fn active_panel(&mut self) -> &mut Panel {
        &mut self.panels[self.active]
    }

    pub(in crate::app::state) fn other_index(&self) -> usize {
        1 - self.active
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
    }

    // -- Event handling ----------------------------------------------------

    pub async fn apply_event(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Progress(u) => {
                if let Some(Dialog::Progress(p)) = &mut self.dialog
                    && p.id == u.id
                {
                    p.update(&u);
                }
            }
            AppEvent::Conflict(info) => {
                // The engine is paused awaiting a decision. Stash the progress
                // dialog and raise the overwrite prompt over it.
                if let Some(Dialog::Progress(p)) = self.dialog.take() {
                    self.stashed_progress = Some(p);
                }
                self.dialog = Some(Dialog::Overwrite(OverwriteDialog::new(info)));
            }
            AppEvent::TaskDone { id, outcome } => {
                self.tasks.remove(&id);
                if let Some(Dialog::Progress(p)) = &self.dialog
                    && p.id == id
                {
                    self.dialog = None;
                }
                if let TaskOutcome::Failed(msg) = outcome {
                    self.dialog = Some(Dialog::Message(MessageDialog::error(msg)));
                }
                // Drop selections that were just operated on, then refresh.
                for p in self.panels.iter_mut() {
                    p.selection.clear();
                }
                self.reload_all().await;
                // After a delete, drop the cursor onto the file above the
                // deleted one rather than letting it snap to the top.
                if let Some(name) = self.pending_focus.take() {
                    let p = &mut self.panels[self.active];
                    if let Some(i) = p.entries.iter().position(|e| e.name == name) {
                        p.cursor = i;
                    }
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
            AppEvent::FindDone { id, results } => {
                self.tasks.remove(&id);
                if let Some(Dialog::Progress(p)) = &self.dialog
                    && p.id == id
                {
                    self.dialog = None;
                }
                self.panelize_results(results);
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

    fn show_info(&mut self, title: &str, msg: impl Into<String>) {
        self.dialog = Some(Dialog::Message(MessageDialog {
            title: title.to_string(),
            message: msg.into(),
            is_error: false,
        }));
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
