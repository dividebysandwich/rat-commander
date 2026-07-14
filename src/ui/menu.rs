//! Interactive pulldown menu bar (F9 / F2).

use crate::panel::sort::SortKey;
use crate::panel::ViewFormat;
use crate::ui::menubar::titles;
use crate::vfs::remote::Protocol;
use crate::ui::theme::Theme;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

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
    /// Open the list of running background transfers.
    BackgroundOps,
    SelectGroup,
    UnselectGroup,
    Invert,
    SetFormat(usize, ViewFormat),
    SetSort(usize, SortKey),
    ToggleReverse(usize),
    SwapPanels,
    Refresh,
    ToggleSplit,
    FindFile,
    /// Mark files identical between the left and right panel directories.
    FindDuplicates,
    ProcExplorer,
    DiskExplorer,
    DiskManager,
    /// Open the network-connections explorer (Linux only).
    NetworkConnections,
    CompareDirs,
    CompareFiles,
    Connect(usize, Protocol),
    Disconnect(usize),
    /// Switch a panel (side) to an already-open remote session by id.
    SwitchSession(usize, usize),
    /// Disconnect (with confirmation) the remote session with this id.
    DisconnectSession(usize),
    /// Open the drive-letter picker for a panel (Windows).
    Drive(usize),
    Settings,
    Confirmations,
    /// Open `themes.toml` in the internal editor.
    EditThemes,
    /// Open `rc.ext` (file associations) in the internal editor.
    EditExtensions,
    /// Open the F2 user `menu` file in the internal editor.
    EditUserMenu,
    Quit,
}

impl MenuAction {
    fn selectable(self) -> bool {
        !matches!(self, MenuAction::Separator)
    }
}

struct MenuItem {
    /// The item's label in the active language (translated when built).
    label: String,
    /// Optional keyboard-shortcut hint, drawn right-aligned in the dropdown
    /// (e.g. `"F3"`, `"Shift-F6"`). Empty for items without a shortcut.
    shortcut: &'static str,
    action: MenuAction,
    /// When false the item is greyed out and cannot be selected or activated
    /// (e.g. "Go local" while the panel is already on a local directory).
    enabled: bool,
}

impl MenuItem {
    /// Grey this item out so it can't be navigated to or activated. Used for
    /// context-dependent items that don't apply in the current state.
    fn disabled(mut self, disabled: bool) -> Self {
        self.enabled = !disabled;
        self
    }

    /// Whether the item can be selected/activated: not a separator, and enabled.
    fn selectable(&self) -> bool {
        self.enabled && self.action.selectable()
    }
}

struct Menu {
    items: Vec<MenuItem>,
}

/// Result of a key press routed to the menu.
pub enum MenuSignal {
    Stay,
    Close,
    Activate(MenuAction),
}

pub struct MenuBarState {
    menus: Vec<Menu>,
    active: usize,
    item: usize,
    /// Screen rect of each top-bar title, recorded at render time.
    title_rects: Vec<Rect>,
    /// Screen rect of each dropdown item (with its item index).
    item_rects: Vec<(usize, Rect)>,
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
                sep(),
                item("Sort: &Name", MenuAction::SetSort(side, SortKey::Name)),
                item("Sort: &Extension", MenuAction::SetSort(side, SortKey::Extension)),
                item("Sort: &Size", MenuAction::SetSort(side, SortKey::Size)),
                item("Sort: &Modify time", MenuAction::SetSort(side, SortKey::ModifyTime)),
                item("Sort: &Unsorted", MenuAction::SetSort(side, SortKey::Unsorted)),
                sep(),
                item("&Reverse order", MenuAction::ToggleReverse(side)),
                sep(),
                item("SFT&P connection...", MenuAction::Connect(side, Protocol::Sftp)),
                item("F&TP connection...", MenuAction::Connect(side, Protocol::Ftp)),
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
                let label = if side == 0 {
                    "&Drive...      Alt-F1"
                } else {
                    "&Drive...      Alt-F2"
                };
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
                sep(),
                item("&Background operations...", MenuAction::BackgroundOps),
                sep(),
                item_key("Select &group", "+", MenuAction::SelectGroup),
                item_key("U&nselect group", "-", MenuAction::UnselectGroup),
                item_key("&Invert selection", "*", MenuAction::Invert),
                sep(),
                item_key("&Quit", "F10", MenuAction::Quit),
            ],
        };

        let mut command_items = vec![
            item("&Find file...", MenuAction::FindFile),
            item("Find d&uplicates...", MenuAction::FindDuplicates),
            item("Compare &directories...", MenuAction::CompareDirs),
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
                item("&Confirmations...", MenuAction::Confirmations),
                item("&Edit themes...", MenuAction::EditThemes),
                item("Edit e&xtensions...", MenuAction::EditExtensions),
                item("Edit &menu file...", MenuAction::EditUserMenu),
            ],
        };

        let active = active.min(4);
        MenuBarState {
            menus: vec![panel_menu(0), file, command, options, panel_menu(1)],
            active,
            item: 0,
            title_rects: Vec::new(),
            item_rects: Vec::new(),
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> MenuSignal {
        match key.code {
            KeyCode::Esc | KeyCode::F(9) | KeyCode::F(10) => MenuSignal::Close,
            KeyCode::Left => {
                self.active = (self.active + self.menus.len() - 1) % self.menus.len();
                self.item = self.first_selectable(0, 1);
                MenuSignal::Stay
            }
            KeyCode::Right => {
                self.active = (self.active + 1) % self.menus.len();
                self.item = self.first_selectable(0, 1);
                MenuSignal::Stay
            }
            KeyCode::Up => {
                self.item = self.next_selectable(self.item, -1);
                MenuSignal::Stay
            }
            KeyCode::Down => {
                self.item = self.next_selectable(self.item, 1);
                MenuSignal::Stay
            }
            KeyCode::Enter => {
                let it = &self.menus[self.active].items[self.item];
                if it.selectable() {
                    MenuSignal::Activate(it.action)
                } else {
                    MenuSignal::Stay
                }
            }
            KeyCode::Char(c) => self.activate_hotkey(c),
            _ => MenuSignal::Stay,
        }
    }

    /// Handle a typed letter: an accelerator in the open dropdown activates that
    /// item; otherwise a top-bar letter (L/F/C/O/R) switches to that menu.
    fn activate_hotkey(&mut self, c: char) -> MenuSignal {
        let lc = c.to_ascii_lowercase();
        if let Some(idx) = self.menus[self.active]
            .items
            .iter()
            .position(|it| it.selectable() && it.hotkey() == Some(lc))
        {
            self.item = idx;
            return MenuSignal::Activate(self.menus[self.active].items[idx].action);
        }
        if let Some(ti) = titles()
            .iter()
            .position(|t| t.chars().next().map(|x| x.to_ascii_lowercase()) == Some(lc))
        {
            self.active = ti;
            self.item = self.first_selectable(0, 1);
            return MenuSignal::Stay;
        }
        MenuSignal::Stay
    }

    /// First selectable item at or after `start`, scanning by `dir`.
    fn first_selectable(&self, start: usize, dir: isize) -> usize {
        let items = &self.menus[self.active].items;
        let mut i = start;
        for _ in 0..items.len() {
            if items[i].selectable() {
                return i;
            }
            i = (i as isize + dir).rem_euclid(items.len() as isize) as usize;
        }
        start
    }

    fn next_selectable(&self, from: usize, dir: isize) -> usize {
        let items = &self.menus[self.active].items;
        let n = items.len() as isize;
        let mut i = (from as isize + dir).rem_euclid(n);
        for _ in 0..items.len() {
            if items[i as usize].selectable() {
                return i as usize;
            }
            i = (i + dir).rem_euclid(n);
        }
        from
    }

    /// The top-bar title index at screen column `col` on the menu-bar row, or
    /// `None`. Mirrors the title layout used by `render`/`menubar::render` so it
    /// works even before the bar has been drawn (i.e. to open it on click).
    pub fn title_index_at(area: Rect, col: u16, row: u16) -> Option<usize> {
        if row != area.y {
            return None;
        }
        let mut x = area.x + 1;
        for (i, title) in titles().iter().enumerate() {
            let w = title.chars().count() as u16 + 2; // " {title} "
            if col >= x && col < x + w {
                return Some(i);
            }
            x += w;
        }
        None
    }

    /// Route a left-click to the menu (titles switch/open; items activate;
    /// anything else closes).
    pub fn click(&mut self, area: Rect, col: u16, row: u16) -> MenuSignal {
        // A click on a top-bar title switches to that menu.
        if let Some(i) = Self::title_index_at(area, col, row) {
            self.active = i;
            self.item = self.first_selectable(0, 1);
            return MenuSignal::Stay;
        }
        // A click on a dropdown item activates it.
        for (idx, rect) in &self.item_rects {
            if col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
            {
                let it = &self.menus[self.active].items[*idx];
                if it.selectable() {
                    return MenuSignal::Activate(it.action);
                }
                return MenuSignal::Stay;
            }
        }
        MenuSignal::Close
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        self.title_rects.clear();
        self.item_rects.clear();
        // Top bar with the active title highlighted.
        let bar = Rect { height: 1, ..area };
        let mut spans: Vec<Span> = vec![Span::styled(" ", theme.menubar)];
        let rtl = crate::l10n::active_is_rtl();
        let mut title_x = vec![];
        let mut x = area.x + 1;
        for (i, title) in titles().iter().enumerate() {
            let text = format!(" {} ", crate::l10n::display(title));
            title_x.push(x);
            self.title_rects.push(Rect {
                x,
                y: area.y,
                width: text.chars().count() as u16,
                height: 1,
            });
            let style = if i == self.active {
                Style::default()
                    .bg(theme.dialog_bg)
                    .fg(theme.dialog_fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                theme.menubar
            };
            x += text.chars().count() as u16;
            // The title's first letter (after the leading space) is its hotkey
            // (skipped in RTL, where the reshaped title reads right-to-left).
            let hk = if rtl { None } else { Some(1) };
            spans.extend(label_spans(&text, hk, style, theme).spans);
        }
        f.render_widget(Paragraph::new(Line::from(spans)), bar);

        // Dropdown under the active title.
        let menu = &self.menus[self.active];
        let width = menu
            .items
            .iter()
            .map(|it| {
                let disp = it.label.chars().filter(|&c| c != '&').count();
                if it.shortcut.is_empty() {
                    disp
                } else {
                    // label + a 2-space gap + the right-aligned shortcut
                    disp + 2 + it.shortcut.chars().count()
                }
            })
            .max()
            .unwrap_or(8) as u16
            + 4;
        let height = menu.items.len() as u16 + 2;
        let dx = title_x[self.active].min(area.x + area.width.saturating_sub(width));
        let rect = Rect {
            x: dx,
            y: area.y + 1,
            width: width.min(area.width),
            height: height.min(area.height.saturating_sub(1)),
        };
        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .style(Style::default().fg(theme.menu_fg).bg(theme.menu_bg));
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let mut lines: Vec<Line> = Vec::with_capacity(menu.items.len());
        for (i, it) in menu.items.iter().enumerate() {
            let row_y = inner.y + i as u16;
            if matches!(it.action, MenuAction::Separator) {
                lines.push(Line::from(Span::styled(
                    "─".repeat(inner.width as usize),
                    Style::default().fg(theme.panel_border).bg(theme.menu_bg),
                )));
                continue;
            }
            if row_y < inner.y + inner.height {
                self.item_rects.push((
                    i,
                    Rect { x: inner.x, y: row_y, width: inner.width, height: 1 },
                ));
            }
            let style = if !it.enabled {
                // Greyed out: dimmed foreground, never the selection highlight
                // (navigation skips disabled items so `i` never lands here).
                Style::default().fg(theme.panel_border).bg(theme.menu_bg)
            } else if i == self.item {
                theme.menu_selection
            } else {
                Style::default().fg(theme.menu_fg).bg(theme.menu_bg)
            };
            let (display, hk) = split_hotkey(&it.label);
            // Reshape RTL text for display; in that case the hotkey accent can't
            // line up with the reversed text, so it is dropped (the key still works).
            let display = crate::l10n::display(&display);
            // Disabled items don't accent their accelerator (it's inactive).
            let hk = if rtl || !it.enabled { None } else { hk.map(|i| i + 1) };
            let iw = inner.width as usize;
            let mut text = format!(" {display}");
            // Right-align the shortcut hint (one trailing space from the edge),
            // then pad the row out to the full interior width.
            if !it.shortcut.is_empty() {
                let sc_start = iw.saturating_sub(it.shortcut.chars().count() + 1);
                while text.chars().count() < sc_start {
                    text.push(' ');
                }
                text.push_str(it.shortcut);
            }
            while text.chars().count() < iw {
                text.push(' ');
            }
            // The hotkey sits one column right of its index (the leading space).
            lines.push(label_spans(&text, hk, style, theme));
        }
        f.render_widget(Paragraph::new(lines), inner);
    }
}

impl Default for MenuBarState {
    fn default() -> Self {
        Self::new(1, &[], [false, false])
    }
}

fn item(label: &str, action: MenuAction) -> MenuItem {
    MenuItem { label: crate::l10n::tr(label), shortcut: "", action, enabled: true }
}

/// A menu item whose label is used verbatim (no translation, no `&` hotkey) —
/// for runtime text like a remote-connection label.
fn item_raw(label: String, action: MenuAction) -> MenuItem {
    MenuItem { label, shortcut: "", action, enabled: true }
}

/// A menu item with a right-aligned keyboard-shortcut hint.
fn item_key(label: &str, shortcut: &'static str, action: MenuAction) -> MenuItem {
    MenuItem { label: crate::l10n::tr(label), shortcut, action, enabled: true }
}

fn sep() -> MenuItem {
    MenuItem {
        label: String::new(),
        shortcut: "",
        action: MenuAction::Separator,
        enabled: true,
    }
}

impl MenuItem {
    /// The lower-cased accelerator key for this item, if its label marks one
    /// with `&` (e.g. `"&Copy"` → `'c'`, `"Select &group"` → `'g'`).
    fn hotkey(&self) -> Option<char> {
        let (display, idx) = split_hotkey(&self.label);
        idx.and_then(|i| display.chars().nth(i)).map(|c| c.to_ascii_lowercase())
    }
}

/// Strip the `&` accelerator marker from `label`, returning the display text and
/// the char index (within that text) of the highlighted hotkey, if any.
fn split_hotkey(label: &str) -> (String, Option<usize>) {
    match label.find('&') {
        Some(byte_pos) => {
            let idx = label[..byte_pos].chars().count();
            let display: String = label.chars().filter(|&c| c != '&').collect();
            (display, Some(idx))
        }
        None => (label.to_string(), None),
    }
}

/// Render `text` with the char at `pos` painted in the hotkey accent color.
fn label_spans(text: &str, pos: Option<usize>, base: Style, theme: &Theme) -> Line<'static> {
    let chars: Vec<char> = text.chars().collect();
    match pos {
        Some(p) if p < chars.len() => {
            let hot = base.fg(theme.hotkey_fg).add_modifier(Modifier::BOLD);
            Line::from(vec![
                Span::styled(chars[..p].iter().collect::<String>(), base),
                Span::styled(chars[p].to_string(), hot),
                Span::styled(chars[p + 1..].iter().collect::<String>(), base),
            ])
        }
        _ => Line::from(Span::styled(text.to_string(), base)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
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
        assert!(matches!(hk('g'), Some(MenuAction::SelectGroup)));
        assert!(matches!(hk('n'), Some(MenuAction::UnselectGroup)));
        assert!(matches!(hk('i'), Some(MenuAction::Invert)));
        assert!(matches!(hk('q'), Some(MenuAction::Quit)));
        // The new Checksum entry is accelerated by 'k' (unique in the File menu).
        assert!(matches!(hk('k'), Some(MenuAction::Checksum)));
    }

    #[test]
    fn typing_an_item_hotkey_activates_it() {
        // File menu: 'c' → Copy, 'g' → Select group.
        let mut m = MenuBarState::new(1, &[], [false, false]);
        assert!(matches!(m.handle_key(key('c')), MenuSignal::Activate(MenuAction::Copy)));
        let mut m = MenuBarState::new(1, &[], [false, false]);
        assert!(matches!(m.handle_key(key('g')), MenuSignal::Activate(MenuAction::SelectGroup)));
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
                items
                    .iter()
                    .any(|it| matches!(it.action, MenuAction::DisconnectSession(7))),
                "menu {mi} should offer disconnecting session 7"
            );
        }
        // With no sessions, no such items appear.
        let m = MenuBarState::new(0, &[], [true, true]);
        assert!(!m.menus[0]
            .items
            .iter()
            .any(|it| matches!(it.action, MenuAction::SwitchSession(..))));
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
    fn top_bar_letter_switches_menu_when_unclaimed() {
        // From the File menu, 'o' has no item → switches to Options (index 3),
        // and 'l' switches to the Left panel menu (index 0).
        let mut m = MenuBarState::new(1, &[], [false, false]);
        assert!(matches!(m.handle_key(key('o')), MenuSignal::Stay));
        assert_eq!(m.active, 3);
        let mut m = MenuBarState::new(1, &[], [false, false]);
        assert!(matches!(m.handle_key(key('l')), MenuSignal::Stay));
        assert_eq!(m.active, 0);
        // An item accelerator still wins over a top letter: 'c' in File is Copy,
        // not "Command".
        let mut m = MenuBarState::new(1, &[], [false, false]);
        assert!(matches!(m.handle_key(key('c')), MenuSignal::Activate(MenuAction::Copy)));
    }
}

