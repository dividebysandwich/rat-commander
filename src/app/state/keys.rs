//! Key dispatch: menu/panel/global key routing and directory navigation.

use super::*;

impl AppState {
    /// Top-level key entry point. Implements Midnight-Commander-style Esc-prefix
    /// function-key aliases (Esc-1..Esc-9 => F1..F9, Esc-0 => F10) before
    /// dispatching to the active mode. The aliases are active in the base modes
    /// (panels, editor, viewer); dialogs and the pulldown menu keep Esc as an
    /// immediate cancel.
    pub async fn handle_key(&mut self, key: KeyEvent) -> Flow {
        // Alt arms the menu accelerators in panel mode: it lights up the bar's
        // hotkey letters, and Alt+<letter> opens that top menu directly. (Once a
        // menu is open it captures plain letters, so Alt is no longer needed.)
        if self.in_panel_mode() {
            self.alt_hint = key.modifiers.contains(KeyModifiers::ALT);
            if self.alt_hint
                && let KeyCode::Char(c) = key.code
                && let Some(idx) = menu_title_index(c)
            {
                self.menu = Some(MenuBarState::new(idx, &self.session_list()));
                self.alt_hint = false;
                return Flow::Continue;
            }
        } else {
            self.alt_hint = false;
        }

        let prefixable = self.dialog.is_none() && self.menu.is_none();
        if prefixable {
            if self.pending_esc.take().is_some() {
                // The previous key was a lone Esc; this key completes the
                // sequence. A digit becomes the matching function key.
                if let Some(n) = fkey_for_code(key.code) {
                    return self.route_key(synth_fkey(n)).await;
                }
                // Otherwise deliver the held Esc, then this key normally.
                let _ = self.route_key(esc_key()).await;
                return self.route_key(key).await;
            }
            if key.code == KeyCode::Esc && key.modifiers.is_empty() {
                // Hold the Esc; the next key (or a tick timeout) resolves it.
                self.pending_esc = Some(Instant::now());
                return Flow::Continue;
            }
            // Fast path: terminals send Esc+digit pressed together as Alt+digit.
            if key.modifiers.contains(KeyModifiers::ALT)
                && let Some(n) = fkey_for_code(key.code)
            {
                return self.route_key(synth_fkey(n)).await;
            }
        }
        self.route_key(key).await
    }

    /// Whether the base two-panel view has focus (no dialog, menu, or
    /// full-screen overlay is up) — i.e. the menu bar is visible and idle.
    fn in_panel_mode(&self) -> bool {
        self.dialog.is_none()
            && self.menu.is_none()
            && self.editor.is_none()
            && self.viewer.is_none()
            && self.procview.is_none()
            && self.diskview.is_none()
            && self.diffview.is_none()
            && self.mountview.is_none()
            && self.netview.is_none()
    }

    /// Deliver a held Esc once its function-key window has elapsed without a
    /// following key (called from the event loop's tick).
    pub async fn flush_expired_esc(&mut self) -> Flow {
        if let Some(t) = self.pending_esc
            && t.elapsed() >= ESC_PREFIX_TIMEOUT
        {
            self.pending_esc = None;
            return self.route_key(esc_key()).await;
        }
        Flow::Continue
    }

    async fn route_key(&mut self, key: KeyEvent) -> Flow {
        if self.dialog.is_some() {
            let res = self.dialog.as_mut().unwrap().handle_key(key);
            // Live theme/language preview from the settings form.
            self.preview_settings_choices();
            return self.handle_dialog_result(res).await;
        }
        if self.editor.is_some() {
            let signal = self.editor.as_mut().unwrap().handle_key(key);
            self.apply_editor_signal(signal).await;
            return Flow::Continue;
        }
        if self.viewer.is_some() {
            let sig = self.viewer.as_mut().unwrap().handle_key(key);
            self.apply_viewer_signal(sig);
            return Flow::Continue;
        }
        if let Some(pv) = self.procview.as_mut() {
            match pv.handle_key(key) {
                ProcSignal::Stay => {}
                ProcSignal::Close => self.procview = None,
                ProcSignal::Kill { pid, name, force } => {
                    self.dialog =
                        Some(Dialog::Confirm(ConfirmDialog::kill(pid, &name, force)));
                }
            }
            return Flow::Continue;
        }
        if self.diskview.is_some() {
            let sig = self.diskview.as_mut().unwrap().handle_key(key);
            self.apply_disk_signal(sig).await;
            return Flow::Continue;
        }
        if let Some(nv) = self.netview.as_mut() {
            match nv.handle_key(key) {
                NetSignal::Stay => {}
                NetSignal::Close => self.netview = None,
                NetSignal::Refresh => self.start_network_scan(),
            }
            return Flow::Continue;
        }
        if self.diffview.is_some() {
            match self.diffview.as_mut().unwrap().handle_key(key) {
                DiffSignal::Stay => {}
                DiffSignal::Close => self.diffview = None,
                DiffSignal::Save => {
                    self.dialog = Some(Dialog::Confirm(ConfirmDialog::save_diff()));
                }
                DiffSignal::ConfirmQuit => {
                    self.dialog = Some(Dialog::Confirm(ConfirmDialog::diff_quit()));
                }
            }
            return Flow::Continue;
        }
        if self.mountview.is_some() {
            let sig = self.mountview.as_mut().unwrap().handle_key(key);
            self.apply_mount_signal(sig).await;
            return Flow::Continue;
        }
        if self.menu.is_some() {
            return self.handle_menu_key(key).await;
        }
        self.handle_panel_key(key).await
    }

    async fn handle_menu_key(&mut self, key: KeyEvent) -> Flow {
        let signal = self.menu.as_mut().unwrap().handle_key(key);
        match signal {
            MenuSignal::Stay => Flow::Continue,
            MenuSignal::Close => {
                self.menu = None;
                Flow::Continue
            }
            MenuSignal::Activate(action) => {
                self.menu = None;
                self.run_menu_action(action).await
            }
        }
    }

    pub(in crate::app::state) async fn run_menu_action(&mut self, action: MenuAction) -> Flow {
        match action {
            MenuAction::Separator => {}
            MenuAction::View => return self.open_view().await,
            MenuAction::Edit => return self.open_edit().await,
            MenuAction::Copy => self.open_transfer_dialog(OpKind::Copy),
            MenuAction::Move => self.open_transfer_dialog(OpKind::Move),
            MenuAction::MultiRename => self.open_multi_rename(),
            MenuAction::Mkdir => self.open_mkdir(),
            MenuAction::Delete => self.open_delete_dialog(),
            MenuAction::Chmod => self.open_chmod(),
            MenuAction::Chown => self.open_chown(),
            MenuAction::Symlink => self.open_symlink(),
            MenuAction::Compress => self.open_compress(),
            MenuAction::Checksum => self.open_checksum(),
            MenuAction::SelectGroup => self.open_select_group(true),
            MenuAction::UnselectGroup => self.open_select_group(false),
            MenuAction::Invert => self.invert_selection(),
            MenuAction::SetFormat(side, fmt) => self.panels[side].format = fmt,
            MenuAction::SetSort(side, key) => {
                self.panels[side].sort.key = key;
                self.panels[side].resort();
            }
            MenuAction::ToggleReverse(side) => {
                self.panels[side].sort.reverse = !self.panels[side].sort.reverse;
                self.panels[side].resort();
            }
            MenuAction::SwapPanels => self.panels.swap(0, 1),
            MenuAction::Refresh => self.reload_all().await,
            MenuAction::ToggleSplit => self.split = self.split.toggle(),
            MenuAction::FindFile => self.open_find_dialog(),
            MenuAction::FindDuplicates => {
                self.dialog = Some(Dialog::Form(FormDialog::find_duplicates()))
            }
            MenuAction::ProcExplorer => self.open_proc_explorer(),
            MenuAction::DiskExplorer => self.open_disk_explorer(),
            MenuAction::DiskManager => self.mountview = Some(MountView::new()),
            MenuAction::NetworkConnections => self.open_network_prompt(),
            MenuAction::CompareDirs => self.dialog = Some(Dialog::Compare(CompareDialog::new())),
            MenuAction::CompareFiles => self.open_compare_files().await,
            MenuAction::Connect(side, proto) => {
                if self.other_panel_is_remote(side) {
                    self.show_error(
                        "The other panel is already remote — one panel must stay local.".to_string(),
                    );
                } else {
                    self.dialog = Some(Dialog::Form(FormDialog::connect(
                        proto,
                        side,
                        self.config.recent_remotes.clone(),
                    )));
                }
            }
            MenuAction::Disconnect(side) => self.go_local(side).await,
            MenuAction::SwitchSession(side, id) => self.switch_to_session(side, id).await,
            MenuAction::DisconnectSession(id) => self.ask_disconnect_session(id),
            MenuAction::Drive(side) => self.open_drive_dialog(side),
            MenuAction::Settings => self.open_settings(),
            MenuAction::Confirmations => self.open_confirmations(),
            MenuAction::EditThemes => self.open_edit_themes(),
            MenuAction::Quit => return self.request_quit(),
        }
        Flow::Continue
    }

    pub(in crate::app::state) async fn handle_panel_key(&mut self, key: KeyEvent) -> Flow {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Alt-F1 / Alt-F2 open the drive/connection picker for the left/right panel.
        if key.modifiers.contains(KeyModifiers::ALT) {
            match key.code {
                KeyCode::F(1) => {
                    self.open_drive_dialog(0);
                    return Flow::Continue;
                }
                KeyCode::F(2) => {
                    self.open_drive_dialog(1);
                    return Flow::Continue;
                }
                _ => {}
            }
        }

        match key.code {
            // -- Quit / function keys --
            KeyCode::F(10) => return self.request_quit(),
            KeyCode::Char('q') if ctrl => return Flow::Quit, // immediate fallback if F10 is intercepted
            KeyCode::F(1) => self.open_help(),
            KeyCode::F(2) => self.open_user_menu(),
            KeyCode::F(3) => return self.open_view().await,
            KeyCode::F(4) => return self.open_edit().await,
            KeyCode::F(5) => self.open_transfer_dialog(OpKind::Copy),
            // Shift-F6 / Ctrl-F6 open the multi-rename tool; plain F6 is move.
            KeyCode::F(6) if ctrl || key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.open_multi_rename()
            }
            KeyCode::F(6) => self.open_transfer_dialog(OpKind::Move),
            KeyCode::F(7) => self.open_mkdir(),
            KeyCode::F(8) => self.open_delete_dialog(),
            KeyCode::F(9) => self.open_menu(),

            // -- Panel navigation --
            KeyCode::Up => self.active_panel().move_cursor(-1),
            KeyCode::Down => self.active_panel().move_cursor(1),
            KeyCode::PageUp => {
                let p = self.active_panel();
                let step = p.page.max(1) as isize;
                p.move_cursor(-step);
            }
            KeyCode::PageDown => {
                let p = self.active_panel();
                let step = p.page.max(1) as isize;
                p.move_cursor(step);
            }
            KeyCode::Home => self.active_panel().move_home(),
            KeyCode::End => self.active_panel().move_end(),
            KeyCode::Insert => self.active_panel().toggle_mark_and_advance(),
            KeyCode::Tab => self.active = self.other_index(),

            // -- Enter: run command or descend --
            KeyCode::Enter => {
                if !self.cmd.is_empty() {
                    let cmd = self.cmd.take();
                    // A built-in `cd` changes the active panel instead of being
                    // run in a (throwaway) subshell where it would have no effect.
                    if let Some(arg) = parse_cd(&cmd) {
                        self.change_dir(arg).await;
                        return Flow::Continue;
                    }
                    return Flow::RunCommand(cmd);
                }
                self.enter_dir().await;
            }

            // -- Command-line editing --
            KeyCode::Left => self.cmd.move_left(),
            KeyCode::Right => self.cmd.move_right(),
            KeyCode::Backspace => self.cmd.backspace(),
            KeyCode::Delete => self.cmd.delete(),
            KeyCode::Esc => self.cmd.clear(),

            // -- View / sort / layout toggles (Ctrl chords) --
            KeyCode::Char('u') if ctrl => self.panels.swap(0, 1),
            KeyCode::Char('o') if ctrl => return Flow::SubShell,
            KeyCode::Char('r') if ctrl => {
                let _ = self.active_panel().reload().await;
            }
            KeyCode::Char('t') if ctrl => self.active_panel().toggle_mark_and_advance(),
            KeyCode::Char('x') if ctrl => self.split = self.split.toggle(),
            KeyCode::Char('w') if ctrl => {
                let p = self.active_panel();
                p.format = p.format.toggle();
            }
            KeyCode::Char('s') if ctrl => self.cycle_sort(),
            KeyCode::Char('e') if ctrl => {
                let p = self.active_panel();
                p.sort.reverse = !p.sort.reverse;
                p.resort();
            }

            // -- Selection by wildcard (only when the command line is empty) --
            KeyCode::Char('+') if self.cmd.is_empty() => self.open_select_group(true),
            KeyCode::Char('-') if self.cmd.is_empty() => self.open_select_group(false),
            KeyCode::Char('*') if self.cmd.is_empty() => self.invert_selection(),

            // -- Otherwise, type into the command line --
            KeyCode::Char(c) => self.cmd.insert(c),

            _ => {}
        }
        Flow::Continue
    }

    pub(in crate::app::state) async fn enter_dir(&mut self) {
        let p = &self.panels[self.active];
        // Directory / ".." navigation first, then "enter archive file".
        let target = p
            .target_dir_under_cursor()
            .or_else(|| archive_target_under_cursor(p));
        let Some((newcwd, focus)) = target else {
            // Not a directory/archive: an image opens the flasher (Linux); any
            // other file opens with its default app.
            #[cfg(target_os = "linux")]
            if self.try_flash_under_cursor() {
                return;
            }
            self.open_with_default();
            return;
        };
        // Re-resolve the backend: navigation may cross backends (local↔archive).
        let backend = match self.registry.resolve(&newcwd) {
            Ok(b) => b,
            Err(e) => return self.show_error(e.to_string()),
        };
        // Atomic move: if the target can't be listed (e.g. permission denied),
        // the panel stays where it is rather than getting stuck in it.
        self.active_panel()
            .try_enter(newcwd, backend, focus.as_deref())
            .await;
    }

    fn cycle_sort(&mut self) {
        let p = self.active_panel();
        let cur = p.sort.key;
        let idx = SortKey::ALL.iter().position(|k| *k == cur).unwrap_or(0);
        p.sort.key = SortKey::ALL[(idx + 1) % SortKey::ALL.len()];
        p.resort();
    }

    /// Handle a `cd` typed at the command line: change the active panel's
    /// directory. Supports `cd` / `cd ~` (home), `cd /abs`, `cd rel`, and `cd ..`.
    /// If the target can't be listed, the panel stays put (no blocking error).
    pub(in crate::app::state) async fn change_dir(&mut self, arg: &str) {
        let arg = arg.trim();
        let cur = self.panels[self.active].cwd.clone();

        let newcwd: VfsPath = if cur.scheme == "file" {
            let target: PathBuf = if arg.is_empty() || arg == "~" {
                home_dir()
            } else if let Some(rest) = arg.strip_prefix("~/") {
                home_dir().join(rest)
            } else {
                let raw = Path::new(arg);
                if raw.is_absolute() {
                    raw.to_path_buf()
                } else {
                    cur.path.join(raw)
                }
            };
            VfsPath::local(normalize_path(&target))
        } else {
            // Inside an archive/remote backend: support `..` and relative joins.
            match arg {
                "" | "~" => return,
                ".." => match cur.parent() {
                    Some(p) => p,
                    None => return,
                },
                _ => cur.join(arg),
            }
        };

        let backend = match self.registry.resolve(&newcwd) {
            Ok(b) => b,
            Err(e) => return self.show_error(e.to_string()),
        };
        self.active_panel().try_enter(newcwd, backend, None).await;
    }

    fn open_menu(&mut self) {
        // F9 opens the pulldown menu matching the active panel: Left (0)/Right (4).
        let active = if self.active == 0 { 0 } else { 4 };
        self.menu = Some(MenuBarState::new(active, &self.session_list()));
    }

    fn open_user_menu(&mut self) {
        if self.user_menu.is_empty() {
            return self.show_error("No user-menu entries (see the config 'menu' file)");
        }
        self.dialog = Some(Dialog::UserMenu(UserMenuDialog::new(self.user_menu.clone())));
    }

    /// Expand mc-style menu macros against the active panel.
    pub(in crate::app::state) fn expand_macros(&self, tpl: &str) -> String {
        use crate::vfs::remote::shell_quote;
        let p = &self.panels[self.active];
        let cwd = p.cwd.path.to_string_lossy().into_owned();
        let cur = p.current_entry().map(|e| e.name.clone()).unwrap_or_default();
        let marked: Vec<String> = p.selection.marked_names(&p.entries).iter().map(|n| shell_quote(n)).collect();
        let tagged = marked.join(" ");
        let selected = if marked.is_empty() {
            shell_quote(&cur)
        } else {
            tagged.clone()
        };

        // Scan for %X macros (%% → literal %).
        let mut out = String::with_capacity(tpl.len());
        let mut chars = tpl.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('%') => out.push('%'),
                Some('f') | Some('p') => out.push_str(&cur),
                Some('d') => out.push_str(&cwd),
                Some('t') => out.push_str(&tagged),
                Some('s') => out.push_str(&selected),
                Some(other) => {
                    out.push('%');
                    out.push(other);
                }
                None => out.push('%'),
            }
        }
        out
    }

    /// F1: show the help screen — the embedded user manual. The `.md` name makes
    /// the viewer open in Markdown render mode (markup hidden); F8 toggles to the
    /// raw source, so enable syntax highlighting for that.
    pub(in crate::app::state) fn open_help(&mut self) {
        let mut v = ViewerState::new(HELP_NAME.to_string(), HELP_TEXT.as_bytes().to_vec());
        v.enable_syntax(self.dark_ui());
        self.viewer = Some(v);
    }

}
