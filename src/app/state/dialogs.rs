//! Dialog result/submit handling and the dialog openers.

use super::*;

impl AppState {
    pub(in crate::app::state) async fn handle_dialog_result(&mut self, res: DialogResult) -> Flow {
        match res {
            DialogResult::None => Flow::Continue,
            DialogResult::Cancel => {
                self.dialog = None;
                // Revert a live theme preview when the settings dialog is cancelled.
                if let Some(name) = self.theme_backup.take() {
                    self.theme = Theme::by_name(&name, self.truecolor);
                }
                Flow::Continue
            }
            DialogResult::Submit(s) => {
                self.dialog = None;
                self.theme_backup = None; // keep any previewed theme
                self.handle_submit(s).await;
                if self.pending_quit {
                    Flow::Quit
                } else if let Some(cmd) = self.pending_run.take() {
                    Flow::RunCommand(cmd)
                } else {
                    Flow::Continue
                }
            }
            DialogResult::Abort(id) => {
                if self.flash_tasks.contains_key(&id) {
                    // Don't abort a flash outright — confirm first, stashing the
                    // progress view so Resume can restore it (the flash keeps
                    // running in the background meanwhile).
                    if let Some(Dialog::Progress(p)) = self.dialog.take() {
                        self.stashed_progress = Some(p);
                    }
                    self.dialog = Some(Dialog::Confirm(ConfirmDialog::abort_flash(id)));
                } else if let Some(h) = self.tasks.get(&id) {
                    h.cancel.cancel();
                }
                // Keep the progress dialog until TaskDone confirms cancellation.
                Flow::Continue
            }
            DialogResult::Overwrite(id, decision) => {
                // Send the decision back to the paused engine, then restore the
                // operation's progress dialog. (On Abort, TaskDone will close it.)
                if let Some(h) = self.tasks.get(&id) {
                    let _ = h.reply.try_send(decision);
                }
                self.dialog = self.stashed_progress.take().map(Dialog::Progress);
                Flow::Continue
            }
        }
    }

    pub(in crate::app::state) async fn handle_submit(&mut self, submit: Submit) {
        match submit {
            Submit::MkDir(name) => {
                let path = self.panels[self.active].cwd.join(&name);
                let backend = self.panels[self.active].backend.clone();
                match backend.mkdir(&path).await {
                    Ok(()) => {
                        let _ = self.panels[self.active].reload_keeping(Some(&name)).await;
                    }
                    Err(e) => self.show_error(format!("mkdir failed: {e}")),
                }
            }
            Submit::Copy(sources, dest) => self.begin_transfer(OpKind::Copy, sources, &dest).await,
            Submit::Move(sources, dest) => self.begin_transfer(OpKind::Move, sources, &dest).await,
            Submit::MultiRename(plan) => self.do_multi_rename(plan).await,
            Submit::Delete(targets) => {
                if targets.iter().any(|t| t.is_archive()) {
                    self.start_archive_remove(targets);
                } else {
                    self.start_op(OpKind::Delete, targets, None, None);
                }
            }
            Submit::Compress(sources, name) => self.start_compress(sources, name),
            Submit::Connect(side, creds) => self.connect_remote(side, creds).await,
            Submit::UserCommand(tpl) => self.pending_run = Some(self.expand_macros(&tpl)),
            Submit::KillProcess { pid, force } => self.kill_process(pid, force),
            Submit::CompareDirs(mode) => self.compare_dirs(mode).await,
            Submit::Quit => self.pending_quit = true,
            Submit::EditorSaveQuit => self.save_editor(true).await,
            Submit::EditorSave => self.save_editor(false).await,
            Submit::DiffSave => self.save_diff().await,
            Submit::DiffSaveQuit => {
                self.save_diff().await;
                self.diffview = None;
            }
            Submit::DiffDiscardQuit => self.diffview = None,
            Submit::EditorDiscardQuit => {
                self.editor = None;
                self.reload_all().await;
            }
            Submit::Select {
                select,
                pattern,
                files_only,
                case_sensitive,
                shell,
            } => self.apply_select(select, &pattern, files_only, case_sensitive, shell),
            Submit::SearchReplace(p) => self.apply_search_replace(p),
            Submit::Find(p) => self.start_find(p),
            Submit::Chmod(path, mode) => {
                let backend = self.panels[self.active].backend.clone();
                match backend.set_permissions(&path, mode).await {
                    Ok(()) => {
                        let _ = self.panels[self.active].reload().await;
                    }
                    Err(e) => self.show_error(format!("chmod failed: {e}")),
                }
            }
            Submit::Chown(path, owner, group) => self.apply_chown(path, &owner, &group).await,
            Submit::Symlink { dir, target, name } => {
                // The symlink is created in `dir` (the destination panel), so use
                // that location's backend.
                match self.registry.resolve(&dir) {
                    Ok(backend) => {
                        let link = dir.join(&name);
                        match backend.symlink(&target, &link).await {
                            Ok(()) => self.reload_all().await,
                            Err(e) => self.show_error(format!("symlink failed: {e}")),
                        }
                    }
                    Err(e) => self.show_error(e.to_string()),
                }
            }
            Submit::Settings(v) => {
                self.config.editor = v.editor;
                self.config.viewer = v.viewer;
                self.config.use_internal_viewer = v.use_internal_viewer;
                self.config.use_internal_editor = v.use_internal_editor;
                self.config.theme = v.theme;
                self.config.truecolor = Some(v.truecolor);
                self.config.animation = v.animation;
                self.config.system_status = v.system_status;
                self.truecolor = v.truecolor;
                // Re-theme the running UI immediately.
                self.theme = Theme::by_name(&self.config.theme, self.truecolor);
                if let Err(e) = self.config.save() {
                    self.show_error(format!("could not save settings: {e}"));
                }
            }
            Submit::Confirmations(v) => {
                self.config.confirm_delete = v.delete;
                self.config.confirm_overwrite = v.overwrite;
                self.config.confirm_execute = v.execute;
                self.config.confirm_unmount = v.unmount;
                self.config.confirm_exit = v.exit;
                if let Err(e) = self.config.save() {
                    self.show_error(format!("could not save settings: {e}"));
                }
            }
            Submit::OpenWith(path) => {
                tokio::spawn(async move { launch_default(path).await });
            }
            Submit::Mount { device, path } => {
                // Create the mount point first if it doesn't exist (with consent).
                if std::path::Path::new(&path).exists() {
                    self.do_mount(device, path, false).await;
                } else {
                    self.dialog =
                        Some(Dialog::Confirm(ConfirmDialog::create_mountpoint(&device, &path)));
                }
            }
            Submit::MountCreate { device, path } => self.do_mount(device, path, true).await,
            Submit::SudoPassword(password) => self.run_pending_sudo(password).await,
            Submit::MountDevice(device) => self.prompt_mount_path(device),
            Submit::FormatDevice(device) => {
                self.dialog = Some(Dialog::Form(FormDialog::format(device)));
            }
            Submit::AskUnmount(mountpoint) => self.ask_unmount(mountpoint).await,
            Submit::DoUnmount(mountpoint) => self.do_unmount(mountpoint).await,
            Submit::SyncPath(mountpoint) => self.do_sync(mountpoint).await,
            Submit::Format(spec) => {
                self.dialog = Some(Dialog::Confirm(ConfirmDialog::format(spec)));
            }
            Submit::DoFormat(spec) => self.do_format(spec).await,
            Submit::ViewerGoto(value, mode) => {
                if let Some(v) = self.viewer.as_mut()
                    && !v.goto(&value, mode)
                {
                    self.show_error(format!("Invalid {} value: {value}", goto_mode_label(mode)));
                }
            }
            // -- Image flashing --
            Submit::FlashSelected(spec) => {
                // A non-removable target gets an extra red warning first.
                self.dialog = Some(Dialog::Confirm(if spec.target.removable {
                    ConfirmDialog::flash_confirm(spec)
                } else {
                    ConfirmDialog::flash_danger(spec)
                }));
            }
            Submit::FlashConfirm(spec) => {
                self.dialog = Some(Dialog::Confirm(ConfirmDialog::flash_confirm(spec)));
            }
            Submit::DoFlash(spec) => self.start_flash(spec).await,
            Submit::FlashBrowse(target) => self.open_flash_browser(target),
            Submit::FlashBrowsePicked(path, target) => self.flash_picked_image(path, target),
            Submit::FlashPassword(pw) => {
                if let Some(spec) = self.pending_flash.take() {
                    self.begin_flash(spec, crate::flash::FlashAuth::SudoPassword(pw));
                }
            }
            Submit::FlashResume => {
                self.dialog = self.stashed_progress.take().map(Dialog::Progress);
            }
            Submit::FlashAbort(id) => {
                if let Some(c) = self.flash_tasks.get(&id) {
                    c.cancel();
                }
                self.stashed_progress = None;
                // The progress view stays closed; FlashDone will report the result.
            }
            // -- Create image (read a device out to a file) --
            Submit::ImageBrowse(target) => self.open_image_browser(target),
            Submit::ImageSave(spec) => {
                // Confirm before clobbering an existing file; else start straight away.
                if spec.dest_path.exists() {
                    self.dialog = Some(Dialog::Confirm(ConfirmDialog::image_overwrite(spec)));
                } else {
                    self.start_image(spec).await;
                }
            }
            Submit::DoImage(spec) => self.start_image(spec).await,
            Submit::ImagePassword(pw) => {
                if let Some(spec) = self.pending_image.take() {
                    self.begin_image(spec, crate::flash::FlashAuth::SudoPassword(pw));
                }
            }
            // -- Drive / connection picker --
            Submit::SetDrive(side, letter) => self.set_drive(side, letter).await,
            Submit::OpenConnect(side, proto) => {
                self.dialog = Some(Dialog::Form(FormDialog::connect(
                    proto,
                    side,
                    self.config.recent_remotes.clone(),
                )));
            }
            Submit::DisconnectPanel(side) => self.disconnect(side).await,
        }
    }

    fn apply_select(
        &mut self,
        select: bool,
        pattern: &str,
        files_only: bool,
        case_sensitive: bool,
        shell: bool,
    ) {
        let p = &mut self.panels[self.active];
        let res = if select {
            p.selection
                .select_group(&p.entries, pattern, files_only, case_sensitive, shell)
        } else {
            p.selection
                .unselect_group(&p.entries, pattern, case_sensitive, shell)
        };
        if let Err(e) = res {
            self.show_error(format!("invalid pattern: {e}"));
        }
    }

    async fn apply_chown(&mut self, path: VfsPath, owner: &str, group: &str) {
        let uid = match resolve_uid(owner) {
            Ok(u) => u,
            Err(e) => return self.show_error(e),
        };
        let gid = match resolve_gid(group) {
            Ok(g) => g,
            Err(e) => return self.show_error(e),
        };
        let backend = self.panels[self.active].backend.clone();
        match backend.set_owner(&path, uid, gid).await {
            Ok(()) => {
                let _ = self.panels[self.active].reload().await;
            }
            Err(e) => self.show_error(format!("chown failed: {e}")),
        }
    }

    pub(in crate::app::state) fn open_transfer_dialog(&mut self, kind: OpKind) {
        let sources = self.panels[self.active].operation_targets();
        if sources.is_empty() {
            return;
        }
        // A search-result panel is not a real destination directory.
        if self.panels[self.other_index()].is_panelized() {
            self.show_error("Cannot copy into a search-result panel");
            return;
        }
        // Destination is an archive → add into it (rebuild), not a file copy.
        if self.panels[self.other_index()].cwd.is_archive() {
            if self.panels[self.active].cwd.is_archive() {
                self.show_error("Cannot copy directly between archives; extract first");
                return;
            }
            let dest = self.panels[self.other_index()].cwd.clone();
            self.start_archive_add(kind, sources, dest);
            return;
        }
        // Prefill the destination panel's path. For a remote panel, show the
        // "scheme://path" form so the copy targets that backend; deleting the
        // "scheme://" prefix redirects the copy to a local path.
        let cwd = &self.panels[self.other_index()].cwd;
        let dest = if cwd.scheme == "file" {
            cwd.path.to_string_lossy().into_owned()
        } else {
            cwd.display()
        };
        let (title, purpose) = match kind {
            OpKind::Copy => ("Copy", InputPurpose::CopyDest(sources)),
            OpKind::Move => ("Move", InputPurpose::MoveDest(sources)),
            OpKind::Delete => unreachable!(),
        };
        let prompt = format!("{title} to:");
        self.dialog = Some(Dialog::Input(InputDialog::new(title, prompt, dest, purpose)));
    }

    pub(in crate::app::state) fn open_delete_dialog(&mut self) {
        let targets = self.panels[self.active].operation_targets();
        if targets.is_empty() {
            return;
        }
        if self.config.confirm_delete {
            self.dialog = Some(Dialog::Confirm(ConfirmDialog::delete(targets)));
        } else {
            self.start_op(OpKind::Delete, targets, None, None);
        }
    }

    pub(in crate::app::state) fn open_mkdir(&mut self) {
        self.dialog = Some(Dialog::Input(InputDialog::new(
            "Create directory",
            "Enter directory name:",
            "",
            InputPurpose::MkDir,
        )));
    }

    pub(in crate::app::state) fn open_select_group(&mut self, select: bool) {
        self.dialog = Some(Dialog::Select(SelectDialog::new(select)));
    }

    pub(in crate::app::state) fn invert_selection(&mut self) {
        let p = &mut self.panels[self.active];
        let names: Vec<String> = p
            .entries
            .iter()
            .filter(|e| e.name != "..")
            .map(|e| e.name.clone())
            .collect();
        for n in names {
            p.selection.toggle(&n);
        }
    }

    pub(in crate::app::state) fn open_settings(&mut self) {
        // Remember the current theme so Esc can revert a live preview.
        self.theme_backup = Some(self.config.theme.clone());
        self.dialog = Some(Dialog::Form(FormDialog::settings(&self.config, self.truecolor)));
    }

    pub(in crate::app::state) fn open_confirmations(&mut self) {
        self.dialog = Some(Dialog::Form(FormDialog::confirmations(&self.config)));
    }

    pub(in crate::app::state) fn open_chmod(&mut self) {
        let p = &self.panels[self.active];
        if !p.backend.capabilities().permissions {
            return self.show_error("This filesystem does not support permissions");
        }
        let Some(e) = p.current_entry() else {
            return self.show_error("No file under cursor");
        };
        if e.name == ".." {
            return self.show_error("No file under cursor");
        }
        let path = p.cwd.join(&e.name);
        let mode = e.mode.unwrap_or(0o644) & 0o777;
        self.dialog = Some(Dialog::Form(FormDialog::chmod(path, mode)));
    }

    pub(in crate::app::state) fn open_chown(&mut self) {
        let p = &self.panels[self.active];
        if !p.backend.capabilities().ownership {
            return self.show_error("This filesystem does not support ownership");
        }
        let Some(e) = p.current_entry() else {
            return self.show_error("No file under cursor");
        };
        if e.name == ".." {
            return self.show_error("No file under cursor");
        }
        let path = p.cwd.join(&e.name);
        let owner = e
            .uid
            .and_then(uid_name)
            .unwrap_or_else(|| e.uid.map(|u| u.to_string()).unwrap_or_default());
        let group = e
            .gid
            .and_then(gid_name)
            .unwrap_or_else(|| e.gid.map(|g| g.to_string()).unwrap_or_default());
        self.dialog = Some(Dialog::Form(FormDialog::chown(path, owner, group)));
    }

    pub(in crate::app::state) fn open_symlink(&mut self) {
        // The link is created in the *other* panel, pointing at the active
        // panel's file under the cursor (both prefilled, editable).
        let other = self.other_index();
        if !self.panels[other].backend.capabilities().symlinks {
            return self.show_error("This filesystem does not support symlinks");
        }
        let dir = self.panels[other].cwd.clone();
        let active = &self.panels[self.active];
        let (target, name) = match active.current_entry() {
            Some(e) if e.name != ".." => (
                active.cwd.join(&e.name).path.to_string_lossy().into_owned(),
                e.name.clone(),
            ),
            _ => (String::new(), String::new()),
        };
        self.dialog = Some(Dialog::Form(FormDialog::symlink(dir, target, name)));
    }

    // -- Archives ----------------------------------------------------------

    /// Open the local file under the cursor with the system default program
    /// (xdg-open), but only if a MIME handler is actually defined for it. Runs
    /// detached so the TUI keeps running.
    pub(in crate::app::state) fn open_with_default(&mut self) {
        let p = &self.panels[self.active];
        if p.cwd.scheme != "file" {
            return;
        }
        let Some(e) = p.current_entry() else {
            return;
        };
        if e.kind != VfsKind::File {
            return;
        }
        let name = e.name.clone();
        let path = p.cwd.path.join(&e.name);
        // When "confirm execute" is on, ask before launching the default app.
        if self.config.confirm_execute {
            self.dialog = Some(Dialog::Confirm(ConfirmDialog::execute(&name, path)));
        } else {
            tokio::spawn(async move { launch_default(path).await });
        }
    }

}
