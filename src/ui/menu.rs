//! The file manager's pulldown menu bar (F9 / F2): its actions and the menus
//! they are laid out in. The bar itself is the shared widget in
//! [`crate::ui::pulldown`], which the editor's menu is built on too.

use crate::panel::ViewFormat;
use crate::panel::sort::{SortConfig, SortKey};
use crate::ui::menubar::titles;
use crate::ui::pulldown::{self, Action, Menu, MenuItem, PulldownState};
use crate::vfs::remote::Protocol;
use ratatui::layout::Rect;

/// What [`MenuAction::CopyToClipboard`] puts on the clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipTarget {
    /// The cursor entry's bare file name.
    Name,
    /// The cursor entry's full path, as [`crate::vfs::VfsPath::display`] shows it.
    FullPath,
    /// Every selected path, one per line (the cursor entry when nothing is
    /// selected, so the key always does something).
    Selection,
}

/// An action a menu item triggers. Mapped to app behaviour in `AppState`.
#[derive(Debug, Clone, Copy)]
pub enum MenuAction {
    Separator,
    View,
    Edit,
    Copy,
    Move,
    /// Open the multi-rename dialog for the selected files.
    MultiRename,
    Mkdir,
    Delete,
    Chmod,
    Chown,
    Symlink,
    Compress,
    /// Compute a checksum of the file under the cursor.
    Checksum,
    /// Share the selected file(s) with a nearby device over the LAN (QR code).
    SendFile,
    /// Take files from a nearby device into the active panel's directory over
    /// the LAN (QR code).
    ReceiveFiles,
    /// Start the screensaver now (command palette).
    Screensaver,
    /// Opens the Git submenu (File → Git, or Alt-G). Never runs an action itself.
    GitMenu,
    /// Stage / unstage the file(s) under the cursor (git) — the Ctrl-G toggle.
    GitStage,
    /// Open the side-by-side diff of the file under the cursor against HEAD.
    GitDiff,
    /// `git status` of the panel's repository, shown as raw output.
    GitStatus,
    GitBrowseRev,
    /// `git log` of the panel's repository, shown as raw output.
    GitLog,
    /// `git add` the selected files/directories.
    GitAdd,
    /// `git restore --staged` the selected files/directories.
    GitUnstage,
    /// `git rm` the selected files/directories (confirmed).
    GitRemove,
    /// `git restore` — discard worktree changes to the selection (confirmed).
    GitRestore,
    /// Commit the index (message / amend / stage-all collected in a form).
    GitCommit,
    /// Save the working tree as a stash.
    GitStash,
    /// Open the stash picker (show / apply / pop / drop).
    GitStashList,
    /// `git fetch` (remote / prune options collected in a form).
    GitFetch,
    /// `git pull` (rebase option collected in a form).
    GitPull,
    /// `git push` (force / force-with-lease / upstream options in a form).
    GitPush,
    /// Pull then push in one step.
    GitSync,
    /// Switch branches, picked from a dropdown of local + remote branches.
    GitCheckout,
    /// `git reset` (mode + target collected in a form).
    GitReset,
    /// `git init` a repository in the panel's directory (confirmed).
    GitInit,
    /// `git clone` a URL into the panel's directory (form).
    GitClone,
    /// Copy the cursor file's name or full path, or every selected path, to the
    /// system clipboard.
    CopyToClipboard(ClipTarget),
    /// Open the list of running background transfers.
    BackgroundOps,
    SelectGroup,
    UnselectGroup,
    Invert,
    SetFormat(usize, ViewFormat),
    /// Opens a panel menu's Sort order submenu. Never runs an action itself.
    SortMenu,
    SetSort(usize, SortKey),
    ToggleReverse(usize),
    /// Toggle grouping directories before files in a panel's listing.
    ToggleDirsFirst(usize),
    SwapPanels,
    Refresh,
    ToggleSplit,
    FindFile,
    /// Run a command and list the files its output names (external panelize).
    PanelizeCommand,
    /// Mark files identical between the left and right panel directories.
    FindDuplicates,
    ProcExplorer,
    DiskExplorer,
    DiskManager,
    /// Open the network-connections explorer (Linux only).
    NetworkConnections,
    CompareDirs,
    CompareFiles,
    /// Mirror one panel's directory onto the other (Command → Synchronize).
    SyncDirs,
    /// Open the fuzzy command palette (Ctrl-P).
    CommandPalette,
    /// Open the directory hotlist / bookmarks (Ctrl-\).
    Hotlist,
    /// Set the active panel's persistent listing filter.
    PanelFilter,
    /// Show the active panel's directory history as a pickable list (Alt-H).
    DirHistory,
    /// Point the other panel at the active panel's directory (Alt-I).
    SyncPanels,
    /// Show the cursor's directory on the other panel and step on (Alt-O).
    ChdirOther,
    Connect(usize, Protocol),
    Disconnect(usize),
    /// Switch a panel (side) to an already-open remote session by id.
    SwitchSession(usize, usize),
    /// Disconnect (with confirmation) the remote session with this id.
    DisconnectSession(usize),
    /// Open the drive-letter picker for a panel (Windows).
    #[cfg_attr(not(windows), allow(dead_code))] // constructed only under cfg(windows)
    Drive(usize),
    Settings,
    /// Open Settings on its Confirmations tab (a command-palette entry; the
    /// Options menu has no item of its own for it).
    Confirmations,
    /// Open `themes.toml` in the internal editor.
    EditThemes,
    /// Open `rc.ext` (file associations) in the internal editor.
    EditExtensions,
    /// Open the F2 user `menu` file in the internal editor.
    EditUserMenu,
    Quit,
}

impl Action for MenuAction {
    fn separator() -> Self {
        MenuAction::Separator
    }

    fn is_separator(self) -> bool {
        matches!(self, MenuAction::Separator)
    }
}

/// The file manager's menu bar: the shared pulldown widget over [`MenuAction`].
pub type MenuBarState = PulldownState<MenuAction>;
/// What a key press did to the file manager's menu bar.
pub type MenuSignal = pulldown::MenuSignal<MenuAction>;
/// One item of the file manager's menus.
type FileMenuItem = MenuItem<MenuAction>;

// The item constructors, bound to this menu's action type so the builders below
// read as they did when the machinery lived here.
fn item(label: &str, action: MenuAction) -> FileMenuItem {
    pulldown::item(label, action)
}
fn item_raw(label: String, action: MenuAction) -> FileMenuItem {
    pulldown::item_raw(label, action)
}
fn item_key(label: &str, shortcut: &'static str, action: MenuAction) -> FileMenuItem {
    pulldown::item_key(label, shortcut, action)
}
fn item_sub(
    label: &str,
    shortcut: &'static str,
    action: MenuAction,
    submenu: Vec<FileMenuItem>,
) -> FileMenuItem {
    pulldown::item_sub(label, shortcut, action, submenu)
}
fn sep() -> FileMenuItem {
    pulldown::sep()
}

impl MenuBarState {
    /// Build the standard menu set (Left, File, Command, Options, Right).
    /// `active` selects which top menu is initially open (0 = Left, 4 = Right).
    /// `sessions` are the open remote connections `(id, label)`, listed in each
    /// panel menu so they can be switched to / disconnected without the drive
    /// picker. `side_remote` is `[left, right]`: whether each panel is on a
    /// remote directory, which enables its "Go local" item (greyed when local).
    pub fn new(active: usize, sessions: &[(usize, String)], side_remote: [bool; 2]) -> Self {
        let panel_menu = |side: usize| {
            // `items` is grown below with the open-session rows (and, on Windows,
            // a leading Drive entry).
            let mut items = vec![
                item("&Full view", MenuAction::SetFormat(side, ViewFormat::Full)),
                item("&Brief view", MenuAction::SetFormat(side, ViewFormat::Brief)),
                item("&Details view", MenuAction::SetFormat(side, ViewFormat::Details)),
                item("Tree v&iew", MenuAction::SetFormat(side, ViewFormat::Tree)),
                item("&3D view", MenuAction::SetFormat(side, ViewFormat::Space3d)),
                item("T&humbnails view", MenuAction::SetFormat(side, ViewFormat::Thumbs)),
                item("&Activity log", MenuAction::SetFormat(side, ViewFormat::Activity)),
                sep(),
                item_sub("&Sort order", "▶", MenuAction::SortMenu, sort_menu_items(side)),
                sep(),
                item("SFT&P connection...", MenuAction::Connect(side, Protocol::Sftp)),
                item("F&TP connection...", MenuAction::Connect(side, Protocol::Ftp)),
                item("FTPS c&onnection...", MenuAction::Connect(side, Protocol::Ftps)),
                item("S&CP connection...", MenuAction::Connect(side, Protocol::Scp)),
                item("Go &local (keep session)", MenuAction::Disconnect(side))
                    .disabled(!side_remote[side]),
            ];
            // List the open connections: one row to switch this panel to each,
            // then one row to disconnect each. Empty when nothing is connected.
            if !sessions.is_empty() {
                items.push(sep());
                for (id, label) in sessions {
                    items.push(item_raw(
                        format!("Go to {label}"),
                        MenuAction::SwitchSession(side, *id),
                    ));
                }
                items.push(sep());
                for (id, label) in sessions {
                    items.push(item_raw(
                        format!("Disconnect {label}"),
                        MenuAction::DisconnectSession(*id),
                    ));
                }
            }
            // Drive-letter switching is a Windows concept; Alt-F1 (left) / Alt-F2
            // (right) are the matching shortcuts.
            #[cfg(windows)]
            {
                let label =
                    if side == 0 { "&Drive...      Alt-F1" } else { "&Drive...      Alt-F2" };
                items.insert(0, sep());
                items.insert(0, item(label, MenuAction::Drive(side)));
            }
            Menu { items }
        };

        let file = Menu {
            items: vec![
                item_key("&View", "F3", MenuAction::View),
                item_key("&Edit", "F4", MenuAction::Edit),
                item_key("&Copy", "F5", MenuAction::Copy),
                item_key("&Rename/Move", "F6", MenuAction::Move),
                item_key("M&ulti rename", "Shift-F6", MenuAction::MultiRename),
                item_key("&Make directory", "F7", MenuAction::Mkdir),
                item_key("&Delete", "F8", MenuAction::Delete),
                sep(),
                item("C&hmod", MenuAction::Chmod),
                item("Cho&wn", MenuAction::Chown),
                item("&Symlink", MenuAction::Symlink),
                sep(),
                item("Com&press...", MenuAction::Compress),
                item("Chec&ksum...", MenuAction::Checksum),
                item("Send over &LAN...", MenuAction::SendFile),
                item("Receive over L&AN...", MenuAction::ReceiveFiles),
                item("Cop&y path to clipboard", MenuAction::CopyToClipboard(ClipTarget::FullPath)),
                sep(),
                item_sub("&Git", "Alt-G  ▶", MenuAction::GitMenu, git_menu_items()),
                sep(),
                item("&Background operations...", MenuAction::BackgroundOps),
                sep(),
                item_key("Select gr&oup", "+", MenuAction::SelectGroup),
                item_key("U&nselect group", "-", MenuAction::UnselectGroup),
                item_key("&Invert selection", "*", MenuAction::Invert),
                sep(),
                item_key("&Quit", "F10", MenuAction::Quit),
            ],
        };

        let mut command_items = vec![
            item("C&ommand palette...", MenuAction::CommandPalette),
            item_key("Directory &hotlist...", "Ctrl-\\", MenuAction::Hotlist),
            item_key("Directory hi&story...", "Alt-H", MenuAction::DirHistory),
            item_key("Sy&nc panels", "Alt-I", MenuAction::SyncPanels),
            item_key("Show directory on other p&anel", "Alt-O", MenuAction::ChdirOther),
            item_key("Panel f&ilter...", "Alt-Shift-I", MenuAction::PanelFilter),
            sep(),
            item("&Find file...", MenuAction::FindFile),
            item("Paneli&ze command output...", MenuAction::PanelizeCommand),
            item("Find d&uplicates...", MenuAction::FindDuplicates),
            item("Compare &directories...", MenuAction::CompareDirs),
            item("S&ynchronize directories...", MenuAction::SyncDirs),
            item("Compare fi&les...", MenuAction::CompareFiles),
            item("&Process explorer...", MenuAction::ProcExplorer),
            item("Disk &explorer...", MenuAction::DiskExplorer),
        ];
        // The disk mounter relies on Linux `/proc`+`/sys` and `mount`/`sudo`;
        // it isn't offered on other platforms.
        #[cfg(target_os = "linux")]
        command_items.push(item("Disk &manager...", MenuAction::DiskManager));
        // The network explorer parses Linux `ss` output; Linux only.
        #[cfg(target_os = "linux")]
        command_items.push(item("Network &connections...", MenuAction::NetworkConnections));
        command_items.extend([
            sep(),
            item("S&wap panels", MenuAction::SwapPanels),
            item("&Re-read directories", MenuAction::Refresh),
            item("&Toggle split V/H", MenuAction::ToggleSplit),
        ]);
        let command = Menu { items: command_items };

        let options = Menu {
            items: vec![
                item("&Settings...", MenuAction::Settings),
                item("&Edit themes...", MenuAction::EditThemes),
                item("Edit e&xtensions...", MenuAction::EditExtensions),
                item("Edit &menu file...", MenuAction::EditUserMenu),
            ],
        };

        MenuBarState::build(
            titles().to_vec(),
            vec![panel_menu(0), file, command, options, panel_menu(1)],
            active,
        )
    }

    /// Open the bar straight into the File menu's **Git** submenu (Alt-G).
    pub fn new_git(sessions: &[(usize, String)], side_remote: [bool; 2]) -> Self {
        let mut m = Self::new(1, sessions, side_remote);
        let git = m.menus()[1].items.iter().position(|it| matches!(it.action, MenuAction::GitMenu));
        if let Some(idx) = git {
            m.item = idx;
            m.open_sub();
        }
        m
    }

    /// Tick each panel's current sort key and order toggles in its Sort order
    /// submenu. `sort` is `[left, right]`.
    pub fn with_sort(mut self, sort: [SortConfig; 2]) -> Self {
        for (menu, cfg) in [(0, sort[0]), (4, sort[1])] {
            let Some(parent) = self.menus[menu]
                .items
                .iter_mut()
                .find(|it| matches!(it.action, MenuAction::SortMenu))
            else {
                continue;
            };
            for it in &mut parent.submenu {
                let on = match it.action {
                    MenuAction::SetSort(_, key) => key == cfg.key,
                    MenuAction::ToggleReverse(_) => cfg.reverse,
                    MenuAction::ToggleDirsFirst(_) => cfg.dirs_first,
                    _ => false,
                };
                it.shortcut = match (on, it.shortcut) {
                    (true, "Ctrl-E") => "✓ Ctrl-E",
                    (true, _) => "✓",
                    (false, s) => s,
                };
            }
        }
        self
    }

    /// The top-bar title index at screen column `col` on the menu-bar row, or
    /// `None` — used to open the bar on a click, before it has been drawn.
    pub fn title_index_at(area: Rect, col: u16, row: u16) -> Option<usize> {
        pulldown::title_index_at(&titles(), area, col, row)
    }
}

impl Default for MenuBarState {
    fn default() -> Self {
        Self::new(1, &[], [false, false])
    }
}

/// The sort keys offered in a panel's Sort order submenu (and the command
/// palette), with their label keys.
pub const SORT_KEYS: &[(&str, SortKey)] = &[
    ("Sort: &Name", SortKey::Name),
    ("Sort: &Extension", SortKey::Extension),
    ("Sort: &Size", SortKey::Size),
    ("Sort: &Modify time", SortKey::ModifyTime),
    ("Sort: &Change time", SortKey::ChangeTime),
    ("Sort: &Access time", SortKey::AccessTime),
    ("Sort: &Birth time", SortKey::BirthTime),
    ("Sort: &Unsorted", SortKey::Unsorted),
];

/// A panel menu's Sort order submenu: the sort keys, then the order toggles.
/// [`MenuBarState::with_sort`] ticks the ones currently in effect.
fn sort_menu_items(side: usize) -> Vec<FileMenuItem> {
    let mut items: Vec<FileMenuItem> =
        SORT_KEYS.iter().map(|&(label, key)| item(label, MenuAction::SetSort(side, key))).collect();
    items.extend([
        sep(),
        item_key("&Reverse order", "Ctrl-E", MenuAction::ToggleReverse(side)),
        item("&Directories first", MenuAction::ToggleDirsFirst(side)),
    ]);
    items
}

/// The Git submenu (File → Git, or Alt-G), newest-to-oldest in workflow order:
/// inspect, stage, commit, exchange with the remote, switch/undo, set up.
/// [`GIT_MENU_KEYS`] mirrors these label keys for the l10n accelerator test and
/// the command palette.
fn git_menu_items() -> Vec<FileMenuItem> {
    vec![
        item("&Status...", MenuAction::GitStatus),
        item("&Log...", MenuAction::GitLog),
        item_key("&Diff vs HEAD", "Alt-D", MenuAction::GitDiff),
        sep(),
        item("&Add (stage)", MenuAction::GitAdd),
        item_key("Stage/unsta&ge", "Ctrl-G", MenuAction::GitStage),
        item("&Unstage", MenuAction::GitUnstage),
        item("Re&move...", MenuAction::GitRemove),
        item("Res&tore (discard)...", MenuAction::GitRestore),
        sep(),
        item("&Commit...", MenuAction::GitCommit),
        item("Stash sa&ve...", MenuAction::GitStash),
        item("Stash&es...", MenuAction::GitStashList),
        sep(),
        item("&Fetch...", MenuAction::GitFetch),
        item("&Pull...", MenuAction::GitPull),
        item("Pus&h...", MenuAction::GitPush),
        item("S&ync (pull + push)", MenuAction::GitSync),
        sep(),
        item("Chec&kout...", MenuAction::GitCheckout),
        item("&Reset...", MenuAction::GitReset),
        sep(),
        item("&Init repository...", MenuAction::GitInit),
        item("Clo&ne...", MenuAction::GitClone),
        sep(),
        item("&Browse a revision...", MenuAction::GitBrowseRev),
    ]
}

/// The Git submenu's label keys paired with their actions — the single source
/// shared by the menu, the command palette, and the l10n accelerator test.
pub const GIT_MENU_KEYS: &[(&str, MenuAction)] = &[
    ("&Status...", MenuAction::GitStatus),
    ("&Log...", MenuAction::GitLog),
    ("&Diff vs HEAD", MenuAction::GitDiff),
    ("&Add (stage)", MenuAction::GitAdd),
    ("Stage/unsta&ge", MenuAction::GitStage),
    ("&Unstage", MenuAction::GitUnstage),
    ("Re&move...", MenuAction::GitRemove),
    ("Res&tore (discard)...", MenuAction::GitRestore),
    ("&Commit...", MenuAction::GitCommit),
    ("Stash sa&ve...", MenuAction::GitStash),
    ("Stash&es...", MenuAction::GitStashList),
    ("&Fetch...", MenuAction::GitFetch),
    ("&Pull...", MenuAction::GitPull),
    ("Pus&h...", MenuAction::GitPush),
    ("S&ync (pull + push)", MenuAction::GitSync),
    ("Chec&kout...", MenuAction::GitCheckout),
    ("&Reset...", MenuAction::GitReset),
    ("&Init repository...", MenuAction::GitInit),
    ("Clo&ne...", MenuAction::GitClone),
    ("&Browse a revision...", MenuAction::GitBrowseRev),
];
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::pulldown::split_hotkey;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn key_code(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    #[test]
    fn split_hotkey_extracts_marker() {
        assert_eq!(split_hotkey("&Copy"), ("Copy".to_string(), Some(0)));
        assert_eq!(split_hotkey("Select &group"), ("Select group".to_string(), Some(7)));
        assert_eq!(split_hotkey("U&nselect"), ("Unselect".to_string(), Some(1)));
        assert_eq!(split_hotkey("plain"), ("plain".to_string(), None));
    }

    #[test]
    fn item_hotkeys_match_the_request() {
        // File menu (active index 1): the requested accelerators.
        let m = MenuBarState::new(1, &[], [false, false]);
        let file = &m.menus[1];
        let hk = |action_label: char| {
            file.items.iter().find(|it| it.hotkey() == Some(action_label)).map(|it| it.action)
        };
        assert!(matches!(hk('v'), Some(MenuAction::View)));
        assert!(matches!(hk('e'), Some(MenuAction::Edit)));
        assert!(matches!(hk('c'), Some(MenuAction::Copy)));
        assert!(matches!(hk('r'), Some(MenuAction::Move)));
        assert!(matches!(hk('m'), Some(MenuAction::Mkdir)));
        assert!(matches!(hk('d'), Some(MenuAction::Delete)));
        assert!(matches!(hk('n'), Some(MenuAction::UnselectGroup)));
        assert!(matches!(hk('i'), Some(MenuAction::Invert)));
        assert!(matches!(hk('q'), Some(MenuAction::Quit)));
        // The new Checksum entry is accelerated by 'k' (unique in the File menu).
        assert!(matches!(hk('k'), Some(MenuAction::Checksum)));
        // 'g' belongs to the Git submenu, so "Select group" moved to 'o'.
        assert!(matches!(hk('g'), Some(MenuAction::GitMenu)));
        assert!(matches!(hk('o'), Some(MenuAction::SelectGroup)));
    }

    #[test]
    fn typing_an_item_hotkey_activates_it() {
        // File menu: 'c' → Copy, 'o' → Select group.
        let mut m = MenuBarState::new(1, &[], [false, false]);
        assert!(matches!(m.handle_key(key('c')), MenuSignal::Activate(MenuAction::Copy)));
        let mut m = MenuBarState::new(1, &[], [false, false]);
        assert!(matches!(m.handle_key(key('o')), MenuSignal::Activate(MenuAction::SelectGroup)));
        // Command menu (index 2): 'f' → Find file, 'w' → Swap panels.
        let mut m = MenuBarState::new(2, &[], [false, false]);
        assert!(matches!(m.handle_key(key('f')), MenuSignal::Activate(MenuAction::FindFile)));
        let mut m = MenuBarState::new(2, &[], [false, false]);
        assert!(matches!(m.handle_key(key('w')), MenuSignal::Activate(MenuAction::SwapPanels)));
    }

    #[test]
    fn f10_and_f9_and_esc_close_the_menu() {
        for code in [KeyCode::F(10), KeyCode::F(9), KeyCode::Esc] {
            let mut m = MenuBarState::new(1, &[], [false, false]);
            let sig = m.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
            assert!(matches!(sig, MenuSignal::Close), "{code:?} should close the menu");
        }
    }

    #[test]
    fn hotkeys_are_unique_within_each_menu() {
        // Sessions are present to make sure their runtime labels never introduce
        // a duplicate accelerator into a panel menu.
        let sessions = [(0usize, "sftp://u@host".to_string())];
        let m = MenuBarState::new(0, &sessions, [true, true]);
        for (mi, menu) in m.menus.iter().enumerate() {
            let mut seen = Vec::new();
            for it in &menu.items {
                if let Some(hk) = it.hotkey() {
                    assert!(!seen.contains(&hk), "duplicate hotkey {hk:?} in menu {mi}");
                    seen.push(hk);
                }
            }
        }
    }

    #[test]
    fn open_sessions_appear_in_both_panel_menus() {
        let sessions = [(7usize, "sftp://u@host".to_string())];
        let m = MenuBarState::new(0, &sessions, [true, true]);
        // Panel menus are index 0 (left) and 4 (right); each should list a
        // switch item for the session on its own side plus a disconnect item.
        for (side, mi) in [(0usize, 0usize), (1, 4)] {
            let items = &m.menus[mi].items;
            assert!(
                items.iter().any(|it| matches!(
                    it.action,
                    MenuAction::SwitchSession(s, 7) if s == side
                )),
                "menu {mi} should offer switching side {side} to session 7"
            );
            assert!(
                items.iter().any(|it| matches!(it.action, MenuAction::DisconnectSession(7))),
                "menu {mi} should offer disconnecting session 7"
            );
        }
        // With no sessions, no such items appear.
        let m = MenuBarState::new(0, &[], [true, true]);
        assert!(
            !m.menus[0].items.iter().any(|it| matches!(it.action, MenuAction::SwitchSession(..)))
        );
    }

    #[test]
    fn go_local_is_disabled_when_panel_is_already_local() {
        // Left panel local, right panel remote: only the right menu's "Go local"
        // is selectable; the left one is greyed out and can't be activated.
        let m = MenuBarState::new(0, &[], [false, true]);
        let go_local = |mi: usize| {
            m.menus[mi]
                .items
                .iter()
                .find(|it| matches!(it.action, MenuAction::Disconnect(_)))
                .expect("panel menu has a Go local item")
        };
        assert!(!go_local(0).selectable(), "left is local → Go local disabled");
        assert!(go_local(4).selectable(), "right is remote → Go local enabled");

        // A disabled Go local can't be reached by its 'l' accelerator.
        let mut m = MenuBarState::new(0, &[], [false, true]);
        assert!(
            matches!(m.handle_key(key('l')), MenuSignal::Stay),
            "typing 'l' must not activate a disabled Go local"
        );
    }

    #[test]
    fn sort_submenu_sets_keys_and_ticks_the_current_ones() {
        // 's' opens the panel menu's Sort order submenu; 'b' then sorts by birth time.
        let mut m = MenuBarState::new(4, &[], [false, false]);
        assert!(matches!(m.handle_key(key('s')), MenuSignal::Stay));
        assert!(m.sub_open, "'s' opens the Sort order submenu");
        assert!(matches!(
            m.handle_key(key('b')),
            MenuSignal::Activate(MenuAction::SetSort(1, SortKey::BirthTime))
        ));
        let mut m = MenuBarState::new(0, &[], [false, false]);
        m.handle_key(key('s'));
        assert!(matches!(
            m.handle_key(key('d')),
            MenuSignal::Activate(MenuAction::ToggleDirsFirst(0))
        ));

        let right = SortConfig {
            key: SortKey::ChangeTime,
            reverse: true,
            dirs_first: false,
            ..Default::default()
        };
        let m = MenuBarState::new(0, &[], [false, false]).with_sort([SortConfig::default(), right]);
        let ticks = |menu: usize| -> Vec<(String, &'static str)> {
            let parent =
                m.menus[menu].items.iter().find(|it| matches!(it.action, MenuAction::SortMenu));
            let sub = &parent.expect("panel menu has a Sort order item").submenu;
            sub.iter()
                .filter(|it| it.shortcut.starts_with('✓'))
                .map(|it| (it.label.clone(), it.shortcut))
                .collect()
        };
        assert_eq!(
            ticks(0),
            vec![("Sort: &Name".to_string(), "✓"), ("&Directories first".to_string(), "✓")]
        );
        assert_eq!(
            ticks(4),
            vec![
                ("Sort: &Change time".to_string(), "✓"),
                ("&Reverse order".to_string(), "✓ Ctrl-E")
            ]
        );
    }

    /// Index of the File menu's Git parent item.
    fn git_idx(m: &MenuBarState) -> usize {
        m.menus[1]
            .items
            .iter()
            .position(|it| matches!(it.action, MenuAction::GitMenu))
            .expect("the File menu has a Git item")
    }

    #[test]
    fn git_parent_opens_a_submenu_rather_than_acting() {
        // Its accelerator reveals the submenu instead of firing GitMenu.
        let mut m = MenuBarState::new(1, &[], [false, false]);
        assert!(matches!(m.handle_key(key('g')), MenuSignal::Stay));
        assert!(m.sub_open, "'g' opens the Git submenu");
        assert_eq!(m.item, git_idx(&m), "and highlights its parent");

        // Enter on the parent opens it too, as does →.
        for opener in [KeyCode::Enter, KeyCode::Right] {
            let mut m = MenuBarState::new(1, &[], [false, false]);
            m.item = git_idx(&m);
            assert!(matches!(m.handle_key(key_code(opener)), MenuSignal::Stay));
            assert!(m.sub_open, "{opener:?} opens the submenu");
        }
    }

    #[test]
    fn submenu_navigates_activates_and_steps_back() {
        let mut m = MenuBarState::new_git(&[], [false, false]);
        assert!(m.sub_open, "Alt-G opens straight into the Git submenu");
        // It lands on the first selectable row: Status.
        assert!(matches!(m.handle_key(key(' ')), MenuSignal::Stay)); // unclaimed: no-op
        assert!(m.sub_open, "an unclaimed letter must not close it or switch menus");

        // A submenu accelerator activates its own item.
        let mut m = MenuBarState::new_git(&[], [false, false]);
        assert!(matches!(m.handle_key(key('l')), MenuSignal::Activate(MenuAction::GitLog)));
        // ...even though 'l' is "Send over LAN" in the parent menu and "Left" on
        // the top bar — the submenu's own accelerators win while it is open.

        // Enter activates the highlighted row.
        let mut m = MenuBarState::new_git(&[], [false, false]);
        assert!(matches!(
            m.handle_key(key_code(KeyCode::Enter)),
            MenuSignal::Activate(MenuAction::GitStatus)
        ));

        // ↓ moves within the submenu, skipping separators.
        let mut m = MenuBarState::new_git(&[], [false, false]);
        m.handle_key(key_code(KeyCode::Down));
        assert!(matches!(
            m.handle_key(key_code(KeyCode::Enter)),
            MenuSignal::Activate(MenuAction::GitLog)
        ));

        // Esc / ← close only the submenu, leaving the File menu open.
        for back in [KeyCode::Esc, KeyCode::Left] {
            let mut m = MenuBarState::new_git(&[], [false, false]);
            assert!(matches!(m.handle_key(key_code(back)), MenuSignal::Stay));
            assert!(!m.sub_open, "{back:?} steps back to the parent");
            assert_eq!(m.active, 1, "and stays in the File menu");
        }
        // F10 still closes the whole bar from inside a submenu.
        let mut m = MenuBarState::new_git(&[], [false, false]);
        assert!(matches!(m.handle_key(key_code(KeyCode::F(10))), MenuSignal::Close));
    }

    #[test]
    fn git_submenu_accelerators_are_unique() {
        let m = MenuBarState::new(1, &[], [false, false]);
        let sub = &m.menus[1].items[git_idx(&m)].submenu;
        let mut seen = std::collections::HashSet::new();
        for it in sub.iter().filter(|it| it.selectable()) {
            let hk = it.hotkey().expect("every git item marks an accelerator");
            assert!(seen.insert(hk), "duplicate accelerator '{hk}' in the Git submenu");
        }
        // The submenu covers every action the palette offers.
        assert_eq!(sub.iter().filter(|it| it.selectable()).count(), GIT_MENU_KEYS.len());
    }

    #[test]
    fn top_bar_letter_switches_menu_when_unclaimed() {
        // The Options menu claims none of L/F/R, so those letters fall through to
        // the top bar: 'f' → File (1), 'l' → Left (0).
        let mut m = MenuBarState::new(3, &[], [false, false]);
        assert!(matches!(m.handle_key(key('f')), MenuSignal::Stay));
        assert_eq!(m.active, 1);
        let mut m = MenuBarState::new(3, &[], [false, false]);
        assert!(matches!(m.handle_key(key('l')), MenuSignal::Stay));
        assert_eq!(m.active, 0);
        // An item accelerator still wins over a top letter: in File 'c' is Copy
        // (not "Command") and 'l' is "Send over LAN" (not the Left menu). In
        // Options nothing claims 'c', so it moves on to the Command menu.
        let mut m = MenuBarState::new(1, &[], [false, false]);
        assert!(matches!(m.handle_key(key('c')), MenuSignal::Activate(MenuAction::Copy)));
        let mut m = MenuBarState::new(1, &[], [false, false]);
        assert!(matches!(m.handle_key(key('l')), MenuSignal::Activate(MenuAction::SendFile)));
        let mut m = MenuBarState::new(3, &[], [false, false]);
        assert!(matches!(m.handle_key(key('c')), MenuSignal::Stay));
        assert_eq!(m.active, 2);
    }
}
