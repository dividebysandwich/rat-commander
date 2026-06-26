//! Color theme — the classic Midnight Commander blue scheme.

use ratatui::style::{Color, Modifier, Style};

/// Centralized styles so the look can be tweaked in one place (and themed in a
/// later phase).
#[derive(Clone)]
pub struct Theme {
    pub panel_bg: Color,
    pub panel_fg: Color,
    pub panel_border: Color,
    pub panel_border_active: Color,
    pub header_fg: Color,
    pub cursor: Style,
    pub cursor_inactive: Style,
    pub marked_fg: Color,
    pub dir_fg: Color,
    pub exec_fg: Color,
    pub symlink_fg: Color,
    pub menubar: Style,
    pub fkey_label: Style,
    pub fkey_num: Style,
    pub dialog_bg: Color,
    pub dialog_fg: Color,
    pub button: Style,
    pub button_focused: Style,
    pub error_fg: Color,
}

impl Theme {
    pub fn mc() -> Self {
        Theme {
            panel_bg: Color::Blue,
            panel_fg: Color::Gray,
            panel_border: Color::Cyan,
            panel_border_active: Color::White,
            header_fg: Color::Yellow,
            cursor: Style::default()
                .bg(Color::Cyan)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
            cursor_inactive: Style::default().bg(Color::Blue).fg(Color::White),
            marked_fg: Color::Yellow,
            dir_fg: Color::White,
            exec_fg: Color::Green,
            symlink_fg: Color::Cyan,
            menubar: Style::default().bg(Color::Cyan).fg(Color::Black),
            fkey_label: Style::default().bg(Color::Cyan).fg(Color::Black),
            fkey_num: Style::default().bg(Color::Black).fg(Color::White),
            dialog_bg: Color::Gray,
            dialog_fg: Color::Black,
            button: Style::default().bg(Color::Gray).fg(Color::Black),
            button_focused: Style::default()
                .bg(Color::Black)
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            error_fg: Color::Red,
        }
    }

    /// Base style for panel content (background + default foreground).
    pub fn panel_base(&self) -> Style {
        Style::default().bg(self.panel_bg).fg(self.panel_fg)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::mc()
    }
}
