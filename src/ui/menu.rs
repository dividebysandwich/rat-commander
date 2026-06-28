//! Interactive pulldown menu bar (F9 / F2).

use crate::panel::sort::SortKey;
use crate::panel::ViewFormat;
use crate::ui::menubar::TITLES;
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
    Mkdir,
    Delete,
    Chmod,
    Chown,
    Symlink,
    Compress,
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
    ProcExplorer,
    DiskExplorer,
    CompareDirs,
    CompareFiles,
    Connect(usize, Protocol),
    Disconnect(usize),
    Settings,
    Quit,
}

impl MenuAction {
    fn selectable(self) -> bool {
        !matches!(self, MenuAction::Separator)
    }
}

struct MenuItem {
    label: &'static str,
    action: MenuAction,
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
    pub fn new(active: usize) -> Self {
        let panel_menu = |side: usize| Menu {
            items: vec![
                item("Full view", MenuAction::SetFormat(side, ViewFormat::Full)),
                item("Brief view", MenuAction::SetFormat(side, ViewFormat::Brief)),
                sep(),
                item("Sort: Name", MenuAction::SetSort(side, SortKey::Name)),
                item("Sort: Extension", MenuAction::SetSort(side, SortKey::Extension)),
                item("Sort: Size", MenuAction::SetSort(side, SortKey::Size)),
                item("Sort: Modify time", MenuAction::SetSort(side, SortKey::ModifyTime)),
                item("Sort: Unsorted", MenuAction::SetSort(side, SortKey::Unsorted)),
                sep(),
                item("Reverse order", MenuAction::ToggleReverse(side)),
                sep(),
                item("SFTP connection...", MenuAction::Connect(side, Protocol::Sftp)),
                item("FTP connection...", MenuAction::Connect(side, Protocol::Ftp)),
                item("SCP connection...", MenuAction::Connect(side, Protocol::Scp)),
                item("Disconnect (local)", MenuAction::Disconnect(side)),
            ],
        };

        let file = Menu {
            items: vec![
                item("View          F3", MenuAction::View),
                item("Edit          F4", MenuAction::Edit),
                item("Copy          F5", MenuAction::Copy),
                item("Rename/Move   F6", MenuAction::Move),
                item("Make directory F7", MenuAction::Mkdir),
                item("Delete        F8", MenuAction::Delete),
                sep(),
                item("Chmod", MenuAction::Chmod),
                item("Chown", MenuAction::Chown),
                item("Symlink", MenuAction::Symlink),
                sep(),
                item("Compress...", MenuAction::Compress),
                sep(),
                item("Select group   +", MenuAction::SelectGroup),
                item("Unselect group -", MenuAction::UnselectGroup),
                item("Invert selection *", MenuAction::Invert),
                sep(),
                item("Quit          F10", MenuAction::Quit),
            ],
        };

        let command = Menu {
            items: vec![
                item("Find file...", MenuAction::FindFile),
                item("Compare directories...", MenuAction::CompareDirs),
                item("Compare files...", MenuAction::CompareFiles),
                item("Process explorer...", MenuAction::ProcExplorer),
                item("Disk explorer...", MenuAction::DiskExplorer),
                sep(),
                item("Swap panels", MenuAction::SwapPanels),
                item("Re-read directories", MenuAction::Refresh),
                item("Toggle split V/H", MenuAction::ToggleSplit),
            ],
        };

        let options = Menu {
            items: vec![item("Settings...", MenuAction::Settings)],
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
            KeyCode::Esc | KeyCode::F(9) => MenuSignal::Close,
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
                let action = self.menus[self.active].items[self.item].action;
                if action.selectable() {
                    MenuSignal::Activate(action)
                } else {
                    MenuSignal::Stay
                }
            }
            _ => MenuSignal::Stay,
        }
    }

    /// First selectable item at or after `start`, scanning by `dir`.
    fn first_selectable(&self, start: usize, dir: isize) -> usize {
        let items = &self.menus[self.active].items;
        let mut i = start;
        for _ in 0..items.len() {
            if items[i].action.selectable() {
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
            if items[i as usize].action.selectable() {
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
        for (i, title) in TITLES.iter().enumerate() {
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
                let action = self.menus[self.active].items[*idx].action;
                if action.selectable() {
                    return MenuSignal::Activate(action);
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
        let mut title_x = vec![];
        let mut x = area.x + 1;
        for (i, title) in TITLES.iter().enumerate() {
            let text = format!(" {title} ");
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
            spans.push(Span::styled(text, style));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), bar);

        // Dropdown under the active title.
        let menu = &self.menus[self.active];
        let width = menu
            .items
            .iter()
            .map(|it| it.label.chars().count())
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
            let style = if i == self.item {
                theme.menu_selection
            } else {
                Style::default().fg(theme.menu_fg).bg(theme.menu_bg)
            };
            let mut label = format!(" {} ", it.label);
            let target = inner.width as usize;
            while label.chars().count() < target {
                label.push(' ');
            }
            lines.push(Line::from(Span::styled(label, style)));
        }
        f.render_widget(Paragraph::new(lines), inner);
    }
}

impl Default for MenuBarState {
    fn default() -> Self {
        Self::new(1)
    }
}

fn item(label: &'static str, action: MenuAction) -> MenuItem {
    MenuItem { label, action }
}

fn sep() -> MenuItem {
    MenuItem {
        label: "",
        action: MenuAction::Separator,
    }
}
