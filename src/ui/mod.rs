//! UI rendering: the root `draw` plus the chrome widgets.

pub mod cmdline;
pub mod dialog;
pub mod fkeys;
pub mod graphics;
pub mod hexcolor;
pub mod layout;
pub mod menu;
pub mod menubar;
pub mod textedit;
pub mod theme;
pub mod theme_editor;

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
            d.render(f, area, &theme, state.gfx.as_mut());
        }
        return;
    }
    if let Some(v) = state.viewer.as_mut() {
        // A fullscreen pixel image would repaint over a modal dialog, so hand the
        // viewer graphics only when no dialog is up (it then falls back to
        // half-block art, which composites with the dialog correctly).
        let view_gfx = if state.dialog.is_some() { None } else { state.gfx.as_mut() };
        crate::viewer::render::render(f, area, v, &theme, view_gfx);
        if let Some(d) = &mut state.dialog {
            d.render(f, area, &theme, state.gfx.as_mut());
        }
        return;
    }
    if let Some(pv) = state.procview.as_mut() {
        // The process explorer's graphs animate whenever truecolor is available,
        // regardless of the global animation toggle.
        let mut th = theme.clone();
        th.animated = state.truecolor;
        crate::proc::render::render(f, area, pv, &th, state.gfx.as_mut());
        if let Some(d) = &mut state.dialog {
            d.render(f, area, &theme, state.gfx.as_mut());
        }
        return;
    }
    if let Some(dv) = state.diskview.as_mut() {
        crate::disk::render::render(f, area, dv, &theme, state.gfx.as_mut());
        if let Some(d) = &mut state.dialog {
            d.render(f, area, &theme, state.gfx.as_mut());
        }
        return;
    }
    if let Some(dv) = state.diffview.as_mut() {
        crate::diff::render::render(f, area, dv, &theme);
        if let Some(d) = &mut state.dialog {
            d.render(f, area, &theme, state.gfx.as_mut());
        }
        return;
    }
    if let Some(te) = state.theme_editor.as_mut() {
        theme_editor::render::render(f, area, te, &theme);
        if let Some(d) = &mut state.dialog {
            d.render(f, area, &theme, state.gfx.as_mut());
        }
        return;
    }
    if let Some(mv) = state.mountview.as_mut() {
        crate::mount::render::render(f, area, mv, &theme);
        if let Some(d) = &mut state.dialog {
            d.render(f, area, &theme, state.gfx.as_mut());
        }
        return;
    }
    if let Some(nv) = state.netview.as_mut() {
        // The overview diagram auto-refreshes; its full-width terminal-graphics
        // image would repaint over a modal dialog on the next update (the diff
        // won't re-emit the dialog's unchanged cells). Render the net view without
        // graphics while a dialog is up so the dialog composites correctly.
        let net_gfx = if state.dialog.is_some() { None } else { state.gfx.as_mut() };
        crate::net::render::render(f, area, nv, &theme, net_gfx);
        if let Some(d) = &mut state.dialog {
            d.render(f, area, &theme, state.gfx.as_mut());
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

    // Accelerator letters show while the menu is open, or when Alt arms them
    // (only in classic menu mode — quick search uses Alt for its own input).
    menubar::render(f, rows[0], &theme, state.menu.is_some() || state.alt_hint);

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

    // Mini progress bar for backgrounded transfers, to the left of the status
    // widget (or right-anchored when it's hidden).
    if let Some((done, total, count)) = state.background_summary()
        && let Some(rect) = state.menu_progress_rect(rows[0])
    {
        menubar::render_mini_progress(f, rect, done, total, count, &theme);
    }

    // Half-height mode confines the panels to the top half of the body; the
    // bottom half is left unpainted, exposing the backdrop (Norton-Commander
    // style). Ctrl-F1 / Ctrl-F2 hide the left / right panel entirely; a hidden
    // side yields its space so the visible panel fills the width (and both may
    // be hidden, leaving only the menu and F-key bars).
    let panels_area = if state.half_height {
        Rect { height: rows[1].height.div_ceil(2), ..rows[1] }
    } else {
        rows[1]
    };
    // Each panel keeps its usual half of the split; hiding one simply leaves its
    // half as exposed backdrop rather than growing the surviving panel.
    let (l, r) = split_body(panels_area, state.split);
    let left_area = (!state.panel_hidden[0]).then_some(l);
    let right_area = (!state.panel_hidden[1]).then_some(r);

    // Paint the command-line console across the whole body first; the panels are
    // drawn over it, so it only shows through where a panel is hidden or the
    // half-height mode exposes it (Norton-Commander style). Skipped until the
    // shell has produced output, so the backdrop stays blank until then. The
    // console is sized to the whole terminal (matching the PTY), so its recent
    // lines are anchored to the bottom of the body (see `render_console`).
    if state.console.is_used() {
        render_console(f, rows[1], &state.console);
    }

    let active = state.active;
    let brief_cols = state.config.brief_columns;
    // The quick search renders as an inline input on the active panel's
    // mini-status row (FAR/NC style); only the active panel shows it.
    let left_qs = if active == 0 {
        state.quick_search.as_ref().map(|q| q.query.as_str())
    } else {
        None
    };
    let right_qs = if active == 1 {
        state.quick_search.as_ref().map(|q| q.query.as_str())
    } else {
        None
    };
    // Whether the terminal-graphics layer can draw a pixel-image preview.
    let gfx_on = state.gfx.as_ref().is_some_and(|g| g.available());
    for (i, area_opt, qs) in [(0, left_area, left_qs), (1, right_area, right_qs)] {
        if let Some(pa) = area_opt {
            render_panel(
                f, pa, &mut state.panels[i], active == i, &state.details[i], &theme, brief_cols, qs,
                gfx_on,
            );
        } else {
            // A hidden panel keeps no live geometry, so stray clicks in the
            // freed area can't move an unseen cursor or show a stray caret.
            state.panels[i].hit = None;
            state.panels[i].quick_caret = None;
            state.panels[i].preview_image_area = None;
        }
    }

    // Composite any Details image-thumbnail previews with the graphics layer, now
    // that the panels are laid out. Skipped while a dialog or the menu is up, so a
    // repainted image can't bleed over them (as with the net-view diagram above).
    if state.dialog.is_none() && state.menu.is_none() {
        for i in 0..2 {
            if let Some(area) = state.panels[i].preview_image_area
                && let crate::details::Preview::Image(pi) = &state.details[i].preview
                && let Some(g) = state.gfx.as_mut()
            {
                // Centre the thumbnail in the preview area (aspect-preserved),
                // using the terminal's cell-pixel size to size the target rect.
                let target = crate::util::img::center_rect(area, pi.img.width(), pi.img.height(), g.cell());
                let (sig, img) = (pi.sig, &pi.img);
                g.draw_cached(f, target, crate::ui::graphics::Slot::DetailsPreview(i as u16), sig, || img.clone());
            }
        }
    }

    let cwd = state.console_cwd().display();
    let caret = cmdline::render(f, rows[2], &state.cmd, &cwd, &theme);

    fkeys::render(f, rows[3], &fkeys::panel_labels(), &theme);

    // Pulldown menu overlays the panels (but sits below modal dialogs).
    if let Some(m) = &mut state.menu {
        m.render(f, area, &theme);
    }

    if let Some(d) = &mut state.dialog {
        // A drive/connection picker anchors over its target panel; if that panel
        // is hidden, fall back to centering on the whole screen.
        let darea = match d.anchor_panel() {
            Some(0) => left_area.unwrap_or(area),
            Some(1) => right_area.unwrap_or(area),
            _ => area,
        };
        d.render(f, darea, &theme, state.gfx.as_mut());
    } else if state.menu.is_none() {
        // A live quick search shows its caret on the active panel; otherwise
        // the command line is the editable focus.
        if let Some(qp) = state.panels[active].quick_caret {
            f.set_cursor_position(qp);
        } else {
            f.set_cursor_position(caret);
        }
    }
}

/// Paint the console emulator's screen into `area`, cell for cell, as the
/// backdrop behind the panels. The console is sized to the whole terminal, so
/// its rows are anchored to the *bottom*: the cursor line (the live prompt)
/// lands on the last row of `area` and the rows above it fill upward, so the
/// most recent output sits just above the command line and stays visible even
/// when only the lower part of the backdrop is exposed (half-height mode).
/// Default colours map to `Color::Reset`, so it matches the real terminal.
fn render_console(f: &mut Frame, area: Rect, console: &crate::console::Console) {
    use ratatui::style::{Modifier, Style};
    let parser = console.parser();
    let Ok(parser) = parser.lock() else { return };
    let screen = parser.screen();
    let (rows, cols) = screen.size();
    // Map emulator row -> body row so the cursor line lands on the bottom of the
    // body; rows above fill upward, rows off either end are clipped. Signed,
    // since the emulator (full terminal) is usually taller than the body.
    let offset = area.height as i32 - 1 - screen.cursor_position().0 as i32;
    let buf = f.buffer_mut();
    for r in 0..rows {
        let ty = r as i32 + offset;
        if ty < 0 || ty >= area.height as i32 {
            continue;
        }
        let ty = ty as u16;
        for c in 0..area.width.min(cols) {
            let Some(cell) = screen.cell(r, c) else { continue };
            let mut style = Style::default()
                .fg(vt_color(cell.fgcolor()))
                .bg(vt_color(cell.bgcolor()));
            if cell.bold() {
                style = style.add_modifier(Modifier::BOLD);
            }
            if cell.italic() {
                style = style.add_modifier(Modifier::ITALIC);
            }
            if cell.underline() {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            if cell.inverse() {
                style = style.add_modifier(Modifier::REVERSED);
            }
            let contents = cell.contents();
            let target = &mut buf[(area.x + c, area.y + ty)];
            target.set_symbol(if contents.is_empty() { " " } else { contents });
            target.set_style(style);
        }
    }
}

/// Convert a `vt100` colour to a Ratatui one; the terminal default maps to
/// `Color::Reset` so console output keeps the real terminal's default colours.
fn vt_color(c: vt100::Color) -> ratatui::style::Color {
    use ratatui::style::Color;
    match c {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
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
        // Pin the Full format so the test is independent of any persisted view.
        state.panels[0].format = crate::panel::ViewFormat::Full;
        state.panels[1].format = crate::panel::ViewFormat::Full;
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
        state.menu = Some(crate::ui::menu::MenuBarState::new(1, &[], [false, false]));

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
            .draw(|f| crate::viewer::render::render(f, f.area(), &mut v, &theme, None))
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
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    /// A key press with the Control modifier held (e.g. Ctrl-F1).
    fn key_ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    fn text_of(t: &Terminal<TestBackend>) -> String {
        let b = t.backend().buffer();
        let mut s = String::new();
        for y in 0..b.area.height { for x in 0..b.area.width { s.push_str(b[(x,y)].symbol()); } s.push('\n'); }
        s
    }

    /// A rendered `AppState` at 120x30, driven through one `draw`.
    async fn drawn(state: &mut AppState) -> Terminal<TestBackend> {
        let mut t = Terminal::new(TestBackend::new(120, 30)).unwrap();
        t.draw(|f| draw(f, state)).unwrap();
        t
    }

    /// Each rendered panel contributes exactly one top-left border corner, so
    /// counting them tells how many panels are on screen.
    fn panel_count(t: &Terminal<TestBackend>) -> usize {
        text_of(t).matches('┌').count()
    }

    #[tokio::test]
    async fn ctrl_f1_f2_hide_panels_but_keep_chrome() {
        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        st.init().await;

        // Both panels visible by default.
        assert_eq!(panel_count(&drawn(&mut st).await), 2);

        // Ctrl-F1 hides the left panel: one panel remains (in its own half).
        st.handle_key(key_ctrl(KeyCode::F(1))).await;
        assert!(st.panel_hidden[0] && !st.panel_hidden[1]);
        assert_eq!(panel_count(&drawn(&mut st).await), 1);

        // Ctrl-F2 hides the right one too: no panels, but the menu bar and the
        // F-key bar stay on screen.
        st.handle_key(key_ctrl(KeyCode::F(2))).await;
        let t = drawn(&mut st).await;
        assert_eq!(panel_count(&t), 0);
        let text = text_of(&t);
        assert!(text.contains("File") && text.contains("Options"), "menu bar remains");
        assert!(text.contains("Help") && text.contains("Quit"), "F-key bar remains");

        // Ctrl-F1 again brings the left panel back.
        st.handle_key(key_ctrl(KeyCode::F(1))).await;
        assert_eq!(panel_count(&drawn(&mut st).await), 1);
    }

    #[tokio::test]
    async fn hiding_active_panel_moves_focus_to_the_visible_one() {
        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        st.init().await;
        // Focus the left panel, then hide it — focus must move to the right.
        st.active = 0;
        st.handle_key(key_ctrl(KeyCode::F(1))).await;
        assert_eq!(st.active, 1, "focus follows to the still-visible panel");
        // With the left panel hidden, Tab must not move focus onto it.
        st.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)).await;
        assert_eq!(st.active, 1, "Tab skips the hidden panel");
    }

    #[tokio::test]
    async fn console_output_shows_through_a_hidden_panel() {
        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        st.init().await;

        // With no console output, hiding a panel leaves a blank backdrop.
        st.panel_hidden = [true, false];
        assert!(!text_of(&drawn(&mut st).await).contains("PING_MARKER"));

        // Feed the console some output (as a captured command would): it now
        // shows through the hidden (left) panel's half, behind the visible one.
        st.console.feed(b"PING_MARKER output line\r\n");
        let text = text_of(&drawn(&mut st).await);
        assert!(text.contains("PING_MARKER"), "console output visible under the hidden panel");

        // Showing both panels again covers the console (they paint opaquely).
        st.panel_hidden = [false, false];
        assert!(
            !text_of(&drawn(&mut st).await).contains("PING_MARKER"),
            "visible panels occlude the console backdrop"
        );

        // A console line wide enough to reach across both panels' interiors must
        // still be fully occluded — panels clear their cells, so the backdrop
        // can't bleed through cells the listing doesn't fill.
        st.console.feed(&vec![b'X'; 118]);
        st.console.feed(b"\r\n");
        let wide = "X".repeat(100);
        assert!(
            !text_of(&drawn(&mut st).await).contains(&wide),
            "a wide backdrop line must not bleed through visible panels"
        );
    }

    #[tokio::test]
    async fn half_height_exposes_the_lower_backdrop() {
        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        st.init().await;

        st.handle_key(key_ctrl(KeyCode::F(4))).await;
        assert!(st.half_height);
        let t = drawn(&mut st).await;
        // Both panels are still present, only shorter.
        assert_eq!(panel_count(&t), 2);
        // The last body row (just above the command line) is blank backdrop.
        let b = t.backend().buffer();
        let body_bottom = b.area.height - 3; // menu(0) … cmd(h-2) … fkeys(h-1)
        let blank = (0..b.area.width).all(|x| b[(x, body_bottom)].symbol() == " ");
        assert!(blank, "half-height leaves the lower body exposed");
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

    #[tokio::test]
    async fn disk_explorer_opens_and_draws() {
        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        st.init().await;
        let mut dv = crate::disk::DiskView::new(std::path::PathBuf::from("/tmp"));
        dv.scanning = false;
        dv.entries =
            vec![crate::disk::DiskEntry { name: "data".into(), size: 5_000_000, files: vec![] }];
        st.diskview = Some(dv);
        let mut t = Terminal::new(TestBackend::new(120, 30)).unwrap();
        t.draw(|f| draw(f, &mut st)).unwrap();
        let text = text_of(&t);
        assert!(text.contains("Disk Explorer"), "disk explorer renders over the UI");
        assert!(text.contains("data"), "subdirectory box labeled");
    }

    #[tokio::test]
    async fn process_explorer_opens_and_draws() {
        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        st.init().await;
        st.procview = Some(crate::proc::ProcView::new());
        let mut t = Terminal::new(TestBackend::new(120, 30)).unwrap();
        t.draw(|f| draw(f, &mut st)).unwrap();
        let text = text_of(&t);
        assert!(text.contains("Process Explorer"), "explorer renders over the UI");
        // The core panel border shows the CPU name "(N cores)", or "Cores (N)".
        assert!(text.to_lowercase().contains("cores"), "per-core graph present");
        assert!(text.contains("Mem"), "memory graph present");
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
        t.draw(|f| dlg.render(f, f.area(), &theme, None)).unwrap();
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
    fn command_palette_renders_and_filters() {
        use crate::ui::dialog::{
            CommandPaletteDialog, Dialog, PaletteAction, PaletteCategory, PaletteEntry,
        };
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let entries = vec![
            PaletteEntry::new("Copy", PaletteCategory::Command, PaletteAction::ToggleBookmarkCurrent),
            PaletteEntry::new(
                "Compare files",
                PaletteCategory::Command,
                PaletteAction::ToggleBookmarkCurrent,
            ),
            PaletteEntry::new(
                "Theme: Dracula",
                PaletteCategory::Setting,
                PaletteAction::ToggleBookmarkCurrent,
            ),
        ];
        let mut dlg = Dialog::CommandPalette(CommandPaletteDialog::new(entries));
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let theme = crate::ui::theme::Theme::mc();
        t.draw(|f| dlg.render(f, f.area(), &theme, None)).unwrap();
        let text = text_of(&t);
        assert!(text.contains("Command palette"), "shows the title");
        assert!(text.contains("Theme: Dracula"), "lists a settings entry");
        // Typing narrows the list to the matching entries.
        let mut key = |c| dlg.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        key('d');
        key('r');
        key('a');
        t.draw(|f| dlg.render(f, f.area(), &theme, None)).unwrap();
        let text = text_of(&t);
        assert!(text.contains("Dracula"), "'dra' keeps the Dracula theme entry");
        assert!(!text.contains("Compare files"), "'dra' filters out non-matches");
    }

    #[test]
    fn multi_rename_dialog_previews_new_names() {
        use crate::ui::dialog::{Dialog, MultiRenameDialog};
        use crate::vfs::VfsPath;
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let sources = vec![
            VfsPath::local("/tmp/photo.jpg"),
            VfsPath::local("/tmp/note.txt"),
        ];
        let mut dlg = Dialog::MultiRename(MultiRenameDialog::new(
            sources,
            "20260630".to_string(),
            "143007".to_string(),
        ));
        let theme = crate::ui::theme::Theme::mc();
        let mut t = Terminal::new(TestBackend::new(100, 24)).unwrap();

        // Default mask "[N].[E]" reproduces the original names.
        t.draw(|f| dlg.render(f, f.area(), &theme, None)).unwrap();
        let text = text_of(&t);
        for needle in ["Multi rename", "Original name", "New name", "photo.jpg", "note.txt", "Execute"] {
            assert!(text.contains(needle), "multi-rename dialog should show {needle:?}");
        }

        // Retype the mask to "[N]_[C]" and confirm the preview column reacts and
        // the counter advances per file.
        let mut key = |c: KeyCode| dlg.handle_key(KeyEvent::new(c, KeyModifiers::NONE));
        for _ in 0..7 {
            key(KeyCode::Backspace);
        }
        for c in ['[', 'N', ']', '_', '[', 'C', ']'] {
            key(KeyCode::Char(c));
        }
        t.draw(|f| dlg.render(f, f.area(), &theme, None)).unwrap();
        let text = text_of(&t);
        assert!(text.contains("photo_1"), "first file gets counter 1");
        assert!(text.contains("note_2"), "second file gets counter 2");
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
