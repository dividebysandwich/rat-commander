//! UI rendering: the root `draw` plus the chrome widgets.

pub mod cmdline;
pub mod dialog;
pub mod fkeys;
pub mod layout;
pub mod menu;
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

    // The editor and viewer take over the entire screen — no menu bar, so the
    // file content uses the full height.
    if let Some(ed) = state.editor.as_mut() {
        crate::editor::render::render(f, area, ed, &theme);
        if let Some(d) = &state.dialog {
            d.render(f, area, &theme);
        }
        return;
    }
    if let Some(v) = state.viewer.as_mut() {
        crate::viewer::render::render(f, area, v, &theme);
        if let Some(d) = &state.dialog {
            d.render(f, area, &theme);
        }
        return;
    }

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

    // Pulldown menu overlays the panels (but sits below modal dialogs).
    if let Some(m) = &state.menu {
        m.render(f, area, &theme);
    }

    if let Some(d) = &state.dialog {
        d.render(f, area, &theme);
    } else if state.menu.is_none() {
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
        // Vertical column separators are drawn.
        assert!(text.contains('│'), "expected vertical column separators");
    }

    #[tokio::test]
    async fn renders_menu_overlay() {
        let (tx, _rx) = async_bridge::channel();
        let mut state = AppState::new(tx);
        state.init().await;
        state.menu = Some(crate::ui::menu::MenuBarState::new(1));

        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut state)).unwrap();
        let text = buffer_text(terminal.backend().buffer());

        for needle in ["View", "Chmod", "Symlink", "Quit"] {
            assert!(text.contains(needle), "menu should contain {needle:?}");
        }
    }

    #[test]
    fn viewer_renders_hex_dump() {
        use crate::viewer::{ViewMode, ViewerState};
        let mut v = ViewerState::new("f.bin".into(), b"AB".to_vec());
        v.mode = ViewMode::Hex;
        let theme = crate::ui::theme::Theme::mc();

        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| crate::viewer::render::render(f, f.area(), &mut v, &theme))
            .unwrap();
        let text = buffer_text(terminal.backend().buffer());

        assert!(text.contains("41 42"), "hex bytes for 'AB'");
        assert!(text.contains("|AB|"), "ascii gutter");
    }

    #[test]
    fn editor_renders_status_and_text() {
        use crate::editor::render::render as ed_render;
        use crate::editor::EditorState;
        use crate::vfs::VfsPath;
        let mut ed = EditorState::new("note.txt".into(), VfsPath::local("/tmp/n"), "hello\nworld");
        let theme = crate::ui::theme::Theme::mc();

        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| ed_render(f, f.area(), &mut ed, &theme))
            .unwrap();
        let text = buffer_text(terminal.backend().buffer());

        assert!(text.contains("note.txt"), "status shows filename");
        assert!(text.contains("Ln 1/2"), "status shows line/total");
        assert!(text.contains("C=104") || text.contains("0x68"), "ASCII code of 'h'");
        assert!(text.contains("hello") && text.contains("world"), "text body");
        assert!(text.contains("Save") && text.contains("Quit"), "shortcut bar");
    }
}




