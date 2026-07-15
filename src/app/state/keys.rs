//! Key dispatch: menu/panel/global key routing and directory navigation.

use super::*;

impl AppState {
    /// Top-level key entry point. Implements Midnight-Commander-style Esc-prefix
    /// function-key aliases (Esc-1..Esc-9 => F1..F9, Esc-0 => F10) before
    /// dispatching to the active mode. The aliases are active in the base modes
    /// (panels, editor, viewer); dialogs and the pulldown menu keep Esc as an
    /// immediate cancel.
    pub async fn handle_key(&mut self, key: KeyEvent) -> Flow {
        // An active quick search captures every key (including Alt+letter, so a
        // held-Alt sequence like Alt+H+I+G extends the query rather than each
        // Alt+letter restarting it). Esc/Enter/other keys exit it via its handler.
        if self.quick_search.is_some() && self.in_panel_mode() {
            return self.handle_quick_search_key(key).await;
        }

        // Esc-prefix function-key aliases (Esc-1..Esc-9 => F1..F9, Esc-0 => F10)
        // are active in the base modes (panels, editor, viewer); dialogs and the
        // pulldown menu keep Esc as an immediate cancel.
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

        // In the base panel view: Alt-S / Ctrl-S start a quick search with an
        // empty box (Midnight-Commander style); the letters typed afterward build
        // the query. Alt + a menu letter opens that top menu, and Alt alone arms
        // the menu-bar hotkey hint (cleared by the next non-Alt key). Alt+F1/F2
        // drive pickers are F-key codes handled in handle_panel_key, not here.
        if self.in_panel_mode() {
            let alt = key.modifiers.contains(KeyModifiers::ALT);
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            // `alt != ctrl` = exactly one held, so AltGr (Ctrl+Alt, a composing
            // key on some layouts) still types normally.
            if (alt != ctrl) && matches!(key.code, KeyCode::Char('s' | 'S')) {
                self.alt_hint = false;
                self.start_quick_search_empty();
                return Flow::Continue;
            }
            self.alt_hint = alt;
            if alt
                && let KeyCode::Char(c) = key.code
                && let Some(idx) = menu_title_index(c)
                // Alt-F is word-forward while editing a non-empty command line;
                // it only opens the File menu when the line is empty.
                && (c != 'f' || self.cmd.is_empty())
                // Alt-O belongs to "show this directory on the other panel"; the
                // Options menu keeps Alt-Shift-O (and F9), since `menu_title_index`
                // lower-cases and so still matches the shifted letter.
                && c != 'o'
            {
                self.menu = Some(MenuBarState::new(idx, &self.session_list(), self.side_remote()));
                self.alt_hint = false;
                return Flow::Continue;
            }
        } else {
            self.alt_hint = false;
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
            && self.theme_editor.is_none()
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
            // Remember a committed search app-wide (prefills future prompts).
            if let Some(v) = self.viewer.as_ref() {
                self.search_memory.viewer_query = v.search_seed().to_string();
            }
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
                NetSignal::Kill { pid, program, force } => {
                    self.dialog = Some(Dialog::Confirm(ConfirmDialog::kill(pid, &program, force)));
                }
                NetSignal::ResolveDns(ip) => self.start_reverse_dns(ip),
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
        if self.theme_editor.is_some() {
            let sig = self.theme_editor.as_mut().unwrap().handle_key(key);
            self.apply_theme_editor_signal(sig);
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
            MenuAction::SendFile => self.send_file(),
            // The Git submenu's parent never acts on its own — opening it is
            // handled inside the menu bar.
            MenuAction::GitMenu => {}
            MenuAction::GitStatus
            | MenuAction::GitLog
            | MenuAction::GitDiff
            | MenuAction::GitStage
            | MenuAction::GitAdd
            | MenuAction::GitUnstage
            | MenuAction::GitRemove
            | MenuAction::GitRestore
            | MenuAction::GitCommit
            | MenuAction::GitFetch
            | MenuAction::GitPull
            | MenuAction::GitPush
            | MenuAction::GitSync
            | MenuAction::GitCheckout
            | MenuAction::GitReset
            | MenuAction::GitInit
            | MenuAction::GitClone => self.git_action(action).await,
            MenuAction::BackgroundOps => self.open_background_ops(),
            MenuAction::SelectGroup => self.open_select_group(true),
            MenuAction::UnselectGroup => self.open_select_group(false),
            MenuAction::Invert => self.invert_selection(),
            MenuAction::SetFormat(side, fmt) => self.set_format(side, fmt).await,
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
            MenuAction::SyncDirs => self.open_sync(),
            MenuAction::CompareFiles => self.open_compare_files().await,
            MenuAction::CommandPalette => self.open_command_palette(),
            MenuAction::Hotlist => self.open_hotlist(),
            MenuAction::PanelFilter => self.open_panel_filter(),
            MenuAction::DirHistory => self.open_dir_history(),
            MenuAction::SyncPanels => self.sync_other_panel().await,
            MenuAction::ChdirOther => self.chdir_other_panel().await,
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
            MenuAction::EditExtensions => self.open_edit_extensions(),
            MenuAction::EditUserMenu => self.open_edit_user_menu(),
            MenuAction::Quit => return self.request_quit(),
        }
        Flow::Continue
    }

    pub(in crate::app::state) async fn handle_panel_key(&mut self, key: KeyEvent) -> Flow {
        // An active quick search captures all keys until Esc/Enter/exit.
        if self.quick_search.is_some() {
            return self.handle_quick_search_key(key).await;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);

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

        // Emacs/readline editing of the command line (the same set every dialog
        // input honours). C-E, C-W and Alt-F only edit when the line has text;
        // when empty they keep their panel meaning (reverse sort / cycle view /
        // File menu), handled below and in `handle_key`.
        if cmdline_edit_wanted(key, self.cmd.is_empty()) {
            self.cmd.apply_readline(key);
            return Flow::Continue;
        }

        match key.code {
            // -- Panel visibility (Norton-Commander style) --
            // Ctrl-F1 / Ctrl-F2 hide the left / right panel; Ctrl-F4 toggles the
            // half-height mode. The menu and F-key bars stay on screen throughout.
            // (Ctrl-F3 is avoided: many terminals encode it as `CSI 1;5 R`, which
            // collides with the cursor-position report and is swallowed.)
            KeyCode::F(1) if ctrl => self.toggle_panel_hidden(0),
            KeyCode::F(2) if ctrl => self.toggle_panel_hidden(1),
            KeyCode::F(4) if ctrl => self.half_height = !self.half_height,

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
            // The Brief view is column-major (entries fill top-to-bottom, column
            // by column), so a single-step Up/Down moves the cursor *visually*
            // up/down and rolls over to the previous/next column at a column edge.
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
            // Tab flips focus, but never onto a hidden panel.
            KeyCode::Tab => {
                let other = self.other_index();
                if !self.panel_hidden[other] {
                    self.active = other;
                }
            }

            // Directory history: Alt-←/Alt-→ (and MC's Alt-y/Alt-u) step the
            // active panel back / forward through the directories it has visited.
            KeyCode::Left if alt && !ctrl => self.go_back(self.active).await,
            KeyCode::Right if alt && !ctrl => self.go_forward(self.active).await,

            // Alt-Enter copies the name under the cursor onto the command line.
            KeyCode::Enter if alt => self.insert_name_under_cursor(),

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
                if self.panels[self.active].is_tree() {
                    self.tree_enter().await;
                } else {
                    return self.enter_dir().await;
                }
            }

            // -- Command-line editing (in the multi-column Brief view, and when
            //    the command line is empty, Left/Right move sideways by a whole
            //    column: one column height in entries, clamped to the ends so the
            //    first column's Left lands on the top-left and the last column's
            //    Right lands on the very bottom) --
            KeyCode::Left => {
                let empty = self.cmd.is_empty();
                let p = &mut self.panels[self.active];
                if empty && p.brief_grid() {
                    let step = p.brief_rows.max(1) as isize;
                    p.move_cursor(-step);
                } else {
                    self.cmd.move_left();
                }
            }
            KeyCode::Right => {
                let empty = self.cmd.is_empty();
                let p = &mut self.panels[self.active];
                if empty && p.brief_grid() {
                    let step = p.brief_rows.max(1) as isize;
                    p.move_cursor(step);
                } else {
                    self.cmd.move_right();
                }
            }
            KeyCode::Backspace => self.cmd.backspace(),
            KeyCode::Delete => self.cmd.delete(),
            KeyCode::Esc => self.cmd.clear(),

            // -- View / sort / layout toggles (Ctrl chords) --
            KeyCode::Char('u') if ctrl => self.panels.swap(0, 1),
            KeyCode::Char('o') if ctrl => {
                // A nested instance (started inside another's subshell) can't run
                // its own subshell; explain rather than toggle.
                if self.subshell_disabled {
                    let title = crate::l10n::trd("Subshell disabled");
                    let msg = crate::l10n::trd(
                        "This Rat Commander is running inside another instance's subshell.",
                    );
                    self.show_info(&title, msg);
                } else {
                    return Flow::SubShell;
                }
            }
            KeyCode::Char('r') if ctrl => {
                let _ = self.active_panel().reload().await;
            }
            KeyCode::Char('t') if ctrl => self.active_panel().toggle_mark_and_advance(),
            KeyCode::Char('x') if ctrl => self.split = self.split.toggle(),
            // Ctrl-P opens the fuzzy command palette (`!alt` so AltGr still types).
            KeyCode::Char('p') if ctrl && !alt => self.open_command_palette(),
            // Command-line history: Alt-P/Alt-N cycle previous/next in place;
            // Alt-Shift-H opens the scrollable Shell History window above it.
            // (`!ctrl` so AltGr = Ctrl+Alt still composes characters instead.)
            KeyCode::Char('p') if alt && !ctrl => self.cmd.history_prev(),
            KeyCode::Char('n') if alt && !ctrl => self.cmd.history_next(),
            // The two history windows share the letter: Alt-H lists the panel's
            // directories, Alt-Shift-H the shell's commands.
            KeyCode::Char('h') if alt && !ctrl => self.open_dir_history(),
            KeyCode::Char('H') if alt && !ctrl => self.open_shell_history(),
            // Directory history (Midnight Commander keys): Alt-y back, Alt-u fwd.
            KeyCode::Char('y') if alt && !ctrl => self.go_back(self.active).await,
            KeyCode::Char('u') if alt && !ctrl => self.go_forward(self.active).await,
            // Directory hotlist (bookmarks), MC's Ctrl-\.
            KeyCode::Char('\\') if ctrl => self.open_hotlist(),
            // Alt-I points the other panel here; Alt-Shift-I sets this panel's
            // persistent listing filter (which used to hold the plain Alt-I).
            KeyCode::Char('i') if alt && !ctrl => self.sync_other_panel().await,
            KeyCode::Char('I') if alt && !ctrl => self.open_panel_filter(),
            // Alt-O shows the cursor's directory on the other panel and steps on.
            KeyCode::Char('o') if alt && !ctrl => self.chdir_other_panel().await,
            // Alt-T cycles the listing type (full / brief / details / tree).
            KeyCode::Char('t') if alt && !ctrl => {
                let side = self.active;
                let next = self.panels[side].format.toggle();
                self.set_format(side, next).await;
            }
            // Git: Ctrl-G stages/unstages, Alt-G opens the Git menu, and Alt-D
            // diffs the file against HEAD (the diff moved off Alt-G when the menu
            // took it; Ctrl-D can't be used — readline claims it to delete a
            // character on the command line, see `cmdline_edit_wanted`).
            KeyCode::Char('g') if ctrl && !alt => self.git_stage_toggle().await,
            KeyCode::Char('g') if alt && !ctrl => self.open_git_menu(),
            KeyCode::Char('d') if alt && !ctrl => self.open_git_diff().await,
            KeyCode::Char('e') if ctrl => {
                let p = self.active_panel();
                p.sort.reverse = !p.sort.reverse;
                p.resort();
            }

            // -- Selection by wildcard (only when the command line is empty) --
            KeyCode::Char('+') if self.cmd.is_empty() => self.open_select_group(true),
            KeyCode::Char('-') if self.cmd.is_empty() => self.open_select_group(false),
            KeyCode::Char('*') if self.cmd.is_empty() => self.invert_selection(),

            // -- Otherwise, type into the command line. A lone Ctrl or Alt with a
            //    letter is an (unbound) shortcut, not text, so it isn't inserted;
            //    plain keys and AltGr combos (Ctrl+Alt, for composed characters)
            //    still type normally. --
            KeyCode::Char(c) if ctrl == alt => self.cmd.insert(c),

            _ => {}
        }
        Flow::Continue
    }

    /// Alt-Enter: copy the name under the cursor onto the command line (shell-
    /// quoted when needed). In Tree view the highlighted directory's name is
    /// used; `..` is never copied.
    fn insert_name_under_cursor(&mut self) {
        let p = &self.panels[self.active];
        let name = if p.is_tree() {
            p.tree.as_ref().and_then(|t| t.selected_path()).map(|path| path.file_name())
        } else {
            p.current_entry().filter(|e| e.name != "..").map(|e| e.name.clone())
        };
        if let Some(name) = name.filter(|n| !n.is_empty()) {
            self.cmd.insert_arg(&shell_arg(&name));
        }
    }

    /// Ctrl-H: open the Shell History window over the command line (a no-op when
    /// nothing has been run yet).
    fn open_shell_history(&mut self) {
        let dlg = ShellHistoryDialog::new(&self.cmd.history);
        if !dlg.is_empty() {
            self.dialog = Some(Dialog::ShellHistory(dlg));
        }
    }

    /// Toggle the visibility of panel `side` (Ctrl-F1 / Ctrl-F2). When the panel
    /// being hidden currently has focus and the other panel is still visible,
    /// focus moves to that visible panel so the active panel is always one the
    /// user can see. Hiding both is allowed (the command line stays usable).
    fn toggle_panel_hidden(&mut self, side: usize) {
        self.panel_hidden[side] = !self.panel_hidden[side];
        let other = self.other_index();
        if self.panel_hidden[self.active] && !self.panel_hidden[other] {
            self.active = other;
        }
    }

    /// Begin a quick search with an empty query (Alt-S / Ctrl-S). The cursor
    /// doesn't move until the first letter is typed (see `jump_quick_search`).
    fn start_quick_search_empty(&mut self) {
        self.quick_search = Some(QuickSearch { query: String::new() });
    }

    /// Move the active panel's cursor to the first entry (in display order)
    /// whose name starts with the current quick-search query, case-insensitively.
    /// In Tree view the tree rows' labels are matched instead. No move when the
    /// query is empty or no entry matches (the cursor stays where it was).
    fn jump_quick_search(&mut self) {
        let q = match &self.quick_search {
            Some(qs) if !qs.query.is_empty() => qs.query.to_lowercase(),
            _ => return,
        };
        let panel = &mut self.panels[self.active];
        if panel.is_tree() {
            if let Some(tree) = panel.tree.as_mut()
                && let Some(idx) =
                    tree.rows.iter().position(|n| n.label.to_lowercase().starts_with(&q))
            {
                tree.cursor = idx;
            }
        } else if let Some(idx) =
            panel.entries.iter().position(|e| e.name.to_lowercase().starts_with(&q))
        {
            panel.cursor = idx;
        }
    }

    /// Handle a key while a quick search is active. Printable chars extend the
    /// query and re-jump; Backspace trims it (keeping the box open even when it
    /// empties); Esc or a navigation key closes it; Enter commits and falls
    /// through to the normal open-dir action; any other key exits the search and
    /// is re-dispatched as a normal panel key.
    async fn handle_quick_search_key(&mut self, key: KeyEvent) -> Flow {
        match key.code {
            // A modifier pressed on its own (Shift to reach an uppercase letter,
            // Ctrl, Alt) is reported as its own key under the enhanced keyboard
            // protocol — it must not dismiss the search.
            KeyCode::Modifier(_) => {}
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(qs) = self.quick_search.as_mut() {
                    qs.query.push(c);
                }
                self.jump_quick_search();
            }
            // Backspace trims the query but keeps the (possibly empty) box open;
            // only Esc or a navigation key dismisses it.
            KeyCode::Backspace => {
                if let Some(qs) = self.quick_search.as_mut() {
                    qs.query.pop();
                }
                self.jump_quick_search();
            }
            KeyCode::Esc => {
                self.quick_search = None;
            }
            KeyCode::Enter => {
                self.quick_search = None;
                // Boxed to break the async mutual recursion with handle_panel_key.
                return Box::pin(
                    self.handle_panel_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
                )
                .await;
            }
            _ => {
                self.quick_search = None;
                return Box::pin(self.handle_panel_key(key)).await;
            }
        }
        Flow::Continue
    }

    pub(in crate::app::state) async fn enter_dir(&mut self) -> Flow {
        let p = &self.panels[self.active];
        // Directory / ".." navigation first, then "enter archive file".
        let target = p
            .target_dir_under_cursor()
            .or_else(|| archive_target_under_cursor(p));
        let Some((newcwd, focus)) = target else {
            // Not a directory/native archive: an rc.ext `Open` rule may mount it
            // via an extfs script or run a command; else an image opens the
            // flasher (Linux), an executable is run, anything else with its
            // default app.
            if let Some(flow) = self.try_ext_open().await {
                return flow;
            }
            #[cfg(target_os = "linux")]
            if self.try_flash_under_cursor() {
                return Flow::Continue;
            }
            return self.open_or_execute_under_cursor().await;
        };
        // Re-resolve the backend: navigation may cross backends (local↔archive).
        let backend = match self.registry.resolve(&newcwd) {
            Ok(b) => b,
            Err(e) => {
                self.show_error(e.to_string());
                return Flow::Continue;
            }
        };
        // Atomic move: if the target can't be listed (e.g. permission denied),
        // the panel stays where it is rather than getting stuck in it.
        self.active_panel()
            .try_enter(newcwd, backend, focus.as_deref())
            .await;
        Flow::Continue
    }

    /// Switch panel `side` to `fmt`, building or dropping its directory tree as
    /// Tree view is entered or left.
    pub(in crate::app::state) async fn set_format(&mut self, side: usize, fmt: ViewFormat) {
        self.panels[side].format = fmt;
        if fmt == ViewFormat::Tree {
            self.panels[side].build_tree().await;
        } else {
            self.panels[side].tree = None;
        }
    }

    /// Enter on a Tree-view row: open/close the branch under the cursor and point
    /// the *other* (inactive) panel at the selected directory.
    pub(in crate::app::state) async fn tree_enter(&mut self) {
        let Some(path) = self.active_panel().tree_toggle().await else {
            return;
        };
        let backend = match self.registry.resolve(&path) {
            Ok(b) => b,
            Err(e) => return self.show_error(e.to_string()),
        };
        let other = self.other_index();
        self.panels[other].try_enter(path, backend, None).await;
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
        self.menu = Some(MenuBarState::new(active, &self.session_list(), self.side_remote()));
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
        let ext = p
            .current_entry()
            .map(|e| e.extension().to_string())
            .unwrap_or_default();
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
                Some('x') => out.push_str(&ext),
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
        // Land on the manual's outline so the reader can jump straight to a topic.
        v.open_outline();
        self.viewer = Some(v);
    }

}

/// Whether `key` should be handled as Emacs/readline editing of the command line
/// right now. The three keys that double as panel shortcuts (C-E reverse sort,
/// C-W cycle view, Alt-F File menu) only edit when the line is non-empty; when
/// empty they fall through to their panel meaning. Plain characters, Backspace,
/// Delete and the arrows are left to the main match (which has its own
/// empty-line special cases), so they are deliberately excluded here.
/// Whether the command line's readline editor claims `key` before the panel
/// shortcuts get a look in. Any panel binding must avoid these chords, or it
/// would be dead code (see the `git_shortcuts_are_not_swallowed` test).
pub(in crate::app::state) fn cmdline_edit_wanted(key: KeyEvent, empty: bool) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        // Always-editing Ctrl chords: begin/end, char left/right, delete char,
        // kill-to-end, yank, set-mark.
        KeyCode::Char('a' | 'b' | 'f' | 'd' | 'k' | 'y' | '@' | ' ') if ctrl && !alt => true,
        // C-H (delete prev char) and Alt-C-H (delete word back).
        KeyCode::Char('h') if ctrl => true,
        // Alt word motions / copy region.
        KeyCode::Char('b' | 'w') if alt && !ctrl => true,
        KeyCode::Backspace if alt => true,
        // Ctrl-W (kill word back) is unconditional now that the listing-type
        // toggle it used to share has moved to Alt-T.
        KeyCode::Char('w') if ctrl && !alt => true,
        // Panel-shortcut conflicts: edit only with text present.
        KeyCode::Char('e') if ctrl && !alt => !empty,
        KeyCode::Char('f') if alt && !ctrl => !empty,
        _ => false,
    }
}

/// Quote a filename for the command line only when it contains characters the
/// shell would treat specially; plain names are inserted verbatim so the common
/// case (`report.txt`) stays clean.
fn shell_arg(name: &str) -> String {
    let safe = !name.is_empty()
        && name.chars().all(|c| c.is_alphanumeric() || "._-+,:@%=/~".contains(c));
    if safe {
        name.to_string()
    } else {
        crate::vfs::remote::shell_quote(name)
    }
}
