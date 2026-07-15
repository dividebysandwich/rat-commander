//! The command palette (Ctrl-P): building its entry list from the live app
//! state, and running the entry the user picks.

use super::*;
use crate::panel::sort::SortKey;
use crate::vfs::remote::Protocol;

impl AppState {
    /// Build the command-palette entries from the current state and open it.
    ///
    /// The entries span four families — every menu action, the direct settings
    /// (theme / language / graphics / toggles), the saved bookmarks, and the
    /// open remote connections — so the one fuzzy box reaches all of them.
    pub(in crate::app::state) fn open_command_palette(&mut self) {
        let side = self.active;
        let mut entries: Vec<PaletteEntry> = Vec::new();

        // -- Commands (existing menu actions; labels reuse the menu catalog) --
        let cmd = |key: &str, action: MenuAction| {
            let label: String = crate::l10n::tr(key).chars().filter(|&c| c != '&').collect();
            PaletteEntry::new(label, PaletteCategory::Command, PaletteAction::Menu(action))
        };
        entries.extend([
            cmd("&View", MenuAction::View),
            cmd("&Edit", MenuAction::Edit),
            cmd("&Copy", MenuAction::Copy),
            cmd("&Rename/Move", MenuAction::Move),
            cmd("M&ulti rename", MenuAction::MultiRename),
            cmd("&Make directory", MenuAction::Mkdir),
            cmd("&Delete", MenuAction::Delete),
            cmd("C&hmod", MenuAction::Chmod),
            cmd("Cho&wn", MenuAction::Chown),
            cmd("&Symlink", MenuAction::Symlink),
            cmd("Com&press...", MenuAction::Compress),
            cmd("Chec&ksum...", MenuAction::Checksum),
            cmd("Send over &LAN...", MenuAction::SendFile),
            cmd("&Background operations...", MenuAction::BackgroundOps),
            cmd("Select gr&oup", MenuAction::SelectGroup),
            cmd("U&nselect group", MenuAction::UnselectGroup),
            cmd("&Invert selection", MenuAction::Invert),
            cmd("&Find file...", MenuAction::FindFile),
        ]);
        // Every Git-submenu action, prefixed so typing "git" surfaces them all.
        entries.extend(crate::ui::menu::GIT_MENU_KEYS.iter().map(|(key, action)| {
            let label: String = crate::l10n::tr(key).chars().filter(|&c| c != '&').collect();
            PaletteEntry::new(
                format!("Git: {label}"),
                PaletteCategory::Command,
                PaletteAction::Menu(*action),
            )
        }));
        entries.extend([
            cmd("Find d&uplicates...", MenuAction::FindDuplicates),
            cmd("Compare &directories...", MenuAction::CompareDirs),
            cmd("S&ynchronize directories...", MenuAction::SyncDirs),
            cmd("Compare fi&les...", MenuAction::CompareFiles),
            cmd("&Process explorer...", MenuAction::ProcExplorer),
            cmd("Disk &explorer...", MenuAction::DiskExplorer),
        ]);
        #[cfg(target_os = "linux")]
        entries.extend([
            cmd("Disk &manager...", MenuAction::DiskManager),
            cmd("Network &connections...", MenuAction::NetworkConnections),
        ]);
        entries.extend([
            cmd("S&wap panels", MenuAction::SwapPanels),
            cmd("&Re-read directories", MenuAction::Refresh),
            cmd("&Toggle split V/H", MenuAction::ToggleSplit),
            cmd("Directory &hotlist...", MenuAction::Hotlist),
            cmd("Panel f&ilter...", MenuAction::PanelFilter),
            cmd("&Settings...", MenuAction::Settings),
            cmd("&Confirmations...", MenuAction::Confirmations),
            cmd("&Edit themes...", MenuAction::EditThemes),
            cmd("Edit e&xtensions...", MenuAction::EditExtensions),
            cmd("Edit &menu file...", MenuAction::EditUserMenu),
            // Panel view / sort act on the active panel.
            cmd("&Full view", MenuAction::SetFormat(side, ViewFormat::Full)),
            cmd("&Brief view", MenuAction::SetFormat(side, ViewFormat::Brief)),
            cmd("&Details view", MenuAction::SetFormat(side, ViewFormat::Details)),
            cmd("Tree v&iew", MenuAction::SetFormat(side, ViewFormat::Tree)),
            cmd("Sort: &Name", MenuAction::SetSort(side, SortKey::Name)),
            cmd("Sort: &Extension", MenuAction::SetSort(side, SortKey::Extension)),
            cmd("Sort: &Size", MenuAction::SetSort(side, SortKey::Size)),
            cmd("Sort: &Modify time", MenuAction::SetSort(side, SortKey::ModifyTime)),
            cmd("Sort: &Unsorted", MenuAction::SetSort(side, SortKey::Unsorted)),
            cmd("&Reverse order", MenuAction::ToggleReverse(side)),
            cmd("SFT&P connection...", MenuAction::Connect(side, Protocol::Sftp)),
            cmd("F&TP connection...", MenuAction::Connect(side, Protocol::Ftp)),
            cmd("S&CP connection...", MenuAction::Connect(side, Protocol::Scp)),
            cmd("&Quit", MenuAction::Quit),
        ]);

        // -- Settings: boolean toggles (label reuses the settings-form keys) --
        let check = |on: bool| if on { "✓" } else { "✗" };
        let toggle = |key: &str, setting: BoolSetting, on: bool| {
            PaletteEntry::new(
                crate::l10n::tr(key),
                PaletteCategory::Setting,
                PaletteAction::ToggleBool(setting),
            )
            .with_hint(check(on))
        };
        entries.extend([
            toggle("Truecolor (gradients)", BoolSetting::Truecolor, self.truecolor),
            toggle("Animations", BoolSetting::Animation, self.config.animation),
            toggle("System status widget", BoolSetting::SystemStatus, self.config.system_status),
            toggle("Reshape RTL text", BoolSetting::ReshapeRtl, self.config.reshape_rtl),
            toggle("Use internal viewer", BoolSetting::InternalViewer, self.config.use_internal_viewer),
            toggle("Use internal editor", BoolSetting::InternalEditor, self.config.use_internal_editor),
            toggle("Confirm delete", BoolSetting::ConfirmDelete, self.config.confirm_delete),
            toggle("Confirm overwrite", BoolSetting::ConfirmOverwrite, self.config.confirm_overwrite),
            toggle("Confirm execute", BoolSetting::ConfirmExecute, self.config.confirm_execute),
            toggle("Confirm unmount", BoolSetting::ConfirmUnmount, self.config.confirm_unmount),
            toggle("Confirm exit", BoolSetting::ConfirmExit, self.config.confirm_exit),
        ]);

        // -- Settings: themes, languages and graphics as direct switches --
        let theme_word = crate::l10n::tr("Theme");
        for name in crate::ui::theme::palette_names() {
            let active = name == self.config.theme;
            entries.push(
                PaletteEntry::new(
                    format!("{theme_word}: {name}"),
                    PaletteCategory::Setting,
                    PaletteAction::SetTheme(name),
                )
                .with_hint(if active { "✓" } else { "" }),
            );
        }
        let lang_word = crate::l10n::tr("Language");
        let active_lang = crate::l10n::active_name();
        for name in crate::l10n::available() {
            let active = name == active_lang;
            entries.push(
                PaletteEntry::new(
                    format!("{lang_word}: {name}"),
                    PaletteCategory::Setting,
                    PaletteAction::SetLanguage(name),
                )
                .with_hint(if active { "✓" } else { "" }),
            );
        }
        let gfx_word = crate::l10n::tr("Graphics");
        for (disp, pref) in [
            ("Auto", "auto"),
            ("Off", "off"),
            ("Kitty", "kitty"),
            ("Sixel", "sixel"),
            ("iTerm2", "iterm"),
        ] {
            let active = pref == self.config.graphics;
            entries.push(
                PaletteEntry::new(
                    format!("{gfx_word}: {disp}"),
                    PaletteCategory::Setting,
                    PaletteAction::SetGraphics(pref.to_string()),
                )
                .with_hint(if active { "✓" } else { "" }),
            );
        }

        // -- Bookmarks: add/remove the current local dir, then jump to each --
        if self.panels[side].cwd.scheme == "file" {
            let cur = self.panels[side].cwd.path.to_string_lossy().to_string();
            let bookmarked = self.config.bookmarks.iter().any(|b| b == &cur);
            let key = if bookmarked {
                "Remove current directory from bookmarks"
            } else {
                "Add current directory to bookmarks"
            };
            entries.push(PaletteEntry::new(
                crate::l10n::tr(key),
                PaletteCategory::Bookmark,
                PaletteAction::ToggleBookmarkCurrent,
            ));
        }
        for bm in &self.config.bookmarks {
            entries.push(PaletteEntry::new(
                bm.clone(),
                PaletteCategory::Bookmark,
                PaletteAction::JumpBookmark(bm.clone()),
            ));
        }

        // -- Open remote connections: switch the active panel to each --
        for (id, label) in self.session_list() {
            entries.push(PaletteEntry::new(
                label,
                PaletteCategory::Connection,
                PaletteAction::Menu(MenuAction::SwitchSession(side, id)),
            ));
        }

        // -- Stored remote servers: reconnect (opens the prefilled connect form) --
        for entry in &self.config.recent_remotes {
            entries.push(PaletteEntry::new(
                format!("{} {}", entry.protocol.to_uppercase(), entry.label()),
                PaletteCategory::Connection,
                PaletteAction::ConnectRemote(side, entry.clone()),
            ));
        }

        self.dialog = Some(Dialog::CommandPalette(CommandPaletteDialog::new(entries)));
    }

    /// Run the entry chosen from the command palette. Menu-backed entries defer
    /// to [`AppState::run_menu_action`] (so they behave exactly like the menu);
    /// the rest apply a setting, jump to a bookmark, or toggle one in place.
    pub(in crate::app::state) async fn run_palette_action(&mut self, action: PaletteAction) -> Flow {
        match action {
            PaletteAction::Menu(a) => return self.run_menu_action(a).await,
            PaletteAction::SetTheme(name) => {
                self.config.theme = name;
                self.theme = Theme::by_name(&self.config.theme, self.truecolor);
                self.save_config_reporting();
            }
            PaletteAction::SetLanguage(name) => {
                crate::l10n::set_active_by_name(&name);
                self.config.language = if name == "English" { None } else { Some(name) };
                self.save_config_reporting();
            }
            PaletteAction::SetGraphics(pref) => {
                self.config.graphics = pref;
                if let Some(g) = self.gfx.as_mut() {
                    g.apply_pref(&self.config.graphics);
                }
                self.save_config_reporting();
            }
            PaletteAction::ToggleBool(b) => {
                self.toggle_bool_setting(b);
                self.save_config_reporting();
            }
            PaletteAction::JumpBookmark(path) => self.jump_to_local_dir(path).await,
            PaletteAction::ToggleBookmarkCurrent => {
                self.toggle_bookmark_current();
                self.save_config_reporting();
            }
            PaletteAction::ConnectRemote(side, entry) => {
                // One panel must stay local — mirror the Connect menu guard.
                if self.other_panel_is_remote(side) {
                    self.show_error(
                        "The other panel is already remote — one panel must stay local.".to_string(),
                    );
                } else if let Some(dlg) = FormDialog::connect_from(&entry, side) {
                    self.dialog = Some(Dialog::Form(dlg));
                }
            }
        }
        Flow::Continue
    }

    /// Flip a boolean setting, applying the same side effects the Settings form
    /// applies on submit (rebuild the theme for truecolor, push RTL reshaping to
    /// the l10n layer, …).
    fn toggle_bool_setting(&mut self, b: BoolSetting) {
        match b {
            BoolSetting::Truecolor => {
                self.truecolor = !self.truecolor;
                self.config.truecolor = Some(self.truecolor);
                self.theme = Theme::by_name(&self.config.theme, self.truecolor);
            }
            BoolSetting::Animation => self.config.animation = !self.config.animation,
            BoolSetting::SystemStatus => self.config.system_status = !self.config.system_status,
            BoolSetting::ReshapeRtl => {
                self.config.reshape_rtl = !self.config.reshape_rtl;
                crate::l10n::set_reshape_rtl(self.config.reshape_rtl);
            }
            BoolSetting::InternalViewer => {
                self.config.use_internal_viewer = !self.config.use_internal_viewer
            }
            BoolSetting::InternalEditor => {
                self.config.use_internal_editor = !self.config.use_internal_editor
            }
            BoolSetting::ConfirmDelete => self.config.confirm_delete = !self.config.confirm_delete,
            BoolSetting::ConfirmOverwrite => {
                self.config.confirm_overwrite = !self.config.confirm_overwrite
            }
            BoolSetting::ConfirmExecute => {
                self.config.confirm_execute = !self.config.confirm_execute
            }
            BoolSetting::ConfirmUnmount => {
                self.config.confirm_unmount = !self.config.confirm_unmount
            }
            BoolSetting::ConfirmExit => self.config.confirm_exit = !self.config.confirm_exit,
        }
    }

    /// Add the active panel's directory to the bookmarks, or remove it if it is
    /// already bookmarked. Only local directories are bookmarkable.
    fn toggle_bookmark_current(&mut self) {
        let p = &self.panels[self.active];
        if p.cwd.scheme != "file" {
            return self.show_error("Bookmarks are only for local directories");
        }
        let path = p.cwd.path.to_string_lossy().to_string();
        if let Some(pos) = self.config.bookmarks.iter().position(|b| b == &path) {
            self.config.bookmarks.remove(pos);
        } else {
            self.config.bookmarks.push(path);
        }
    }

    /// Point the active panel at a local directory (a bookmark jump), switching
    /// it off any remote/archive location it was on.
    pub(in crate::app::state) async fn jump_to_local_dir(&mut self, path: String) {
        let target = VfsPath::local(normalize_path(Path::new(&path)));
        match self.registry.resolve(&target) {
            Ok(backend) => {
                self.active_panel().try_enter(target, backend, None).await;
            }
            Err(e) => self.show_error(e.to_string()),
        }
    }

    /// Persist the config after a palette-driven settings change, surfacing any
    /// write error the same way the Settings dialog does.
    pub(in crate::app::state) fn save_config_reporting(&mut self) {
        if let Err(e) = self.config.save() {
            self.show_error(format!("could not save settings: {e}"));
        }
    }
}
