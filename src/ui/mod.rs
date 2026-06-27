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
    let mut theme = state.theme.clone();
    theme.anim = state.anim_phase;
    theme.animated = state.config.animation && state.truecolor;
    let area = f.area();
    // Remember the frame area so mouse clicks can be hit-tested next event.
    state.last_area = area;

    // The editor and viewer take over the entire screen — no menu bar, so the
    // file content uses the full height.
    if let Some(ed) = state.editor.as_mut() {
        crate::editor::render::render(f, area, ed, &theme);
        if let Some(d) = &mut state.dialog {
            d.render(f, area, &theme);
        }
        return;
    }
    if let Some(v) = state.viewer.as_mut() {
        crate::viewer::render::render(f, area, v, &theme);
        if let Some(d) = &mut state.dialog {
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

    // System-status widget on the right of the menu bar (wide screens only).
    if state.config.system_status && area.width >= menubar::STATUS_MIN_WIDTH {
        let sw = menubar::STATUS_WIDTH;
        let status_area = Rect {
            x: rows[0].x + rows[0].width - sw,
            y: rows[0].y,
            width: sw,
            height: 1,
        };
        menubar::render_status(f, status_area, &state.sampler, &theme);
    }

    let (left_area, right_area) = split_body(rows[1], state.split);
    let active = state.active;
    render_panel(f, left_area, &mut state.panels[0], active == 0, &theme);
    render_panel(f, right_area, &mut state.panels[1], active == 1, &theme);

    let cwd = state.panels[active].cwd.display();
    let caret = cmdline::render(f, rows[2], &state.cmd, &cwd, &theme);

    fkeys::render(f, rows[3], &fkeys::PANEL_LABELS, &theme);

    // Pulldown menu overlays the panels (but sits below modal dialogs).
    if let Some(m) = &mut state.menu {
        m.render(f, area, &theme);
    }

    if let Some(d) = &mut state.dialog {
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






#[cfg(test)]
mod feature_tests {
    use super::*;
    use crate::app::state::AppState;
    use crate::util::async_bridge;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn text_of(t: &Terminal<TestBackend>) -> String {
        let b = t.backend().buffer();
        let mut s = String::new();
        for y in 0..b.area.height { for x in 0..b.area.width { s.push_str(b[(x,y)].symbol()); } s.push('\n'); }
        s
    }

    #[tokio::test]
    async fn status_widget_shows_on_wide_screen() {
        let (tx,_rx)=async_bridge::channel();
        let mut st=AppState::new(tx);
        st.config.system_status = true;
        st.sampler.sample(); st.sampler.sample();
        st.init().await;
        let mut t=Terminal::new(TestBackend::new(120,24)).unwrap();
        t.draw(|f| draw(f,&mut st)).unwrap();
        let text=text_of(&t);
        assert!(text.contains("CPU"), "status CPU label present");
        assert!(text.contains("MEM"), "status MEM present");
    }

    #[tokio::test]
    async fn status_widget_hidden_on_narrow_screen() {
        let (tx,_rx)=async_bridge::channel();
        let mut st=AppState::new(tx);
        st.config.system_status = true;
        st.init().await;
        let mut t=Terminal::new(TestBackend::new(70,24)).unwrap();
        t.draw(|f| draw(f,&mut st)).unwrap();
        assert!(!text_of(&t).contains("CPU"), "no status on narrow screen");
    }

    #[tokio::test]
    async fn panel_shows_disk_usage_and_separator() {
        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        st.init().await;
        st.panels[0].disk = Some(crate::vfs::DiskUsage { total: 1000, free: 900 });

        let mut t = Terminal::new(TestBackend::new(120, 30)).unwrap();
        t.draw(|f| draw(f, &mut st)).unwrap();
        let text = text_of(&t);
        assert!(text.contains("(10%)"), "disk usage percent on the border");
        assert!(text.contains('├') && text.contains('┤'), "mini-status separator");
    }

    #[test]
    fn overwrite_dialog_renders_all_choices() {
        use crate::ui::dialog::{Dialog, OverwriteDialog};
        use crate::ops::progress::ConflictInfo;
        let info = ConflictInfo {
            id: 1,
            name: "test.wav".into(),
            new_path: "~/test.wav".into(),
            new_size: 2822452,
            new_mtime: None,
            old_path: "~/2/test.wav".into(),
            old_size: 2822452,
            old_mtime: None,
        };
        let mut dlg = Dialog::Overwrite(OverwriteDialog::new(info));
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let theme = crate::ui::theme::Theme::mc();
        t.draw(|f| dlg.render(f, f.area(), &theme)).unwrap();
        let text = text_of(&t);
        for needle in [
            "File exists",
            "Overwrite this file?",
            "Yes",
            "Append",
            "Overwrite all files?",
            "Smaller",
            "Size differs",
            "Abort",
        ] {
            assert!(text.contains(needle), "overwrite dialog should show {needle:?}");
        }
    }

    #[test]
    fn animated_gradient_shifts_with_phase() {
        let mut th = crate::ui::theme::Theme::by_name("Dracula", true);
        th.animated = true;
        th.anim = 0;
        let a = th.gradient_at(5, 20);
        th.anim = 8;
        let b = th.gradient_at(5, 20);
        assert_ne!(a, b, "gradient should move with the animation phase");
    }
}
