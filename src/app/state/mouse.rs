//! Mouse handling: panel hit-testing and function-key bar clicks.

use super::*;

/// Two left clicks on the same entry within this window count as a double-click.
const DOUBLE_CLICK: Duration = Duration::from_millis(500);

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
                // Live theme + language preview, mirroring the keyboard path.
                self.preview_settings_choices();
                return self.handle_dialog_result(res).await;
            }
            // The wheel scrolls dialogs with a scrollable region (e.g. the
            // multi-rename file lists); three rows per notch, like the viewer.
            let delta = match ev.kind {
                MouseEventKind::ScrollDown => 3,
                MouseEventKind::ScrollUp => -3,
                _ => return Flow::Continue,
            };
            let res = self.dialog.as_mut().unwrap().handle_scroll(delta);
            // Wheel-scrolling a settings Choice dropdown previews live too.
            self.preview_settings_choices();
            return self.handle_dialog_result(res).await;
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
        // The theme editor hit-tests clicks/scroll against the zones it stored
        // during the last render.
        if self.theme_editor.is_some() {
            let sig = self.theme_editor.as_mut().unwrap().handle_mouse(ev);
            self.apply_theme_editor_signal(sig);
            return Flow::Continue;
        }

        // Disk explorer: a left click selects the box under the pointer; a second
        // click on the same box (within the double-click window) dives into it,
        // mirroring Enter. `usize::MAX` marks a disk click so it never collides
        // with a file-panel double-click.
        if self.diskview.is_some() {
            if let MouseEventKind::Down(MouseButton::Left) = ev.kind {
                const DISK: usize = usize::MAX;
                if let Some(i) = self.diskview.as_ref().unwrap().box_at(col, row) {
                    self.diskview.as_mut().unwrap().selected = i;
                    let now = Instant::now();
                    let double = self.last_click.is_some_and(|(p, idx, t)| {
                        p == DISK && idx == i && now.duration_since(t) < DOUBLE_CLICK
                    });
                    if double {
                        self.last_click = None; // a third click shouldn't re-fire
                        let sig = self.diskview.as_mut().unwrap().enter_selected();
                        self.apply_disk_signal(sig).await;
                    } else {
                        self.last_click = Some((DISK, i, now));
                    }
                }
            }
            return Flow::Continue;
        }

        // Network explorer. In the overview diagram a left click selects the IP
        // node under the pointer and opens its details (with reverse-DNS); the
        // wheel scrolls the grid. In the list panes the wheel scrolls the focused
        // pane. Other events are swallowed so they can't reach the hidden panels.
        if let Some(nv) = self.netview.as_mut() {
            if nv.focus == Pane::Overview {
                let mut sig = NetSignal::Stay;
                match ev.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        if let Some(i) = nv.node_at(col, row) {
                            nv.overview_cursor = i;
                            if let Some((ci, ii)) = nv.overview_nodes.get(i).map(|(c, r, _)| (*c, *r)) {
                                sig = nv.open_ip_detail_at(ci, ii);
                            }
                        }
                    }
                    MouseEventKind::ScrollDown => {
                        let _ = nv.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
                    }
                    MouseEventKind::ScrollUp => {
                        let _ = nv.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
                    }
                    _ => {}
                }
                if let NetSignal::ResolveDns(ip) = sig {
                    self.start_reverse_dns(ip);
                }
                return Flow::Continue;
            }
            let code = match ev.kind {
                MouseEventKind::ScrollDown => Some(KeyCode::Down),
                MouseEventKind::ScrollUp => Some(KeyCode::Up),
                _ => None,
            };
            if let Some(code) = code {
                for _ in 0..3 {
                    let _ = nv.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
                }
            }
            return Flow::Continue;
        }

        // The remaining full-screen overlays don't use the mouse yet; swallow the
        // event so it can't move the hidden file-panel cursor underneath them.
        if self.procview.is_some() || self.diffview.is_some() {
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
                // A click on the menu-bar mini progress bar opens the list of
                // background operations.
                let menubar_row = Rect { x: area.x, y: area.y, width: area.width, height: 1 };
                if let Some(mr) = self.menu_progress_rect(menubar_row)
                    && col >= mr.x
                    && col < mr.x + mr.width
                    && row == mr.y
                {
                    self.open_background_ops();
                    return Flow::Continue;
                }
                // A click on the menu bar (top row) opens that menu.
                if let Some(i) = MenuBarState::title_index_at(area, col, row) {
                    self.menu = Some(MenuBarState::new(i, &self.session_list(), self.side_remote()));
                } else if let Some((side, back)) = self.history_arrow_at(col, row) {
                    // A click on a panel's ◀/▶ history arrow steps it back/forward.
                    self.active = side;
                    if back {
                        self.go_back(side).await;
                    } else {
                        self.go_forward(side).await;
                    }
                } else if let Some((pi, idx)) = self.panel_point(col, row, PointAction::Cursor) {
                    // A second click on the same entry within the window opens it,
                    // exactly like pressing Enter (descend a dir, open a file).
                    let now = Instant::now();
                    let double = self.last_click.is_some_and(|(p, i, t)| {
                        p == pi && i == idx && now.duration_since(t) < DOUBLE_CLICK
                    });
                    if double {
                        self.last_click = None; // don't let a third click re-fire
                        // In the tree, a double-click opens the branch and points
                        // the other panel at it, just like pressing Enter.
                        if self.panels[self.active].is_tree() {
                            self.tree_enter().await;
                            return Flow::Continue;
                        }
                        return self.enter_dir().await;
                    }
                    self.last_click = Some((pi, idx, now));
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                self.panel_point(col, row, PointAction::Cursor);
            }
            MouseEventKind::Down(MouseButton::Right) | MouseEventKind::Drag(MouseButton::Right) => {
                self.panel_point(col, row, PointAction::InvertPaint);
            }
            _ => {}
        }
        Flow::Continue
    }

    /// Map a screen point to a panel entry: activate that panel, move the cursor
    /// onto the entry (every action), and optionally toggle/paint its mark.
    /// Returns the `(panel, entry)` that was hit, or `None` when the point misses
    /// the panels or any entry.
    fn panel_point(&mut self, col: u16, row: u16, action: PointAction) -> Option<(usize, usize)> {
        let pi = if self.panels[0].hit.is_some_and(|h| h.in_panel(col, row)) {
            0
        } else if self.panels[1].hit.is_some_and(|h| h.in_panel(col, row)) {
            1
        } else {
            return None;
        };
        self.active = pi;
        let p = &mut self.panels[pi];
        // Tree view: map the click to a tree row and move the tree cursor. There
        // is no marking in the tree, so any action just positions the cursor.
        if p.format == ViewFormat::Tree {
            let len = p.tree.as_ref().map_or(0, |t| t.rows.len());
            let idx = p.hit?.index_at(col, row, len)?;
            if let Some(t) = p.tree.as_mut() {
                t.cursor = idx;
            }
            return Some((pi, idx));
        }
        let idx = p.hit?.index_at(col, row, p.entries.len())?;
        // The cursor follows the pointer for every action (incl. drags).
        p.cursor = idx;
        if matches!(action, PointAction::Cursor) {
            return Some((pi, idx));
        }
        // Invert the mark, but only once per entry as the drag enters it, so a
        // run of drag events over the same file doesn't flip it repeatedly.
        if self.paint_last == Some((pi, idx)) {
            return Some((pi, idx));
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
        Some((pi, idx))
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
