//! Mouse handling: panel hit-testing and function-key bar clicks.

use super::*;

impl AppState {
    /// Handle a mouse event. Left clicks/drags move the cursor and drive the
    /// menus and dialogs; right clicks/drags mark files.
    pub async fn handle_mouse(&mut self, ev: MouseEvent) -> Flow {
        let area = self.last_area;
        let (col, row) = (ev.column, ev.row);
        let left_down = matches!(ev.kind, MouseEventKind::Down(MouseButton::Left));

        // A modal dialog gets first claim on a left click.
        if self.dialog.is_some() {
            if left_down {
                let res = self.dialog.as_mut().unwrap().handle_click(area, col, row);
                // Live theme preview, mirroring the keyboard path.
                if let Some(Dialog::Form(fd)) = &self.dialog
                    && let Some(name) = fd.theme_choice()
                    && name != self.theme.name
                {
                    self.theme = Theme::by_name(name, self.truecolor);
                }
                return self.handle_dialog_result(res).await;
            }
            return Flow::Continue;
        }

        // Then the pulldown menu.
        if self.menu.is_some() {
            if left_down {
                let signal = self.menu.as_mut().unwrap().click(area, col, row);
                return match signal {
                    MenuSignal::Stay => Flow::Continue,
                    MenuSignal::Close => {
                        self.menu = None;
                        Flow::Continue
                    }
                    MenuSignal::Activate(action) => {
                        self.menu = None;
                        self.run_menu_action(action).await
                    }
                };
            }
            return Flow::Continue;
        }

        // The disk manager handles its own clicks (cursor + double-click menus).
        if self.mountview.is_some() {
            let sig = self.mountview.as_mut().unwrap().handle_mouse(ev);
            self.apply_mount_signal(sig).await;
            return Flow::Continue;
        }

        // The editor and viewer handle their own mouse (cursor/marking/scroll).
        if self.editor.is_some() {
            let sig = self.editor.as_mut().unwrap().handle_mouse(ev);
            self.apply_editor_signal(sig).await;
            return Flow::Continue;
        }
        if self.viewer.is_some() {
            let sig = self.viewer.as_mut().unwrap().handle_mouse(ev);
            self.apply_viewer_signal(sig);
            return Flow::Continue;
        }

        // The remaining full-screen overlays don't use the mouse yet; swallow the
        // event so it can't move the hidden file-panel cursor underneath them.
        if self.procview.is_some() || self.diskview.is_some() || self.diffview.is_some() {
            return Flow::Continue;
        }

        // A fresh press starts a new gesture; forget the last painted entry.
        if matches!(ev.kind, MouseEventKind::Down(_)) {
            self.paint_last = None;
        }

        // Base mode: the F-key bar, then the menu bar, then the file panels.
        match ev.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // A click on the bottom F-key bar acts as that function key.
                if let Some(flow) = self.fkey_bar_click(area, col, row).await {
                    return flow;
                }
                // A click on the menu bar (top row) opens that menu.
                if let Some(i) = MenuBarState::title_index_at(area, col, row) {
                    self.menu = Some(MenuBarState::new(i));
                } else {
                    self.panel_point(col, row, PointAction::Cursor);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                self.panel_point(col, row, PointAction::Cursor)
            }
            MouseEventKind::Down(MouseButton::Right) | MouseEventKind::Drag(MouseButton::Right) => {
                self.panel_point(col, row, PointAction::InvertPaint)
            }
            _ => {}
        }
        Flow::Continue
    }

    /// Map a screen point to a panel entry: activate that panel, move the cursor
    /// onto the entry (every action), and optionally toggle/paint its mark.
    fn panel_point(&mut self, col: u16, row: u16, action: PointAction) {
        let pi = if self.panels[0].hit.is_some_and(|h| h.in_panel(col, row)) {
            0
        } else if self.panels[1].hit.is_some_and(|h| h.in_panel(col, row)) {
            1
        } else {
            return;
        };
        self.active = pi;
        let p = &mut self.panels[pi];
        let Some(hit) = p.hit else { return };
        let Some(idx) = hit.index_at(col, row, p.entries.len()) else {
            return;
        };
        // The cursor follows the pointer for every action (incl. drags).
        p.cursor = idx;
        if matches!(action, PointAction::Cursor) {
            return;
        }
        // Invert the mark, but only once per entry as the drag enters it, so a
        // run of drag events over the same file doesn't flip it repeatedly.
        if self.paint_last == Some((pi, idx)) {
            return;
        }
        self.paint_last = Some((pi, idx));
        let p = &mut self.panels[pi];
        // Selection never touches the "..".
        if let Some(e) = p.entries.get(idx)
            && e.name != ".."
        {
            let name = e.name.clone();
            p.selection.toggle(&name);
        }
    }

    /// If `(col, row)` falls on the bottom F-key bar, run the corresponding
    /// panel-mode function key and return its `Flow`; otherwise `None`.
    async fn fkey_bar_click(&mut self, area: Rect, col: u16, row: u16) -> Option<Flow> {
        let bar = Rect {
            x: area.x,
            y: area.y + area.height.saturating_sub(1),
            width: area.width,
            height: 1,
        };
        let i = crate::ui::fkeys::index_at(bar, &crate::ui::fkeys::PANEL_LABELS, col, row)?;
        let key = KeyEvent::new(KeyCode::F(i as u8 + 1), KeyModifiers::NONE);
        Some(self.handle_panel_key(key).await)
    }

}
