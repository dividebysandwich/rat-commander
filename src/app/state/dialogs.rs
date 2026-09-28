//! Dialog result/submit handling and the dialog openers.

use super::*;

impl AppState {
    /// Apply the settings form's currently-highlighted theme/language choice as a
    /// live preview (called after every settings key/click/scroll). The final
    /// values are committed on submit and reverted on cancel.
    pub(in crate::app::state) fn preview_settings_choices(&mut self) {
        if let Some(Dialog::Form(fd)) = &self.dialog
            && let Some(name) = fd.theme_choice()
            && name != self.theme.name
        {
            self.theme = Theme::by_name(name, self.truecolor);
        }
        if let Some(Dialog::Form(fd)) = &self.dialog
            && let Some(name) = fd.lang_choice()
            && crate::l10n::active_name() != name
        {
            crate::l10n::set_active_by_name(name);
        }
        if let Some(Dialog::Form(fd)) = &self.dialog
            && let Some(on) = fd.check_value("Reshape RTL text")
            && on != crate::l10n::reshape_rtl_enabled()
        {
            crate::l10n::set_reshape_rtl(on);
        }
        if let Some(Dialog::Form(fd)) = &self.dialog
            && let Some(pref) = fd.graphics_choice()
            && let Some(g) = self.gfx.as_mut()
        {
            g.apply_pref(&pref);
        }
        // The 3D style previews through the config itself: `update_space3d`
        // pushes it into the panels on the next loop iteration, and that runs
        // while a dialog is up.
        if let Some(Dialog::Form(fd)) = &self.dialog
            && let Some(style) = fd.space3d_choice()
            && style != self.config.space3d_style
        {
            self.config.space3d_style = style;
        }
    }

    pub(in crate::app::state) async fn handle_dialog_result(&mut self, res: DialogResult) -> Flow {
        // The GeoJSON map edits the editor's text as it goes, whatever the
        // key or click that did it went on to do.
        self.apply_geo_edits();
        // Settings reopens on the tab it was closed on, whether by OK or Cancel.
        if !matches!(res, DialogResult::None)
            && let Some(Dialog::Form(fd)) = &self.dialog
            && let Some(tab) = fd.settings_tab()
        {
            self.settings_tab = tab;
        }
        match res {
            DialogResult::None => Flow::Continue,
            DialogResult::Cancel => {
                // Closing the Send-file dialog stops its HTTP server and removes
                // any temporary archive it was serving.
                if matches!(self.dialog, Some(Dialog::SendFile(_))) {
                    self.stop_send_server();
                }
                // Closing the Receive dialog stops its server, aborting (and
                // cleaning up after) any upload still in flight.
                if matches!(self.dialog, Some(Dialog::Receive(_))) {
                    self.stop_receive_server();
                }
                // A cancellable Busy spinner (git network op / sync planning)
                // aborts the task it was waiting on.
                if matches!(self.dialog, Some(Dialog::Busy(_)))
                    && let Some(h) = self.busy_task.take()
                {
                    h.abort();
                }
                self.dialog = None;
                // Abandon a user-menu command whose `%{…}` prompt was cancelled.
                self.pending_menu = None;
                // Abandon a connect whose key-passphrase prompt was cancelled,
                // so it can't be re-fired later by an unrelated password dialog.
                self.pending_connect = None;
                // Revert a live theme/language preview when Settings is cancelled.
                if let Some(name) = self.theme_backup.take() {
                    self.theme = Theme::by_name(&name, self.truecolor);
                }
                if let Some(name) = self.lang_backup.take() {
                    crate::l10n::set_active_by_name(&name);
                }
                if let Some(on) = self.reshape_backup.take() {
                    crate::l10n::set_reshape_rtl(on);
                }
                if let Some(pref) = self.graphics_backup.take()
                    && let Some(g) = self.gfx.as_mut()
                {
                    g.apply_pref(&pref);
                }
                if let Some(style) = self.space3d_backup.take() {
                    self.config.space3d_style = style;
                }
                Flow::Continue
            }
            DialogResult::Submit(s) => {
                self.dialog = None;
                self.theme_backup = None; // keep any previewed theme
                self.lang_backup = None; // keep any previewed language
                self.reshape_backup = None; // keep any previewed reshape toggle
                self.graphics_backup = None; // keep any previewed graphics mode
                self.space3d_backup = None; // keep any previewed 3D style
                // The command palette can run any action (including ones that
                // return their own Flow, like View → external viewer or Quit), so
                // it is dispatched here rather than through `handle_submit`.
                if let Submit::Palette(action) = s {
                    return self.run_palette_action(action).await;
                }
                // Likewise Shift-F4: opening the named file can hand over to an
                // external editor, which is a Flow of its own.
                if let Submit::EditNewFile(name) = s {
                    return self.open_new_file_editor(name).await;
                }
                self.handle_submit(s).await;
                if self.pending_quit {
                    Flow::Quit
                } else if let Some(cmd) = self.pending_run_fg.take() {
                    Flow::RunCommandForeground(cmd)
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
            DialogResult::Background(id) => {
                // Dismiss the progress dialog but keep the transfer running; it
                // lives on in `tasks`/`task_progress` and shows in the mini bar.
                if let Some(Dialog::Progress(p)) = &self.dialog
                    && p.id == id
                {
                    self.dialog = None;
                }
                // Remote (FTP) transfers: open a fresh browsing connection so the
                // panel isn't blocked sharing the transfer's single connection.
                self.background_reconnect_ftp(id).await;
                Flow::Continue
            }
            DialogResult::Overwrite(id, decision) => {
                // Send the decision back to the paused engine, then restore the
                // operation's progress dialog. (On Abort, TaskDone will close it.)
                if let Some(h) = self.tasks.get(&id) {
                    let _ = h.reply.try_send(TaskReply::Overwrite(decision));
                }
                self.dialog = self.stashed_progress.take().map(Dialog::Progress);
                Flow::Continue
            }
        }
    }

    pub(in crate::app::state) async fn handle_submit(&mut self, submit: Submit) {
        match submit {
            Submit::MkDir(name) => {
                let active = self.active;
                let path = self.panels[active].cwd.join(&name);
                let backend = self.panels[active].backend.clone();
                match backend.mkdir(&path).await {
                    Ok(()) => {
                        let _ = self.panels[active].reload_keeping(Some(&name)).await;
                        // Mirror the new directory into the other panel when it is
                        // showing the same location, keeping its own cursor.
                        let other = self.other_index();
                        if self.panels[other].cwd == self.panels[active].cwd {
                            let _ = self.panels[other].reload_keeping(None).await;
                        }
                    }
                    Err(e) => self.show_error(format!("Could not create directory: {e}")),
                }
            }
            Submit::Copy(sources, dest) => self.begin_transfer(OpKind::Copy, sources, &dest).await,
            Submit::Move(sources, dest) => self.begin_transfer(OpKind::Move, sources, &dest).await,
            Submit::AddTagKey(key, label) => {
                if let Some(ed) = self.editor.as_mut() {
                    ed.add_tag_key(key, label);
                }
            }
            Submit::MultiRename(plan) => self.do_multi_rename(plan).await,
            Submit::Delete(targets) => {
                if targets.iter().any(|t| t.is_native_archive()) {
                    self.start_archive_remove(targets);
                } else {
                    self.start_op(OpKind::Delete, targets, None, None, None);
                }
            }
            Submit::PrivilegeAnswer(id, decision) => {
                // Escalating needs sudo's credential cache primed first, and
                // that may mean asking for a password — so stash the answer and
                // come back to it once we have one.
                let wants_root =
                    matches!(decision, PrivDecision::Escalate | PrivDecision::EscalateAll);
                if wants_root && !crate::priv_ops::available().await {
                    self.pending_priv_answer = Some((id, decision));
                    self.dialog = Some(Dialog::Input(InputDialog::password(
                        "Authentication required",
                        "Enter sudo password:",
                        InputPurpose::EscalatePassword,
                    )));
                } else {
                    self.answer_privilege(id, decision);
                }
            }
            Submit::EscalatePassword(password) => {
                // Validate once, which primes sudo's own timestamp cache; every
                // later escalation runs `sudo -n` off it, so the password is
                // used here and then dropped.
                let pending = self.pending_priv_answer.take();
                match crate::mount::sudo_validate(&password).await {
                    Ok(()) => {
                        if let Some((id, decision)) = pending {
                            self.answer_privilege(id, decision);
                        }
                    }
                    Err(e) => {
                        // Tell the paused engine to skip rather than leaving it
                        // waiting forever on an answer that never comes.
                        if let Some((id, _)) = pending {
                            self.answer_privilege(id, PrivDecision::Skip);
                        }
                        self.show_error(format!("Authentication failed: {e}"));
                    }
                }
            }
            Submit::SelectTab(side, index) => self.tab_select(side, index).await,
            Submit::Trash(targets) => {
                self.start_op(OpKind::Trash, targets, None, None, None);
            }
            Submit::Compress(sources, name) => self.start_compress(sources, name),
            Submit::ArchiveAdd(req) => self.run_archive_add(*req),
            Submit::GotoDir(path) => self.goto_dir(*path).await,
            Submit::OpenSync => self.open_sync(),
            Submit::SyncPlan(mode) => self.start_sync_plan(mode),
            Submit::SyncRun(plan) => self.start_sync(*plan),
            // Every guided Git dialog lands here with a ready-built argv.
            Submit::GitRun { title, args } => match self.git_run_dir() {
                Some(dir) => self.spawn_git(title, dir, args),
                None => self.show_error("Git actions need a local directory"),
            },
            Submit::ForegroundTask(id) => {
                // Re-open the progress dialog for a backgrounded transfer.
                if self.tasks.contains_key(&id) {
                    self.dialog = Some(Dialog::Progress(self.progress_dialog_for(id)));
                }
            }
            Submit::Checksum { path, kind, expected } => self.start_checksum(path, kind, expected),
            Submit::Connect(side, creds) => self.connect_remote(side, creds).await,
            Submit::UserCommand(tpl) => {
                let labels = super::keys::menu_prompts(&tpl);
                if labels.is_empty() {
                    self.queue_menu_command(self.expand_macros(&tpl));
                    if super::keys::menu_uses_untag(&tpl) {
                        self.active_panel().selection.clear();
                    }
                } else {
                    // The command has `%{…}` prompts: ask for each in turn, then
                    // run once the last answer is in (see `advance_menu_prompt`).
                    self.pending_menu =
                        Some(PendingMenu { template: tpl, labels, answers: Vec::new() });
                    self.open_menu_prompt();
                }
            }
            Submit::MenuPrompt(answer) => self.advance_menu_prompt(answer),
            Submit::KillProcess { pid, force } => self.kill_process(pid, force),
            Submit::DeleteDiskFile(path) => self.delete_disk_file(path),
            Submit::CompareDirs(mode) => self.compare_dirs(mode).await,
            Submit::FindDuplicates(crit) => self.start_find_duplicates(crit),
            Submit::Quit => self.pending_quit = true,
            Submit::EditorSaveQuit => self.save_editor(true).await,
            Submit::EditorSave => self.save_editor(false).await,
            Submit::EditorGotoOffset(byte) => {
                if let Some(ed) = self.editor.as_mut() {
                    ed.goto_byte(byte);
                }
            }
            Submit::EditorSaveAs(dest) => self.do_save_as(dest).await,
            Submit::EditorBrowsed(kind, path) => self.editor_browsed(kind, path).await,
            Submit::EditorGotoLine(text) => {
                match text.trim().parse::<usize>() {
                    // The prompt is 1-based, the buffer 0-based.
                    Ok(n) => {
                        if let Some(ed) = self.editor.as_mut() {
                            ed.goto_line(n.saturating_sub(1));
                        }
                    }
                    Err(_) => self.show_error(format!("Not a line number: {text}")),
                }
            }
            Submit::EditorPasteOutput(cmd) => self.editor_paste_output(cmd).await,
            Submit::TrustCertificate { host_port, sha256 } => {
                let pinned = crate::vfs::remote::tls::pins_file()
                    .ok_or_else(|| std::io::Error::other("no configuration directory"))
                    .and_then(|path| crate::vfs::remote::tls::add_pin(&path, &host_port, &sha256));
                match (pinned, self.pending_connect.take()) {
                    (Err(e), _) => self.show_error(format!("Cannot save the certificate: {e}")),
                    (Ok(()), Some((side, creds))) => self.connect_remote(side, creds).await,
                    (Ok(()), None) => {}
                }
            }
            Submit::HexDiffGoto(text) => match crate::bt::interp::edit::parse_int(&text) {
                Some(off) if off >= 0 => {
                    if let Some(hd) = self.hexdiff.as_mut() {
                        hd.goto(off.min(u64::MAX as i128) as u64);
                    }
                }
                _ => self.show_error(format!("Not an offset: {text}")),
            },
            Submit::EditorSort { reverse, ignore_case, unique } => {
                if let Some(ed) = self.editor.as_mut() {
                    ed.sort_block(reverse, ignore_case, unique);
                }
            }
            Submit::EditorOptions(opts) => self.apply_editor_options(*opts),
            Submit::DiffSave => self.save_diff().await,
            Submit::DiffSaveQuit => {
                self.save_diff().await;
                self.diffview = None;
            }
            Submit::DiffDiscardQuit => self.diffview = None,
            Submit::EditorDiscardQuit => self.close_editor().await,
            Submit::EditorTemplate(info) => {
                if let Some(ed) = self.editor.as_mut() {
                    ed.set_template(info.map(|i| *i));
                }
            }
            Submit::EditorEditTemplate(info) => match info.path.clone() {
                Some(path) => self.open_template_editor(path, None),
                None => self.show_error(
                    "This template is built in: there is no template directory to edit it in",
                ),
            },
            Submit::EditorNewTemplate(name) => self.create_template(name),
            Submit::Select { select, pattern, files_only, case_sensitive, shell } => {
                self.apply_select(select, &pattern, files_only, case_sensitive, shell)
            }
            Submit::SearchReplace(p) => self.apply_search_replace(p),
            Submit::DiscardHunk => self.discard_hunk_confirmed().await,
            Submit::ConfirmDropStash { label, args } => {
                self.dialog = Some(Dialog::Confirm(ConfirmDialog::git_drop_stash(&label, args)));
            }
            Submit::Find(p) => self.start_find(p),
            Submit::Panelize(cmd) => self.start_panelize(cmd),
            Submit::Chmod(paths, mode, recursive) => self.apply_chmod(paths, mode, recursive).await,
            Submit::Chown(paths, owner, group, recursive) => {
                self.apply_chown(paths, &owner, &group, recursive).await
            }
            Submit::Symlink { dir, target, name } => {
                // The symlink is created in `dir` (the destination panel), so use
                // that location's backend.
                match self.registry.resolve(&dir) {
                    Ok(backend) => {
                        let link = dir.join(&name);
                        match backend.symlink(&target, &link).await {
                            Ok(()) => self.reload_all().await,
                            Err(e) => self.show_error(format!("Could not create symlink: {e}")),
                        }
                    }
                    Err(e) => self.show_error(format!("Cannot create symlink: {e}")),
                }
            }
            Submit::Settings(v) => {
                let cfg = &mut self.config;
                // Appearance
                cfg.theme = v.theme;
                cfg.animation = v.animation;
                cfg.nerd_font = v.nerd_font;
                cfg.system_status = v.system_status;
                cfg.screensaver_minutes = v.screensaver_minutes;
                cfg.screensaver = v.screensaver;
                // Panels. The watches follow `auto_refresh` and the 3D view's
                // activity switch, so re-arm them now rather than on the next
                // directory change, as the palette toggles do.
                let rewatch = cfg.auto_refresh != v.auto_refresh
                    || cfg.space3d_activity != v.space3d_activity;
                cfg.brief_columns = v.brief_columns;
                cfg.thumb_size = v.thumb_size;
                cfg.space3d_style = v.space3d_style;
                cfg.audio_display = v.audio_display;
                cfg.audio_autoplay = v.audio_autoplay;
                cfg.auto_refresh = v.auto_refresh;
                cfg.space3d_activity = v.space3d_activity;
                cfg.details_activity = v.details_activity;
                // Programs. The shell is process-wide state, so it is only
                // touched when it actually changed.
                cfg.editor = v.editor;
                cfg.viewer = v.viewer;
                cfg.use_internal_viewer = v.use_internal_viewer;
                cfg.use_internal_editor = v.use_internal_editor;
                if cfg.shell != v.shell {
                    crate::shell::set_preferred(&v.shell);
                    cfg.shell = v.shell;
                }
                if let Some(max) = v.command_history_max {
                    cfg.command_history_max = max;
                    self.cmd.set_history_max(max);
                }
                // Confirmations
                cfg.confirm_delete = v.confirm_delete;
                cfg.confirm_overwrite = v.confirm_overwrite;
                cfg.confirm_execute = v.confirm_execute;
                cfg.confirm_unmount = v.confirm_unmount;
                cfg.confirm_exit = v.confirm_exit;
                cfg.use_trash = v.use_trash;
                // Language (store English as the default => None).
                crate::l10n::set_active_by_name(&v.language);
                cfg.language = if v.language == "English" { None } else { Some(v.language) };
                cfg.reshape_rtl = v.reshape_rtl;
                crate::l10n::set_reshape_rtl(v.reshape_rtl);
                // Terminal. Rows already on screen keep their old ending until
                // they are redrawn, and the diffing renderer wouldn't redraw
                // them, so a changed trailing-space mode clears the screen.
                if cfg.strip_trailing_spaces != v.strip_trailing_spaces {
                    cfg.strip_trailing_spaces = v.strip_trailing_spaces;
                    self.force_clear = true;
                }
                cfg.graphics = v.graphics;
                cfg.truecolor = Some(v.truecolor);
                self.truecolor = v.truecolor;
                if let Some(g) = self.gfx.as_mut() {
                    g.apply_pref(&self.config.graphics);
                }
                self.set_command_prompt(v.command_prompt);
                if rewatch {
                    self.update_watches();
                }
                // Re-theme the running UI immediately.
                self.theme = Theme::by_name(&self.config.theme, self.truecolor);
                if let Err(e) = self.config.save() {
                    self.show_error(format!("Could not save settings: {e}"));
                }
            }
            Submit::OpenWith(path) => {
                tokio::spawn(async move { launch_default(path).await });
            }
            Submit::RunProgram(path) => {
                // Queue the executable to run in the foreground once the dialog
                // closes (handle_dialog_result turns this into Flow::RunCommand).
                self.pending_run = Some(run_program_cmd(&path));
            }
            Submit::RecallCommand(cmd) => {
                // Copy the chosen history entry into the command line, ready to
                // edit or run — never executed automatically.
                self.cmd.set(cmd);
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
            Submit::KeyPassphrase(passphrase) => {
                // Retry the connect the passphrase prompt interrupted. The creds
                // are taken, so a second prompt can't re-fire off a stale one.
                if let Some((side, mut creds)) = self.pending_connect.take() {
                    creds.key_passphrase = passphrase;
                    self.connect_remote(side, creds).await;
                }
            }
            Submit::NetworkPassword(password) => self.open_network(password),
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
            Submit::GoLocal(side) => self.go_local(side).await,
            Submit::GoVolume(side, path) => self.go_volume(side, path).await,
            Submit::SwitchSession(side, id) => self.switch_to_session(side, id).await,
            Submit::AskDisconnectSession(id) => self.ask_disconnect_session(id),
            Submit::DisconnectSession(id) => self.disconnect_session(id).await,
            Submit::Hotlist(outcome) => self.apply_hotlist_outcome(outcome).await,
            Submit::PanelFilter { side, pattern } => self.apply_panel_filter(side, pattern).await,
            // Palette actions and Shift-F4 are dispatched in `handle_dialog_result`
            // (which can return their Flow), so they never reach here.
            Submit::Palette(_) | Submit::EditNewFile(_) => {}
        }
    }

    /// Queue an expanded user-menu command to run after the dialog closes. On a
    /// local panel it runs in a suspended foreground shell (MC-style, so its
    /// output is visible); on a remote/archive panel it goes to the
    /// behind-the-panels console (which, on SFTP/SCP, runs it on the remote host).
    fn queue_menu_command(&mut self, cmd: String) {
        if self.console_cwd().scheme == "file" {
            self.pending_run_fg = Some(cmd);
        } else {
            self.pending_run = Some(cmd);
        }
    }

    /// Open an input dialog for the next unanswered `%{…}` prompt of the pending
    /// user-menu command, if any.
    fn open_menu_prompt(&mut self) {
        let label =
            self.pending_menu.as_ref().and_then(|pm| pm.labels.get(pm.answers.len()).cloned());
        if let Some(label) = label {
            self.dialog = Some(Dialog::Input(InputDialog::new(
                "User menu",
                label,
                "",
                InputPurpose::MenuPrompt,
            )));
        }
    }

    /// Record one `%{…}` prompt answer. If more prompts remain, open the next;
    /// otherwise substitute every answer into the template and queue it to run.
    fn advance_menu_prompt(&mut self, answer: String) {
        let (answered, total) = match self.pending_menu.as_mut() {
            Some(pm) => {
                pm.answers.push(answer);
                (pm.answers.len(), pm.labels.len())
            }
            None => return, // a stray prompt with no pending command — ignore
        };
        if answered < total {
            self.open_menu_prompt();
            return;
        }
        let pm = self.pending_menu.take().expect("pending menu present");
        let cmd = self.expand_macros_with(&pm.template, &pm.answers);
        self.queue_menu_command(cmd);
        if super::keys::menu_uses_untag(&pm.template) {
            self.active_panel().selection.clear();
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
            p.selection.select_group(&p.entries, pattern, files_only, case_sensitive, shell)
        } else {
            p.selection.unselect_group(&p.entries, pattern, case_sensitive, shell)
        };
        if let Err(e) = res {
            self.show_error(format!("Invalid pattern: {e}"));
        }
    }

    /// Apply `mode` to every target, recursing into directories when asked.
    async fn apply_chmod(&mut self, paths: Vec<VfsPath>, mode: u32, recursive: bool) {
        let backend = self.panels[self.active].backend.clone();
        let mut errors = Vec::new();
        for root in &paths {
            for t in collect_tree(&backend, root, recursive).await {
                if let Err(e) = backend.set_permissions(&t, mode).await {
                    errors.push(format!("{}: {e}", t.file_name()));
                }
            }
        }
        let _ = self.panels[self.active].reload().await;
        self.report_op_errors("chmod", errors);
    }

    /// Apply ownership to every target, recursing into directories when asked.
    async fn apply_chown(
        &mut self,
        paths: Vec<VfsPath>,
        owner: &str,
        group: &str,
        recursive: bool,
    ) {
        let uid = match resolve_uid(owner) {
            Ok(u) => u,
            Err(e) => return self.show_error(e),
        };
        let gid = match resolve_gid(group) {
            Ok(g) => g,
            Err(e) => return self.show_error(e),
        };
        let backend = self.panels[self.active].backend.clone();
        let mut errors = Vec::new();
        for root in &paths {
            for t in collect_tree(&backend, root, recursive).await {
                if let Err(e) = backend.set_owner(&t, uid, gid).await {
                    errors.push(format!("{}: {e}", t.file_name()));
                }
            }
        }
        let _ = self.panels[self.active].reload().await;
        self.report_op_errors("chown", errors);
    }

    /// Show a summary error dialog when an op failed on some files (no-op on
    /// full success).
    fn report_op_errors(&mut self, op: &str, errors: Vec<String>) {
        if errors.is_empty() {
            return;
        }
        let shown: Vec<String> = errors.iter().take(8).cloned().collect();
        let more = errors.len().saturating_sub(shown.len());
        let mut msg = format!("{op} failed for {} item(s):\n{}", errors.len(), shown.join("\n"));
        if more > 0 {
            msg.push_str(&format!("\n… and {more} more"));
        }
        self.show_error(msg);
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
        // Nor is a backend that cannot be written to. Asked here rather than
        // discovered at `open_write`, so a copy into git history or an ISO is
        // refused before any bytes have been read.
        if !self.panels[self.other_index()].backend.capabilities().writable {
            self.show_error(crate::l10n::tr("That panel's filesystem is read-only"));
            return;
        }
        // Prefill the destination panel's path. For a remote panel, show the
        // "scheme://path" form so the copy targets that backend; deleting the
        // "scheme://" prefix redirects the copy to a local path. An archive is
        // shown as "archive.zip!/dir" for the same reason: kept, the sources go
        // into the archive; replaced by a plain path, they go to the disk.
        let cwd = &self.panels[self.other_index()].cwd;
        let dest = if cwd.scheme == "file" {
            cwd.path.to_string_lossy().into_owned()
        } else if let Some(prefix) = container_dest_prefix(cwd) {
            format!("{prefix}{}", cwd.posix_path())
        } else {
            cwd.display()
        };
        let (title, purpose) = match kind {
            OpKind::Copy => ("Copy", InputPurpose::CopyDest(sources)),
            OpKind::Move => ("Move", InputPurpose::MoveDest(sources)),
            // Delete/Trash have their own confirm dialog and Sync its own
            // planner, so none of them reaches this destination prompt.
            OpKind::Delete | OpKind::Trash | OpKind::Sync => {
                unreachable!("no destination prompt")
            }
        };
        let prompt = format!("{title} to:");
        self.dialog = Some(Dialog::Input(InputDialog::new(title, prompt, dest, purpose)));
    }

    /// F8 (`permanent == false`) and Shift-F8 / Ctrl-F8 (`permanent == true`).
    ///
    /// Trashing only applies to real local files: there is nowhere to move a
    /// file to on an SFTP server or inside a `.zip`, so those keep deleting
    /// outright rather than pretending to be recoverable.
    pub(in crate::app::state) fn open_delete_dialog(&mut self, permanent: bool) {
        let targets = self.panels[self.active].operation_targets();
        if targets.is_empty() {
            return;
        }
        let use_trash = !permanent
            && self.config.use_trash
            && crate::trash::is_available()
            && targets.iter().all(|t| t.is_plain_local());
        if !self.config.confirm_delete {
            let kind = if use_trash { OpKind::Trash } else { OpKind::Delete };
            self.start_op(kind, targets, None, None, None);
            return;
        }
        self.dialog = Some(Dialog::Confirm(if use_trash {
            ConfirmDialog::trash(targets)
        } else if self.config.use_trash && crate::trash::is_available() {
            // The trash is on, so make clear that *this* one really is forever.
            ConfirmDialog::delete_permanently(targets)
        } else {
            ConfirmDialog::delete(targets)
        }));
    }

    pub(in crate::app::state) fn open_mkdir(&mut self) {
        self.dialog = Some(Dialog::Input(InputDialog::new(
            "Create directory",
            "Enter directory name:",
            "",
            InputPurpose::MkDir,
        )));
    }

    /// Shift-F4: ask for a file name, then open the editor on it in the active
    /// panel's directory (see [`AppState::open_new_file_editor`]).
    pub(in crate::app::state) fn open_edit_new_file(&mut self) {
        self.dialog = Some(Dialog::Input(InputDialog::new(
            "Edit new file",
            "File name",
            "",
            InputPurpose::EditNewFile,
        )));
    }

    pub(in crate::app::state) fn open_select_group(&mut self, select: bool) {
        self.dialog = Some(Dialog::Select(SelectDialog::new(select)));
    }

    pub(in crate::app::state) fn invert_selection(&mut self) {
        let p = &mut self.panels[self.active];
        let names: Vec<String> =
            p.entries.iter().filter(|e| e.name != "..").map(|e| e.name.clone()).collect();
        for n in names {
            p.selection.toggle(&n);
        }
    }

    pub(in crate::app::state) fn open_settings(&mut self) {
        self.open_settings_at(self.settings_tab);
    }

    /// Open Settings on `tab` (the palette's Confirmations entry opens its own tab).
    pub(in crate::app::state) fn open_settings_at(&mut self, tab: SettingsTab) {
        // Remember the current theme + language so Esc can revert a live preview.
        self.theme_backup = Some(self.config.theme.clone());
        self.lang_backup = Some(crate::l10n::active_name());
        self.reshape_backup = Some(crate::l10n::reshape_rtl_enabled());
        self.graphics_backup = Some(self.config.graphics.clone());
        self.space3d_backup = Some(self.config.space3d_style);
        self.dialog =
            Some(Dialog::Form(FormDialog::settings(&self.config, self.truecolor).on_tab(tab)));
    }

    pub(in crate::app::state) fn open_chmod(&mut self) {
        let p = &self.panels[self.active];
        if !p.backend.capabilities().permissions {
            return self.show_error("This filesystem does not support permissions");
        }
        let targets = p.operation_targets();
        if targets.is_empty() {
            return self.show_error("No files selected");
        }
        // Prefill the bits from the file under the cursor (a representative).
        let mode =
            p.current_entry().filter(|e| e.name != "..").and_then(|e| e.mode).unwrap_or(0o644)
                & 0o777;
        self.dialog = Some(Dialog::Form(FormDialog::chmod(targets, mode)));
    }

    pub(in crate::app::state) fn open_chown(&mut self) {
        let p = &self.panels[self.active];
        if !p.backend.capabilities().ownership {
            return self.show_error("This filesystem does not support ownership");
        }
        let targets = p.operation_targets();
        if targets.is_empty() {
            return self.show_error("No files selected");
        }
        // Prefill owner/group from the file under the cursor (a representative).
        let cur = p.current_entry().filter(|e| e.name != "..");
        let owner = cur
            .and_then(|e| e.uid)
            .map(|u| uid_name(u).unwrap_or_else(|| u.to_string()))
            .unwrap_or_default();
        let group = cur
            .and_then(|e| e.gid)
            .map(|g| gid_name(g).unwrap_or_else(|| g.to_string()))
            .unwrap_or_default();
        self.dialog = Some(Dialog::Form(FormDialog::chown(targets, owner, group)));
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
            Some(e) if e.name != ".." => {
                (active.cwd.join(&e.name).path.to_string_lossy().into_owned(), e.name.clone())
            }
            _ => (String::new(), String::new()),
        };
        self.dialog = Some(Dialog::Form(FormDialog::symlink(dir, target, name)));
    }

    /// Open the checksum options dialog for the file under the cursor. Works on
    /// any backend (local, remote, or inside an archive) since the file is read
    /// through the VFS; only a real file (not `..` or a directory) is accepted.
    pub(in crate::app::state) fn open_checksum(&mut self) {
        let p = &self.panels[self.active];
        let Some(e) = p.current_entry() else {
            return;
        };
        if e.name == ".." || e.kind != VfsKind::File {
            return self.show_error("Select a file to checksum");
        }
        let path = p.cwd.join(&e.name);
        self.dialog = Some(Dialog::Form(FormDialog::checksum(path)));
    }

    // -- Archives ----------------------------------------------------------

    /// Decide what pressing Enter on the (non-directory) file under the cursor
    /// does: a local **executable** with no registered MIME handler (ELF
    /// binaries, shell scripts, …) is run directly in the foreground; anything
    /// else is opened with its default application.
    pub(in crate::app::state) async fn open_or_execute_under_cursor(&mut self) -> Flow {
        let p = &self.panels[self.active];
        // Only local files can be executed / opened by path.
        if p.cwd.scheme != "file" {
            return Flow::Continue;
        }
        let Some(e) = p.current_entry() else {
            return Flow::Continue;
        };
        if e.kind != VfsKind::File {
            return Flow::Continue;
        }
        let name = e.name.clone();
        let executable = e.is_executable();
        let path = p.cwd.path.join(&e.name);

        // Run an executable directly only when the desktop has no handler for it
        // (so e.g. a chmod +x .pdf still opens in its viewer). On non-Linux the
        // MIME query isn't available, so fall through to the default-app path.
        #[cfg(target_os = "linux")]
        let run_directly = executable && !has_mime_handler(&path).await;
        #[cfg(not(target_os = "linux"))]
        let run_directly = {
            let _ = executable;
            false
        };

        if run_directly {
            // Honor "confirm execute": ask first, then run in the foreground.
            if self.config.confirm_execute {
                self.dialog = Some(Dialog::Confirm(ConfirmDialog::run_program(&name, path)));
                return Flow::Continue;
            }
            return Flow::RunCommand(run_program_cmd(&path));
        }
        self.open_with_default();
        Flow::Continue
    }

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

/// Every path an op should touch: `root`, plus — when `recursive` — all
/// descendants if `root` is a real directory. Symlinks are never followed, so
/// the walk can't loop or escape the selected tree. Directories are listed
/// before the op mutates anything, so a permission change that removes search
/// access can't cut the traversal short.
async fn collect_tree(
    backend: &std::sync::Arc<dyn Vfs>,
    root: &VfsPath,
    recursive: bool,
) -> Vec<VfsPath> {
    let mut out = vec![root.clone()];
    if !recursive {
        return out;
    }
    let descend = backend
        .stat(root)
        .await
        .map(|e| e.kind.is_dir() && e.symlink_target.is_none())
        .unwrap_or(false);
    if !descend {
        return out;
    }
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = backend.read_dir(&dir).await else {
            continue;
        };
        for e in entries {
            if e.name == ".." || e.name == "." || e.symlink_target.is_some() {
                continue;
            }
            let child = dir.join(&e.name);
            if e.kind.is_dir() {
                stack.push(child.clone());
            }
            out.push(child);
        }
    }
    out
}
