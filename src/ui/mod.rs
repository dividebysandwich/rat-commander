//! UI rendering: the root `draw` plus the chrome widgets.

pub mod cmdline;
pub mod dialog;
pub mod fkeys;
pub mod layout;
pub mod menubar;
pub mod theme;

use crate::app::state::AppState;
use crate::panel::render::render_panel;
use layout::SplitDir;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Render the entire UI for one frame.
pub fn draw(f: &mut Frame, state: &mut AppState) {
    let theme = state.theme.clone();
    let area = f.area();

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // menu bar
            Constraint::Min(1),    // panels
            Constraint::Length(1), // command line
            Constraint::Length(1), // function keys
        ])
        .split(area);

    menubar::render(f, rows[0], &theme);

    let (left_area, right_area) = split_body(rows[1], state.split);
    let active = state.active;
    render_panel(f, left_area, &mut state.panels[0], active == 0, &theme);
    render_panel(f, right_area, &mut state.panels[1], active == 1, &theme);

    let cwd = state.panels[active].cwd.display();
    let caret = cmdline::render(f, rows[2], &state.cmd, &cwd, &theme);

    fkeys::render(f, rows[3], &fkeys::PANEL_LABELS, &theme);

    if let Some(d) = &state.dialog {
        d.render(f, area, &theme);
    } else {
        f.set_cursor_position(caret);
    }
}

/// Split the body area into (first-panel, second-panel) according to `split`.
fn split_body(area: Rect, split: SplitDir) -> (Rect, Rect) {
    let dir = match split {
        SplitDir::Vertical => Direction::Horizontal,
        SplitDir::Horizontal => Direction::Vertical,
    };
    let parts = Layout::default()
        .direction(dir)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    (parts[0], parts[1])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::AppState;
    use crate::util::async_bridge;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    fn buffer_text(buf: &Buffer) -> String {
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    #[tokio::test]
    async fn renders_chrome_and_columns() {
        let (tx, _rx) = async_bridge::channel();
        let mut state = AppState::new(tx);
        state.init().await;

        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut state)).unwrap();
        let text = buffer_text(terminal.backend().buffer());

        // Menu bar, panel header columns, and the function-key row.
        for needle in [
            "File", "Options", "Right", // menu bar
            "Name", "Size", "Modify time", // full-format header
            "Help", "Copy", "Delete", "Quit", // F-key row
        ] {
            assert!(text.contains(needle), "expected UI to contain {needle:?}");
        }
    }
}
