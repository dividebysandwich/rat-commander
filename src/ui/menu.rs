//! Interactive pulldown menu bar (F9 / F2).

use crate::panel::sort::SortKey;
use crate::panel::ViewFormat;
use crate::ui::menubar::TITLES;
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
}

impl MenuBarState {
    /// Build the standard menu set (Left, File, Command, Options, Right).
    pub fn new() -> Self {
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
                item("Swap panels", MenuAction::SwapPanels),
                item("Re-read directories", MenuAction::Refresh),
                item("Toggle split V/H", MenuAction::ToggleSplit),
            ],
        };

        let options = Menu {
            items: vec![item("Settings...", MenuAction::Settings)],
        };

        MenuBarState {
            menus: vec![panel_menu(0), file, command, options, panel_menu(1)],
            active: 1, // open on "File" by default
            item: 0,
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

    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        // Top bar with the active title highlighted.
        let bar = Rect { height: 1, ..area };
        let mut spans: Vec<Span> = vec![Span::styled(" ", theme.menubar)];
        let mut title_x = vec![];
        let mut x = area.x + 1;
        for (i, title) in TITLES.iter().enumerate() {
            let text = format!(" {title} ");
            title_x.push(x);
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
            .style(Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg));
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let mut lines: Vec<Line> = Vec::with_capacity(menu.items.len());
        for (i, it) in menu.items.iter().enumerate() {
            if matches!(it.action, MenuAction::Separator) {
                lines.push(Line::from(Span::styled(
                    "─".repeat(inner.width as usize),
                    Style::default().fg(theme.panel_border).bg(theme.dialog_bg),
                )));
                continue;
            }
            let style = if i == self.item {
                Style::default()
                    .bg(ratatui::style::Color::Cyan)
                    .fg(theme.dialog_fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg)
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
        Self::new()
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
