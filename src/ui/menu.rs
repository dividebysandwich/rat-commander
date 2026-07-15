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
    /// Share the selected file(s) with a nearby device over the LAN (QR code).
    SendFile,
    /// Opens the Git submenu (File → Git, or Alt-G). Never runs an action itself.
    GitMenu,
    /// Stage / unstage the file(s) under the cursor (git) — the Ctrl-G toggle.
    GitStage,
    /// Open the side-by-side diff of the file under the cursor against HEAD.
    GitDiff,
    /// `git status` of the panel's repository, shown as raw output.
    GitStatus,
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
    /// Open the fuzzy command palette (Ctrl-P).
    CommandPalette,
    /// Open the directory hotlist / bookmarks (Ctrl-\).
    Hotlist,
    /// Set the active panel's persistent listing filter.
    PanelFilter,
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
    /// Nested items opened as a submenu beside the dropdown. Empty for a normal
    /// (leaf) item; a non-empty submenu makes the item open it rather than
    /// activate its own action.
    submenu: Vec<MenuItem>,
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

    /// Whether opening this item reveals a submenu rather than running an action.
    fn has_sub(&self) -> bool {
        !self.submenu.is_empty()
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
    /// Whether the highlighted item's submenu is open (only items with a
    /// non-empty `submenu` can open one).
    sub_open: bool,
    /// Highlighted row inside the open submenu.
    sub_item: usize,
    /// Screen rect of each top-bar title, recorded at render time.
    title_rects: Vec<Rect>,
    /// Screen rect of each dropdown item (with its item index).
    item_rects: Vec<(usize, Rect)>,
    /// Screen rect of each open-submenu row (with its index), for click routing.
    sub_rects: Vec<(usize, Rect)>,
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
                item("Send over &LAN...", MenuAction::SendFile),
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
            item_key("Panel f&ilter...", "Alt-I", MenuAction::PanelFilter),
            sep(),
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
            sub_open: false,
            sub_item: 0,
            title_rects: Vec::new(),
            item_rects: Vec::new(),
            sub_rects: Vec::new(),
        }
    }

    /// Open the bar straight into the File menu's **Git** submenu (Alt-G).
    pub fn new_git(sessions: &[(usize, String)], side_remote: [bool; 2]) -> Self {
        let mut m = Self::new(1, sessions, side_remote);
        if let Some(idx) = m.menus[1]
            .items
            .iter()
            .position(|it| matches!(it.action, MenuAction::GitMenu))
        {
            m.item = idx;
            m.open_sub();
        }
        m
    }

    /// Open the highlighted item's submenu, landing on its first selectable row.
    /// No-op when the item has no submenu.
    fn open_sub(&mut self) {
        let Some(it) = self.menus[self.active].items.get(self.item) else {
            return;
        };
        if !it.has_sub() {
            return;
        }
        self.sub_item = first_sel(&it.submenu, 0, 1);
        self.sub_open = true;
    }

    /// The open submenu's items, or `None` when no submenu is showing.
    fn sub_items(&self) -> Option<&[MenuItem]> {
        if !self.sub_open {
            return None;
        }
        let it = self.menus[self.active].items.get(self.item)?;
        it.has_sub().then_some(it.submenu.as_slice())
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> MenuSignal {
        // An open submenu captures navigation first; Esc/← step back to its parent
        // rather than closing the whole bar.
        if self.sub_open {
            return self.handle_sub_key(key);
        }
        match key.code {
            KeyCode::Esc | KeyCode::F(9) | KeyCode::F(10) => MenuSignal::Close,
            KeyCode::Left => {
                self.active = (self.active + self.menus.len() - 1) % self.menus.len();
                self.item = self.first_selectable(0, 1);
                MenuSignal::Stay
            }
            // → opens the highlighted item's submenu when it has one, else moves
            // on to the next top menu.
            KeyCode::Right => {
                if self.menus[self.active].items[self.item].has_sub() {
                    self.open_sub();
                } else {
                    self.active = (self.active + 1) % self.menus.len();
                    self.item = self.first_selectable(0, 1);
                }
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
                if !it.selectable() {
                    MenuSignal::Stay
                } else if it.has_sub() {
                    self.open_sub();
                    MenuSignal::Stay
                } else {
                    MenuSignal::Activate(it.action)
                }
            }
            KeyCode::Char(c) => self.activate_hotkey(c),
            _ => MenuSignal::Stay,
        }
    }

    /// Keys while a submenu is open. Esc/← close just the submenu; F9/F10 close
    /// the whole bar; letters match the submenu's own accelerators.
    fn handle_sub_key(&mut self, key: KeyEvent) -> MenuSignal {
        if self.sub_items().is_none() {
            // Defensive: the submenu vanished (shouldn't happen) — drop the flag.
            self.sub_open = false;
            return MenuSignal::Stay;
        }
        // Resolve the move against a borrow, then apply it — `sub_items()` borrows
        // `self`, so nothing may be assigned while it is held.
        let (next, signal) = {
            let items = self.sub_items().expect("checked above");
            match key.code {
                KeyCode::F(9) | KeyCode::F(10) => return MenuSignal::Close,
                KeyCode::Esc | KeyCode::Left => {
                    self.sub_open = false;
                    return MenuSignal::Stay;
                }
                KeyCode::Up => (next_sel(items, self.sub_item, -1), MenuSignal::Stay),
                KeyCode::Down => (next_sel(items, self.sub_item, 1), MenuSignal::Stay),
                KeyCode::Enter => match items.get(self.sub_item) {
                    Some(it) if it.selectable() => (self.sub_item, MenuSignal::Activate(it.action)),
                    _ => return MenuSignal::Stay,
                },
                KeyCode::Char(c) => {
                    let lc = c.to_ascii_lowercase();
                    match items.iter().position(|it| it.selectable() && it.hotkey() == Some(lc)) {
                        Some(idx) => (idx, MenuSignal::Activate(items[idx].action)),
                        // An unclaimed letter does nothing while a submenu is open
                        // — it must not fall through and switch top menus behind it.
                        None => return MenuSignal::Stay,
                    }
                }
                _ => return MenuSignal::Stay,
            }
        };
        self.sub_item = next;
        signal
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
            // A parent's accelerator reveals its submenu instead of acting.
            if self.menus[self.active].items[idx].has_sub() {
                self.open_sub();
                return MenuSignal::Stay;
            }
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
        first_sel(&self.menus[self.active].items, start, dir)
    }

    fn next_selectable(&self, from: usize, dir: isize) -> usize {
        next_sel(&self.menus[self.active].items, from, dir)
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
        let hit = |r: &Rect| col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height;
        // A click on a top-bar title switches to that menu (closing any submenu).
        if let Some(i) = Self::title_index_at(area, col, row) {
            self.active = i;
            self.sub_open = false;
            self.item = self.first_selectable(0, 1);
            return MenuSignal::Stay;
        }
        // An open submenu's rows take precedence: they overlay the dropdown.
        if self.sub_open {
            let picked = self.sub_rects.iter().find(|(_, r)| hit(r)).map(|(i, _)| *i);
            if let Some(idx) = picked {
                let action = self
                    .sub_items()
                    .and_then(|items| items.get(idx))
                    .filter(|it| it.selectable())
                    .map(|it| it.action);
                self.sub_item = idx;
                return match action {
                    Some(a) => MenuSignal::Activate(a),
                    None => MenuSignal::Stay,
                };
            }
        }
        // A click on a dropdown item activates it (or opens its submenu).
        for (idx, rect) in &self.item_rects {
            if hit(rect) {
                let idx = *idx;
                let it = &self.menus[self.active].items[idx];
                if !it.selectable() {
                    return MenuSignal::Stay;
                }
                let (action, has_sub) = (it.action, it.has_sub());
                self.item = idx;
                if has_sub {
                    self.open_sub();
                    return MenuSignal::Stay;
                }
                return MenuSignal::Activate(action);
            }
        }
        MenuSignal::Close
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        self.title_rects.clear();
        self.item_rects.clear();
        self.sub_rects.clear();
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
        let items = &self.menus[self.active].items;
        let width = menu_width(items);
        let height = items.len() as u16 + 2;
        let dx = title_x[self.active].min(area.x + area.width.saturating_sub(width));
        let rect = Rect {
            x: dx,
            y: area.y + 1,
            width: width.min(area.width),
            height: height.min(area.height.saturating_sub(1)),
        };
        let inner = draw_menu_box(f, rect, theme);
        // While a submenu is open the parent row keeps its highlight, so the path
        // through the menu stays visible.
        self.item_rects = draw_items(f, inner, items, self.item, rtl, theme);

        // The submenu, anchored beside its parent row.
        if let Some(sub) = self.sub_items() {
            let sw = menu_width(sub);
            let sh = (sub.len() as u16 + 2).min(area.height.saturating_sub(1));
            // Prefer the right of the parent dropdown; flip to its left when that
            // would run off the screen edge.
            let right = rect.x + rect.width;
            let sx = if right + sw <= area.x + area.width {
                right
            } else {
                rect.x.saturating_sub(sw).max(area.x)
            };
            // Align the box so its first row meets the parent item, then pull it
            // back inside the screen if it would overhang the bottom.
            let parent_y = inner.y + self.item as u16;
            let max_y = (area.y + area.height).saturating_sub(sh);
            let sy = parent_y.saturating_sub(1).min(max_y).max(area.y + 1);
            let srect = Rect { x: sx, y: sy, width: sw.min(area.width), height: sh };
            let sinner = draw_menu_box(f, srect, theme);
            self.sub_rects = draw_items(f, sinner, sub, self.sub_item, rtl, theme);
        }
    }
}

/// Interior width for a dropdown holding `items`: the longest label (plus its
/// right-aligned shortcut) with padding for the border and margins.
fn menu_width(items: &[MenuItem]) -> u16 {
    items
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
        + 4
}

/// Clear `rect`, draw the menu border, and return the interior.
fn draw_menu_box(f: &mut Frame, rect: Rect, theme: &Theme) -> Rect {
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().fg(theme.menu_fg).bg(theme.menu_bg));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    inner
}

/// Draw `items` into `inner` with row `sel` highlighted, returning the
/// `(index, rect)` of every drawn row for click hit-testing.
fn draw_items(
    f: &mut Frame,
    inner: Rect,
    items: &[MenuItem],
    sel: usize,
    rtl: bool,
    theme: &Theme,
) -> Vec<(usize, Rect)> {
    let mut rects = Vec::new();
    let mut lines: Vec<Line> = Vec::with_capacity(items.len());
    for (i, it) in items.iter().enumerate() {
        let row_y = inner.y + i as u16;
        if matches!(it.action, MenuAction::Separator) {
            lines.push(Line::from(Span::styled(
                "─".repeat(inner.width as usize),
                Style::default().fg(theme.panel_border).bg(theme.menu_bg),
            )));
            continue;
        }
        if row_y < inner.y + inner.height {
            rects.push((i, Rect { x: inner.x, y: row_y, width: inner.width, height: 1 }));
        }
        let style = if !it.enabled {
            // Greyed out: dimmed foreground, never the selection highlight
            // (navigation skips disabled items so `i` never lands here).
            Style::default().fg(theme.panel_border).bg(theme.menu_bg)
        } else if i == sel {
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
    rects
}

impl Default for MenuBarState {
    fn default() -> Self {
        Self::new(1, &[], [false, false])
    }
}

/// The Git submenu (File → Git, or Alt-G), newest-to-oldest in workflow order:
/// inspect, stage, commit, exchange with the remote, switch/undo, set up.
/// [`GIT_MENU_KEYS`] mirrors these label keys for the l10n accelerator test and
/// the command palette.
fn git_menu_items() -> Vec<MenuItem> {
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
    ("&Fetch...", MenuAction::GitFetch),
    ("&Pull...", MenuAction::GitPull),
    ("Pus&h...", MenuAction::GitPush),
    ("S&ync (pull + push)", MenuAction::GitSync),
    ("Chec&kout...", MenuAction::GitCheckout),
    ("&Reset...", MenuAction::GitReset),
    ("&Init repository...", MenuAction::GitInit),
    ("Clo&ne...", MenuAction::GitClone),
];

fn item(label: &str, action: MenuAction) -> MenuItem {
    MenuItem { label: crate::l10n::tr(label), shortcut: "", action, enabled: true, submenu: Vec::new() }
}

/// A menu item whose label is used verbatim (no translation, no `&` hotkey) —
/// for runtime text like a remote-connection label.
fn item_raw(label: String, action: MenuAction) -> MenuItem {
    MenuItem { label, shortcut: "", action, enabled: true, submenu: Vec::new() }
}

/// A menu item with a right-aligned keyboard-shortcut hint.
fn item_key(label: &str, shortcut: &'static str, action: MenuAction) -> MenuItem {
    MenuItem { label: crate::l10n::tr(label), shortcut, action, enabled: true, submenu: Vec::new() }
}

/// A parent item that opens `submenu` (with a right-aligned shortcut hint).
fn item_sub(label: &str, shortcut: &'static str, action: MenuAction, submenu: Vec<MenuItem>) -> MenuItem {
    MenuItem { label: crate::l10n::tr(label), shortcut, action, enabled: true, submenu }
}

fn sep() -> MenuItem {
    MenuItem {
        label: String::new(),
        shortcut: "",
        action: MenuAction::Separator,
        enabled: true,
        submenu: Vec::new(),
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

/// First selectable entry of `items` at or after `start`, scanning by `dir`.
/// Falls back to `start` when nothing is selectable.
fn first_sel(items: &[MenuItem], start: usize, dir: isize) -> usize {
    if items.is_empty() {
        return 0;
    }
    let mut i = start.min(items.len() - 1);
    for _ in 0..items.len() {
        if items[i].selectable() {
            return i;
        }
        i = (i as isize + dir).rem_euclid(items.len() as isize) as usize;
    }
    start
}

/// Next selectable entry of `items` from `from`, wrapping, scanning by `dir`.
fn next_sel(items: &[MenuItem], from: usize, dir: isize) -> usize {
    if items.is_empty() {
        return 0;
    }
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
        assert!(matches!(m.handle_key(key_code(KeyCode::Enter)), MenuSignal::Activate(MenuAction::GitStatus)));

        // ↓ moves within the submenu, skipping separators.
        let mut m = MenuBarState::new_git(&[], [false, false]);
        m.handle_key(key_code(KeyCode::Down));
        assert!(matches!(m.handle_key(key_code(KeyCode::Enter)), MenuSignal::Activate(MenuAction::GitLog)));

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
        // (not "Command") and 'l' is "Send over LAN" (not the Left menu); in
        // Options 'c' is Confirmations.
        let mut m = MenuBarState::new(1, &[], [false, false]);
        assert!(matches!(m.handle_key(key('c')), MenuSignal::Activate(MenuAction::Copy)));
        let mut m = MenuBarState::new(1, &[], [false, false]);
        assert!(matches!(m.handle_key(key('l')), MenuSignal::Activate(MenuAction::SendFile)));
        let mut m = MenuBarState::new(3, &[], [false, false]);
        assert!(matches!(m.handle_key(key('c')), MenuSignal::Activate(MenuAction::Confirmations)));
    }
}

