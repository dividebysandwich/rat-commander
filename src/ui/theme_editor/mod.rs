//! The visual theme editor (Options → Edit themes): a full-screen mode to pick a
//! theme, edit every UI color with a live preview, and save.
//!
//! It follows the standard full-screen "mode" pattern (like `diff`/`proc`): a
//! state struct on [`crate::app::state::AppState`], a `handle_key` returning a
//! [`ThemeEditorSignal`], and a sibling [`render`] module. Sub-prompts (the
//! unsaved-changes confirmation and the "Save as" name entry) are self-contained
//! [`Overlay`]s drawn on top of the editor, so nothing leaks into the global
//! dialog/`Submit` machinery.

pub mod render;

use crate::ui::theme::{self, PreviewKind, ThemeSpec, THEME_FIELDS};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use ratatui::style::Color;

/// The 16 palette swatches offered on non-truecolor terminals (classic ANSI
/// colors as explicit RGB, since specs always store `#rrggbb`).
pub(crate) const SWATCHES: [(u8, u8, u8); 16] = [
    (0x00, 0x00, 0x00), (0xaa, 0x00, 0x00), (0x00, 0xaa, 0x00), (0xaa, 0x55, 0x00),
    (0x00, 0x00, 0xaa), (0xaa, 0x00, 0xaa), (0x00, 0xaa, 0xaa), (0xaa, 0xaa, 0xaa),
    (0x55, 0x55, 0x55), (0xff, 0x55, 0x55), (0x55, 0xff, 0x55), (0xff, 0xff, 0x55),
    (0x55, 0x55, 0xff), (0xff, 0x55, 0xff), (0x55, 0xff, 0xff), (0xff, 0xff, 0xff),
];

/// What [`ThemeEditor::handle_key`] asks the app to do.
pub enum ThemeEditorSignal {
    /// Keep editing; nothing for the app to do.
    Stay,
    /// Close the editor and return to the panels.
    Close,
    /// Persist this spec (update-or-append in the active set + `themes.toml`),
    /// then call [`ThemeEditor::mark_saved`]; the editor keeps running. Boxed
    /// because a [`ThemeSpec`] is large relative to the unit variants.
    Save(Box<ThemeSpec>),
    /// Persist this spec, then close the editor (the "Save" choice on the
    /// unsaved-changes-on-exit prompt). The editor stays open if the save fails.
    SaveAndClose(Box<ThemeSpec>),
}

/// Which pane has keyboard focus. `Color` is the RGB channels (truecolor) or the
/// swatch grid (otherwise).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Focus {
    Picker,
    List,
    Color,
    Buttons,
}

const FOCI: [Focus; 4] = [Focus::Picker, Focus::List, Focus::Color, Focus::Buttons];

/// A modal sub-prompt drawn centered over the editor.
pub(crate) enum Overlay {
    None,
    /// Unsaved changes while switching the picker to `target`. Buttons: 0 = Save,
    /// 1 = Discard, 2 = Cancel.
    ConfirmSwitch { target: usize, button: usize },
    /// Unsaved changes while trying to close the editor. Buttons: 0 = Save,
    /// 1 = Discard, 2 = Cancel.
    ConfirmExit { button: usize },
    /// "Save as" — enter a new theme name.
    SaveAs { name: String, cursor: usize },
}

pub struct ThemeEditor {
    pub(crate) truecolor: bool,
    /// Theme names for the picker (active set, file order).
    pub(crate) names: Vec<String>,
    /// Selected theme index into `names`.
    pub(crate) picker: usize,
    /// Working (editable) copy of the selected theme.
    pub(crate) spec: ThemeSpec,
    /// Last-saved snapshot of the selected theme, for the dirty check.
    baseline: ThemeSpec,
    /// Selected color item (index into [`THEME_FIELDS`]).
    pub(crate) item: usize,
    /// Top of the visible item-list window (scroll offset).
    pub(crate) item_top: usize,
    pub(crate) focus: Focus,
    /// Active RGB channel in the color picker (0 = R, 1 = G, 2 = B) — truecolor.
    pub(crate) channel: usize,
    /// Selected swatch (0..16) in the non-truecolor color picker.
    pub(crate) swatch: usize,
    /// Hex digits typed so far in the color picker (no `#`), or `None` when not
    /// typing. Applied to the selected element once six digits are entered.
    pub(crate) hex_input: Option<String>,
    /// Selected button (0 = Save, 1 = Save as, 2 = Cancel).
    pub(crate) button: usize,
    pub(crate) overlay: Overlay,
    /// The app's active theme name, so a save of it can refresh the live UI.
    pub(crate) app_theme: String,
    // -- Hit-test zones, in absolute screen coords, written every render() so
    //    the mouse handler and the drawing always agree on where things are. --
    pub(crate) z_picker: Rect,
    pub(crate) z_list: Rect,
    pub(crate) z_color: Rect,
    pub(crate) z_buttons: [Rect; 3],
    pub(crate) z_overlay: [Rect; 3],
}

/// Normalize a theme name for matching (lower-case, no spaces/dashes/underscores)
/// — mirrors `theme::norm_name`, which is private.
fn norm(name: &str) -> String {
    name.to_ascii_lowercase().replace([' ', '-', '_'], "")
}

/// The RGB components of a spec color (specs always store `Color::Rgb`).
pub(crate) fn rgb_of(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (128, 128, 128),
    }
}

/// Index of the swatch nearest to `c` (squared-distance), for seeding the
/// non-truecolor picker from an arbitrary color.
fn nearest_swatch(c: Color) -> usize {
    let (r, g, b) = rgb_of(c);
    let dist = |s: &(u8, u8, u8)| {
        let dr = s.0 as i32 - r as i32;
        let dg = s.1 as i32 - g as i32;
        let db = s.2 as i32 - b as i32;
        dr * dr + dg * dg + db * db
    };
    SWATCHES
        .iter()
        .enumerate()
        .min_by_key(|(_, s)| dist(s))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Whether `(col, row)` falls inside `r`.
fn within(r: Rect, col: u16, row: u16) -> bool {
    col >= r.x && col < r.x.saturating_add(r.width) && row >= r.y && row < r.y.saturating_add(r.height)
}

/// The index of the first of `rects` containing `(col, row)`.
fn hit(rects: &[Rect; 3], col: u16, row: u16) -> Option<usize> {
    rects.iter().position(|r| within(*r, col, row))
}

fn enter_key() -> KeyEvent {
    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
}

/// Parse exactly six hex digits (no `#`) into an RGB color.
fn parse_hex6(s: &str) -> Option<Color> {
    if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let n = u32::from_str_radix(s, 16).ok()?;
    Some(Color::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

impl ThemeEditor {
    /// Open the editor on the app's current theme (`app_theme`), or the first
    /// active theme if that name is unknown.
    pub fn new(app_theme: &str, truecolor: bool) -> Self {
        let specs = theme::active_specs();
        let names: Vec<String> = specs.iter().map(|s| s.name.clone()).collect();
        let picker = names.iter().position(|n| norm(n) == norm(app_theme)).unwrap_or(0);
        let spec = specs.get(picker).cloned().unwrap_or_else(|| specs[0].clone());
        let baseline = spec.clone();
        let swatch = nearest_swatch(spec.color_at(0));
        ThemeEditor {
            truecolor,
            names,
            picker,
            spec,
            baseline,
            item: 0,
            item_top: 0,
            focus: Focus::List,
            channel: 0,
            swatch,
            hex_input: None,
            button: 0,
            overlay: Overlay::None,
            app_theme: app_theme.to_string(),
            z_picker: Rect::ZERO,
            z_list: Rect::ZERO,
            z_color: Rect::ZERO,
            z_buttons: [Rect::ZERO; 3],
            z_overlay: [Rect::ZERO; 3],
        }
    }

    /// Whether the working spec differs from its last-saved snapshot.
    pub(crate) fn dirty(&self) -> bool {
        self.spec != self.baseline
    }

    /// The preview surface for the currently-selected item.
    pub(crate) fn preview_kind(&self) -> PreviewKind {
        THEME_FIELDS[self.item.min(THEME_FIELDS.len() - 1)].preview
    }

    /// Confirm a save succeeded: adopt the working spec as the new baseline and
    /// re-sync the picker with the (possibly newly-added) active theme list.
    pub fn mark_saved(&mut self) {
        self.baseline = self.spec.clone();
        self.names = theme::palette_names();
        if let Some(i) = self.names.iter().position(|n| norm(n) == norm(&self.spec.name)) {
            self.picker = i;
        }
    }

    /// Load the active theme at `idx` as the working spec (discarding edits).
    fn load_theme(&mut self, idx: usize) {
        let specs = theme::active_specs();
        let idx = idx.min(specs.len().saturating_sub(1));
        self.picker = idx;
        self.spec = specs.get(idx).cloned().unwrap_or_else(|| self.spec.clone());
        self.baseline = self.spec.clone();
        self.swatch = nearest_swatch(self.spec.color_at(self.item));
    }

    /// Save the working theme and close the editor (the primary Save action —
    /// the button, `F2` and `Ctrl-S`). Persisting-and-staying is only used by the
    /// "save first" choice when switching themes in the picker.
    fn request_save(&self) -> ThemeEditorSignal {
        ThemeEditorSignal::SaveAndClose(Box::new(self.spec.clone()))
    }

    /// Exit the editor, prompting first if there are unsaved changes.
    fn request_exit(&mut self) -> ThemeEditorSignal {
        if self.dirty() {
            self.overlay = Overlay::ConfirmExit { button: 0 };
            ThemeEditorSignal::Stay
        } else {
            ThemeEditorSignal::Close
        }
    }

    // -- key handling -------------------------------------------------------

    pub fn handle_key(&mut self, key: KeyEvent) -> ThemeEditorSignal {
        match self.overlay {
            Overlay::None => {}
            Overlay::ConfirmSwitch { .. } => return self.key_confirm_switch(key),
            Overlay::ConfirmExit { .. } => return self.key_confirm_exit(key),
            Overlay::SaveAs { .. } => return self.key_save_as(key),
        }
        // Esc while typing a hex code just cancels the entry, not the editor.
        if self.hex_input.is_some() && matches!(key.code, KeyCode::Esc) {
            self.hex_input = None;
            return ThemeEditorSignal::Stay;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc | KeyCode::F(10) => return self.request_exit(),
            KeyCode::F(2) => return self.request_save(),
            KeyCode::Char('s') if ctrl => return self.request_save(),
            KeyCode::Tab => self.cycle_focus(1),
            KeyCode::BackTab => self.cycle_focus(-1),
            _ => {
                return match self.focus {
                    Focus::Picker => {
                        self.key_picker(key);
                        ThemeEditorSignal::Stay
                    }
                    Focus::List => {
                        self.key_list(key);
                        ThemeEditorSignal::Stay
                    }
                    Focus::Color => {
                        self.key_color(key);
                        ThemeEditorSignal::Stay
                    }
                    Focus::Buttons => self.key_buttons(key),
                };
            }
        }
        ThemeEditorSignal::Stay
    }

    fn cycle_focus(&mut self, dir: i32) {
        let cur = FOCI.iter().position(|f| *f == self.focus).unwrap_or(0) as i32;
        let n = FOCI.len() as i32;
        self.focus = FOCI[(((cur + dir) % n + n) % n) as usize];
        if self.focus == Focus::Color {
            self.swatch = nearest_swatch(self.spec.color_at(self.item));
        }
    }

    fn key_picker(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Left => self.move_picker(-1),
            KeyCode::Down | KeyCode::Right => self.move_picker(1),
            KeyCode::Home => self.move_picker(-(self.picker as i32)),
            KeyCode::End => self.move_picker(self.names.len() as i32),
            _ => {}
        }
    }

    fn move_picker(&mut self, delta: i32) {
        if self.names.is_empty() {
            return;
        }
        let max = self.names.len() as i32 - 1;
        let target = (self.picker as i32 + delta).clamp(0, max) as usize;
        if target == self.picker {
            return;
        }
        if self.dirty() {
            self.overlay = Overlay::ConfirmSwitch { target, button: 0 };
        } else {
            self.load_theme(target);
        }
    }

    fn key_list(&mut self, key: KeyEvent) {
        // Hex digits type a color code for the selected element without leaving
        // the list; any other key ends an in-progress entry.
        if self.type_hex(key) {
            return;
        }
        self.hex_input = None;
        let last = THEME_FIELDS.len() - 1;
        match key.code {
            KeyCode::Up => self.item = self.item.saturating_sub(1),
            KeyCode::Down => self.item = (self.item + 1).min(last),
            KeyCode::PageUp => self.item = self.item.saturating_sub(8),
            KeyCode::PageDown => self.item = (self.item + 8).min(last),
            KeyCode::Home => self.item = 0,
            KeyCode::End => self.item = last,
            KeyCode::Enter | KeyCode::Right => {
                self.focus = Focus::Color;
                self.swatch = nearest_swatch(self.spec.color_at(self.item));
            }
            _ => {}
        }
    }

    /// Feed a key into hex-code entry (works from both the element list and the
    /// color picker, so the user needn't switch panes). Returns `true` when the
    /// key was a hex digit — extending, and on the sixth applying, the code — or a
    /// Backspace editing an active entry.
    fn type_hex(&mut self, key: KeyEvent) -> bool {
        if let KeyCode::Char(c) = key.code
            && c.is_ascii_hexdigit()
        {
            let buf = self.hex_input.get_or_insert_with(String::new);
            if buf.len() < 6 {
                buf.push(c.to_ascii_lowercase());
            }
            if buf.len() == 6 {
                let code = self.hex_input.take().unwrap_or_default();
                if let Some(color) = parse_hex6(&code) {
                    self.spec.set_color_at(self.item, color);
                    self.swatch = nearest_swatch(color);
                }
            }
            return true;
        }
        if matches!(key.code, KeyCode::Backspace) && self.hex_input.is_some() {
            if let Some(b) = self.hex_input.as_mut() {
                b.pop();
                if b.is_empty() {
                    self.hex_input = None;
                }
            }
            return true;
        }
        false
    }

    fn key_color(&mut self, key: KeyEvent) {
        if self.type_hex(key) {
            return;
        }
        // Any other key ends an in-progress hex entry and acts on the sliders.
        self.hex_input = None;
        // Shift + Left/Right steps the channel by 20 instead of 1 (needs a
        // terminal that reports Shift with arrow keys; otherwise it steps by 1).
        let step = if key.modifiers.contains(KeyModifiers::SHIFT) { 20 } else { 1 };
        if self.truecolor {
            match key.code {
                KeyCode::Up => self.channel = self.channel.saturating_sub(1),
                KeyCode::Down => self.channel = (self.channel + 1).min(2),
                KeyCode::Left => self.adjust_channel(-step),
                KeyCode::Right => self.adjust_channel(step),
                KeyCode::PageUp => self.adjust_channel(16),
                KeyCode::PageDown => self.adjust_channel(-16),
                KeyCode::Home => self.set_channel(0),
                KeyCode::End => self.set_channel(255),
                KeyCode::Enter => self.focus = Focus::List,
                _ => {}
            }
        } else {
            // 16-swatch grid, 4 columns wide.
            match key.code {
                KeyCode::Left => self.swatch = self.swatch.saturating_sub(1),
                KeyCode::Right => self.swatch = (self.swatch + 1).min(15),
                KeyCode::Up => self.swatch = self.swatch.saturating_sub(4),
                KeyCode::Down => self.swatch = (self.swatch + 4).min(15),
                KeyCode::Enter => {
                    self.focus = Focus::List;
                    return;
                }
                _ => return,
            }
            // Selecting a swatch applies it live.
            let (r, g, b) = SWATCHES[self.swatch];
            self.spec.set_color_at(self.item, Color::Rgb(r, g, b));
        }
    }

    fn adjust_channel(&mut self, delta: i32) {
        let (r, g, b) = rgb_of(self.spec.color_at(self.item));
        let mut ch = [r, g, b];
        ch[self.channel] = (ch[self.channel] as i32 + delta).clamp(0, 255) as u8;
        self.spec.set_color_at(self.item, Color::Rgb(ch[0], ch[1], ch[2]));
    }

    fn set_channel(&mut self, value: u8) {
        let (r, g, b) = rgb_of(self.spec.color_at(self.item));
        let mut ch = [r, g, b];
        ch[self.channel] = value;
        self.spec.set_color_at(self.item, Color::Rgb(ch[0], ch[1], ch[2]));
    }

    fn key_buttons(&mut self, key: KeyEvent) -> ThemeEditorSignal {
        match key.code {
            KeyCode::Left => {
                self.button = (self.button + 2) % 3;
                ThemeEditorSignal::Stay
            }
            KeyCode::Right => {
                self.button = (self.button + 1) % 3;
                ThemeEditorSignal::Stay
            }
            KeyCode::Enter | KeyCode::Char(' ') => match self.button {
                0 => self.request_save(),
                1 => {
                    self.overlay = Overlay::SaveAs {
                        cursor: self.spec.name.chars().count(),
                        name: self.spec.name.clone(),
                    };
                    ThemeEditorSignal::Stay
                }
                _ => self.request_exit(),
            },
            _ => ThemeEditorSignal::Stay,
        }
    }

    fn key_confirm_exit(&mut self, key: KeyEvent) -> ThemeEditorSignal {
        let Overlay::ConfirmExit { button } = &mut self.overlay else {
            return ThemeEditorSignal::Stay;
        };
        match key.code {
            KeyCode::Left => *button = (*button + 2) % 3,
            KeyCode::Right => *button = (*button + 1) % 3,
            KeyCode::Esc => self.overlay = Overlay::None, // cancel the exit
            KeyCode::Enter | KeyCode::Char(' ') => {
                let choice = *button;
                match choice {
                    0 => {
                        // Save, then close (the app closes us only if the save
                        // succeeds, so edits aren't lost on a write error).
                        self.overlay = Overlay::None;
                        return ThemeEditorSignal::SaveAndClose(Box::new(self.spec.clone()));
                    }
                    1 => return ThemeEditorSignal::Close, // discard & close
                    _ => self.overlay = Overlay::None,    // cancel: keep editing
                }
            }
            _ => {}
        }
        ThemeEditorSignal::Stay
    }

    fn key_confirm_switch(&mut self, key: KeyEvent) -> ThemeEditorSignal {
        let Overlay::ConfirmSwitch { target, button } = &mut self.overlay else {
            return ThemeEditorSignal::Stay;
        };
        let target = *target;
        match key.code {
            KeyCode::Left => *button = (*button + 2) % 3,
            KeyCode::Right => *button = (*button + 1) % 3,
            KeyCode::Esc => self.overlay = Overlay::None,
            KeyCode::Enter | KeyCode::Char(' ') => {
                let choice = *button;
                self.overlay = Overlay::None;
                match choice {
                    0 => {
                        // Save the current edits, then switch.
                        let saved = Box::new(self.spec.clone());
                        self.load_theme(target);
                        return ThemeEditorSignal::Save(saved);
                    }
                    1 => self.load_theme(target), // discard edits, switch
                    _ => {}                       // cancel: stay put
                }
            }
            _ => {}
        }
        ThemeEditorSignal::Stay
    }

    fn key_save_as(&mut self, key: KeyEvent) -> ThemeEditorSignal {
        let Overlay::SaveAs { name, cursor } = &mut self.overlay else {
            return ThemeEditorSignal::Stay;
        };
        match key.code {
            KeyCode::Esc => {
                self.overlay = Overlay::None;
                ThemeEditorSignal::Stay
            }
            KeyCode::Enter => {
                let trimmed = name.trim().to_string();
                if trimmed.is_empty() {
                    return ThemeEditorSignal::Stay;
                }
                self.overlay = Overlay::None;
                self.spec.name = trimmed;
                self.request_save()
            }
            _ => {
                let _ = crate::ui::textedit::edit_key(name, cursor, key);
                ThemeEditorSignal::Stay
            }
        }
    }

    // -- mouse handling -----------------------------------------------------

    /// Route a mouse event against the zones stored by the last [`render`]. Left
    /// clicks select/activate; the wheel over the color list scrolls it.
    pub fn handle_mouse(&mut self, ev: MouseEvent) -> ThemeEditorSignal {
        let (col, row) = (ev.column, ev.row);
        // While an overlay is up the mouse only reaches its buttons.
        match &self.overlay {
            Overlay::ConfirmSwitch { .. } | Overlay::ConfirmExit { .. } => {
                if matches!(ev.kind, MouseEventKind::Down(MouseButton::Left))
                    && let Some(i) = hit(&self.z_overlay, col, row)
                {
                    return self.activate_overlay_button(i);
                }
                return ThemeEditorSignal::Stay;
            }
            Overlay::SaveAs { .. } => return ThemeEditorSignal::Stay,
            Overlay::None => {}
        }
        let last = THEME_FIELDS.len() - 1;
        match ev.kind {
            MouseEventKind::ScrollDown if within(self.z_list, col, row) => {
                self.item = (self.item + 1).min(last);
            }
            MouseEventKind::ScrollUp if within(self.z_list, col, row) => {
                self.item = self.item.saturating_sub(1);
            }
            MouseEventKind::Down(MouseButton::Left) => return self.click(col, row),
            _ => {}
        }
        ThemeEditorSignal::Stay
    }

    fn click(&mut self, col: u16, row: u16) -> ThemeEditorSignal {
        if within(self.z_picker, col, row) {
            self.focus = Focus::Picker;
            let mid = self.z_picker.x + self.z_picker.width / 2;
            self.move_picker(if col < mid { -1 } else { 1 });
        } else if within(self.z_list, col, row) {
            self.focus = Focus::List;
            let idx = self.item_top + (row - self.z_list.y) as usize;
            if idx < THEME_FIELDS.len() {
                self.item = idx;
            }
        } else if within(self.z_color, col, row) {
            self.focus = Focus::Color;
            self.click_color(col, row);
        } else if let Some(i) = hit(&self.z_buttons, col, row) {
            self.button = i;
            self.focus = Focus::Buttons;
            return self.key_buttons(enter_key());
        }
        ThemeEditorSignal::Stay
    }

    /// A click inside the color picker: pick a channel and set its value from the
    /// click position along the gauge (truecolor), or pick a swatch (16-color).
    fn click_color(&mut self, col: u16, row: u16) {
        let inner = self.z_color;
        if self.truecolor {
            // Three gauge rows: inner.y+1 = R, +2 = G, +3 = B.
            if let Some(ch) = row.checked_sub(inner.y + 1)
                && (ch as usize) < 3
            {
                self.channel = ch as usize;
                let bx = inner.x + 7; // gauge starts after the "R 205 " label
                let right = inner.x + inner.width;
                if col >= bx && right > bx {
                    let bw = (right - bx) as u32;
                    let pos = (col - bx) as u32;
                    let val = if pos + 1 >= bw { 255 } else { (pos * 255 / bw) as u8 };
                    self.set_channel(val);
                }
            }
        } else {
            for (i, &(r, g, b)) in SWATCHES.iter().enumerate() {
                let x = inner.x + (i % 4) as u16 * 4;
                let y = inner.y + 1 + (i / 4) as u16;
                if row == y && col >= x && col < x + 3 {
                    self.swatch = i;
                    self.spec.set_color_at(self.item, Color::Rgb(r, g, b));
                    break;
                }
            }
        }
    }

    /// Click button `i` (0 Save, 1 Discard, 2 Cancel) on the active overlay by
    /// reusing its keyboard handler.
    fn activate_overlay_button(&mut self, i: usize) -> ThemeEditorSignal {
        let kind = match &self.overlay {
            Overlay::ConfirmSwitch { .. } => 1,
            Overlay::ConfirmExit { .. } => 2,
            _ => 0,
        };
        match kind {
            1 => {
                if let Overlay::ConfirmSwitch { button, .. } = &mut self.overlay {
                    *button = i;
                }
                self.key_confirm_switch(enter_key())
            }
            2 => {
                if let Overlay::ConfirmExit { button } = &mut self.overlay {
                    *button = i;
                }
                self.key_confirm_exit(enter_key())
            }
            _ => ThemeEditorSignal::Stay,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::Theme;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn mouse_down(col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn wheel(col: u16, row: u16, down: bool) -> MouseEvent {
        MouseEvent {
            kind: if down { MouseEventKind::ScrollDown } else { MouseEventKind::ScrollUp },
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn rendered(ed: &mut ThemeEditor) -> Terminal<TestBackend> {
        let theme = Theme::mc();
        let mut t = Terminal::new(TestBackend::new(120, 32)).unwrap();
        t.draw(|f| render::render(f, f.area(), ed, &theme)).unwrap();
        t
    }

    fn dialog_item() -> usize {
        THEME_FIELDS.iter().position(|m| m.preview == PreviewKind::Dialog).unwrap()
    }
    fn editor_item() -> usize {
        THEME_FIELDS.iter().position(|m| m.preview == PreviewKind::Editor).unwrap()
    }

    #[test]
    fn opens_on_the_current_theme() {
        let ed = ThemeEditor::new("Midnight Commander", true);
        assert_eq!(norm(&ed.names[ed.picker]), norm("Midnight Commander"));
        assert!(!ed.dirty());
    }

    #[test]
    fn editing_a_channel_marks_dirty_and_changes_the_color() {
        let mut ed = ThemeEditor::new("Midnight Commander", true);
        let before = ed.spec.color_at(ed.item);
        ed.handle_key(k(KeyCode::Right)); // List → Color
        assert_eq!(ed.focus, Focus::Color);
        ed.handle_key(k(KeyCode::End)); // R channel → 255
        let after = ed.spec.color_at(ed.item);
        assert_ne!(before, after, "color must change");
        assert!(ed.dirty());
        assert_eq!(rgb_of(after).0, 255, "red channel maxed");
    }

    #[test]
    fn shift_arrows_step_the_channel_by_twenty() {
        let mut ed = ThemeEditor::new("Midnight Commander", true);
        ed.handle_key(k(KeyCode::Right)); // List → Color
        ed.handle_key(k(KeyCode::Home)); // R channel → 0
        assert_eq!(rgb_of(ed.spec.color_at(ed.item)).0, 0);
        ed.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT)); // +20
        assert_eq!(rgb_of(ed.spec.color_at(ed.item)).0, 20);
        ed.handle_key(k(KeyCode::Right)); // +1
        assert_eq!(rgb_of(ed.spec.color_at(ed.item)).0, 21);
        ed.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT)); // -20
        assert_eq!(rgb_of(ed.spec.color_at(ed.item)).0, 1);
    }

    #[test]
    fn typing_a_hex_code_sets_the_color() {
        let mut ed = ThemeEditor::new("Midnight Commander", true);
        ed.handle_key(k(KeyCode::Right)); // List → Color
        for c in ['1', 'a', '2', 'b', '3', 'c'] {
            ed.handle_key(k(KeyCode::Char(c)));
        }
        assert_eq!(rgb_of(ed.spec.color_at(ed.item)), (0x1a, 0x2b, 0x3c));
        assert!(ed.hex_input.is_none(), "buffer cleared after six digits");
        assert!(ed.dirty());
    }

    #[test]
    fn hex_typing_works_in_the_element_list() {
        let mut ed = ThemeEditor::new("Midnight Commander", true);
        assert_eq!(ed.focus, Focus::List, "list is the default focus");
        for c in ['0', '0', 'f', 'f', '8', '8'] {
            ed.handle_key(k(KeyCode::Char(c)));
        }
        assert_eq!(rgb_of(ed.spec.color_at(ed.item)), (0x00, 0xff, 0x88));
        assert_eq!(ed.focus, Focus::List, "typing hex does not leave the list");
        assert!(ed.dirty());
    }

    #[test]
    fn hex_typing_edits_with_backspace_and_cancels_with_esc() {
        let mut ed = ThemeEditor::new("Midnight Commander", true);
        ed.focus = Focus::Color;
        ed.handle_key(k(KeyCode::Char('f')));
        ed.handle_key(k(KeyCode::Char('0')));
        assert_eq!(ed.hex_input.as_deref(), Some("f0"));
        ed.handle_key(k(KeyCode::Backspace));
        assert_eq!(ed.hex_input.as_deref(), Some("f"));
        // Esc cancels the entry instead of closing the editor.
        assert!(matches!(ed.handle_key(k(KeyCode::Esc)), ThemeEditorSignal::Stay));
        assert!(ed.hex_input.is_none());
        // An arrow key also ends entry and moves a slider instead.
        ed.handle_key(k(KeyCode::Char('a')));
        assert!(ed.hex_input.is_some());
        ed.handle_key(k(KeyCode::Right));
        assert!(ed.hex_input.is_none());
    }

    #[test]
    fn save_signal_carries_the_spec_and_mark_saved_clears_dirty() {
        let mut ed = ThemeEditor::new("Midnight Commander", true);
        ed.handle_key(k(KeyCode::Right));
        ed.handle_key(k(KeyCode::End));
        assert!(ed.dirty());
        match ed.handle_key(k(KeyCode::F(2))) {
            ThemeEditorSignal::SaveAndClose(spec) => assert_eq!(*spec, ed.spec),
            _ => panic!("F2 should save and close"),
        }
        ed.mark_saved();
        assert!(!ed.dirty(), "mark_saved adopts the new baseline");
    }

    #[test]
    fn switching_theme_while_dirty_prompts_and_discard_switches() {
        let mut ed = ThemeEditor::new("Midnight Commander", true);
        ed.handle_key(k(KeyCode::Right));
        ed.handle_key(k(KeyCode::End)); // dirty
        ed.focus = Focus::Picker;
        ed.handle_key(k(KeyCode::Down)); // try to switch → prompt
        assert!(matches!(ed.overlay, Overlay::ConfirmSwitch { target: 1, .. }));
        // Move to "Discard" (button 1) and confirm.
        ed.handle_key(k(KeyCode::Right));
        let sig = ed.handle_key(k(KeyCode::Enter));
        assert!(matches!(sig, ThemeEditorSignal::Stay));
        assert!(matches!(ed.overlay, Overlay::None));
        assert_eq!(ed.picker, 1, "switched to the target theme");
        assert!(!ed.dirty(), "edits discarded onto the new theme");
    }

    #[test]
    fn switching_theme_while_dirty_can_save_first() {
        let mut ed = ThemeEditor::new("Midnight Commander", true);
        let edited_name = ed.spec.name.clone();
        ed.handle_key(k(KeyCode::Right));
        ed.handle_key(k(KeyCode::End));
        ed.focus = Focus::Picker;
        ed.handle_key(k(KeyCode::Down)); // prompt (button defaults to Save)
        match ed.handle_key(k(KeyCode::Enter)) {
            ThemeEditorSignal::Save(spec) => assert_eq!(spec.name, edited_name),
            _ => panic!("Save button should emit a Save of the edited theme"),
        }
        assert_eq!(ed.picker, 1, "after saving, we are on the new theme");
    }

    #[test]
    fn save_as_sets_a_new_name() {
        let mut ed = ThemeEditor::new("Midnight Commander", true);
        ed.focus = Focus::Buttons;
        ed.button = 1; // Save as…
        ed.handle_key(k(KeyCode::Enter));
        assert!(matches!(ed.overlay, Overlay::SaveAs { .. }));
        ed.handle_key(k(KeyCode::Char('X')));
        match ed.handle_key(k(KeyCode::Enter)) {
            ThemeEditorSignal::SaveAndClose(spec) => assert!(spec.name.ends_with('X')),
            _ => panic!("Save as should save (and close) under the new name"),
        }
    }

    #[test]
    fn exiting_while_clean_closes_immediately() {
        let mut ed = ThemeEditor::new("Midnight Commander", true);
        assert!(matches!(ed.handle_key(k(KeyCode::Esc)), ThemeEditorSignal::Close));
    }

    #[test]
    fn exiting_while_dirty_prompts_then_discards() {
        let mut ed = ThemeEditor::new("Midnight Commander", true);
        ed.handle_key(k(KeyCode::Right)); // List → Color
        ed.handle_key(k(KeyCode::End)); // edit → dirty
        // Esc no longer closes directly; it raises the prompt.
        assert!(matches!(ed.handle_key(k(KeyCode::Esc)), ThemeEditorSignal::Stay));
        assert!(matches!(ed.overlay, Overlay::ConfirmExit { button: 0 }));
        // Move to "Discard" (button 1) and confirm → close.
        ed.handle_key(k(KeyCode::Right));
        assert!(matches!(ed.handle_key(k(KeyCode::Enter)), ThemeEditorSignal::Close));
    }

    #[test]
    fn exiting_while_dirty_can_save_and_close() {
        let mut ed = ThemeEditor::new("Midnight Commander", true);
        ed.handle_key(k(KeyCode::Right));
        ed.handle_key(k(KeyCode::End));
        ed.handle_key(k(KeyCode::F(10))); // prompt (Save focused)
        match ed.handle_key(k(KeyCode::Enter)) {
            ThemeEditorSignal::SaveAndClose(spec) => assert_eq!(*spec, ed.spec),
            _ => panic!("Save should save and close"),
        }
    }

    #[test]
    fn exiting_prompt_cancel_keeps_editing() {
        let mut ed = ThemeEditor::new("Midnight Commander", true);
        ed.handle_key(k(KeyCode::Right));
        ed.handle_key(k(KeyCode::End));
        ed.handle_key(k(KeyCode::Esc)); // prompt
        // Move to "Cancel" (button 2) and confirm → dismiss, still editing.
        ed.handle_key(k(KeyCode::Left)); // 0 → 2 (wraps)
        assert!(matches!(ed.handle_key(k(KeyCode::Enter)), ThemeEditorSignal::Stay));
        assert!(matches!(ed.overlay, Overlay::None));
        assert!(ed.dirty(), "edits preserved");
        // Esc on the prompt itself also cancels the exit.
        ed.handle_key(k(KeyCode::Esc)); // prompt again
        assert!(matches!(ed.overlay, Overlay::ConfirmExit { .. }));
        ed.handle_key(k(KeyCode::Esc)); // dismiss prompt
        assert!(matches!(ed.overlay, Overlay::None));
    }

    #[test]
    fn cancel_button_also_prompts_when_dirty() {
        let mut ed = ThemeEditor::new("Midnight Commander", true);
        ed.handle_key(k(KeyCode::Right));
        ed.handle_key(k(KeyCode::End));
        ed.focus = Focus::Buttons;
        ed.button = 2; // Cancel
        assert!(matches!(ed.handle_key(k(KeyCode::Enter)), ThemeEditorSignal::Stay));
        assert!(matches!(ed.overlay, Overlay::ConfirmExit { .. }));
    }

    #[test]
    fn mouse_click_selects_a_list_item() {
        let mut ed = ThemeEditor::new("Midnight Commander", true);
        let _t = rendered(&mut ed);
        ed.handle_mouse(mouse_down(ed.z_list.x + 1, ed.z_list.y + 3));
        assert_eq!(ed.focus, Focus::List);
        assert_eq!(ed.item, ed.item_top + 3);
    }

    #[test]
    fn mouse_wheel_scrolls_the_list() {
        let mut ed = ThemeEditor::new("Midnight Commander", true);
        let _t = rendered(&mut ed);
        let (x, y) = (ed.z_list.x + 1, ed.z_list.y + 1);
        ed.handle_mouse(wheel(x, y, true));
        ed.handle_mouse(wheel(x, y, true));
        assert_eq!(ed.item, 2);
        ed.handle_mouse(wheel(x, y, false));
        assert_eq!(ed.item, 1);
        // The wheel elsewhere does nothing.
        ed.handle_mouse(wheel(0, 0, true));
        assert_eq!(ed.item, 1);
    }

    #[test]
    fn mouse_click_on_a_gauge_sets_that_channel() {
        let mut ed = ThemeEditor::new("Midnight Commander", true);
        let _t = rendered(&mut ed);
        let inner = ed.z_color;
        // Second gauge row = Green; click near the right end for a high value.
        ed.handle_mouse(mouse_down(inner.x + inner.width - 2, inner.y + 2));
        assert_eq!(ed.focus, Focus::Color);
        assert_eq!(ed.channel, 1);
        assert!(rgb_of(ed.spec.color_at(ed.item)).1 > 180, "green raised by the click");
        assert!(ed.dirty());
    }

    #[test]
    fn mouse_click_on_a_button_activates_it() {
        let mut ed = ThemeEditor::new("Midnight Commander", true);
        ed.handle_key(k(KeyCode::Right));
        ed.handle_key(k(KeyCode::End)); // dirty
        let _t = rendered(&mut ed);
        let b = ed.z_buttons[0]; // Save
        assert!(matches!(ed.handle_mouse(mouse_down(b.x + 1, b.y)), ThemeEditorSignal::SaveAndClose(_)));
    }

    #[test]
    fn mouse_click_picks_a_theme() {
        let mut ed = ThemeEditor::new("Midnight Commander", true);
        let _t = rendered(&mut ed);
        // Right half of the picker → next theme.
        let p = ed.z_picker;
        ed.handle_mouse(mouse_down(p.x + p.width - 1, p.y));
        assert_eq!(ed.picker, 1);
        assert_eq!(ed.focus, Focus::Picker);
    }

    #[test]
    fn mouse_click_on_exit_prompt_discards() {
        let mut ed = ThemeEditor::new("Midnight Commander", true);
        ed.handle_key(k(KeyCode::Right));
        ed.handle_key(k(KeyCode::End));
        ed.handle_key(k(KeyCode::Esc)); // ConfirmExit overlay
        let _t = rendered(&mut ed);
        let r = ed.z_overlay[1]; // Discard
        assert!(matches!(ed.handle_mouse(mouse_down(r.x + 1, r.y)), ThemeEditorSignal::Close));
    }

    fn buffer_text(t: &Terminal<TestBackend>) -> String {
        let b = t.backend().buffer();
        let mut s = String::new();
        for y in 0..b.area.height {
            for x in 0..b.area.width {
                s.push_str(b[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    #[test]
    fn renders_chrome_and_all_preview_kinds() {
        let theme = Theme::mc();
        for item in [0usize, dialog_item(), editor_item()] {
            let mut ed = ThemeEditor::new("Midnight Commander", true);
            ed.item = item;
            let mut t = Terminal::new(TestBackend::new(120, 32)).unwrap();
            t.draw(|f| render::render(f, f.area(), &mut ed, &theme)).unwrap();
            let text = buffer_text(&t);
            assert!(text.contains("Theme Editor"), "title present");
            assert!(text.contains("Preview"), "preview pane present");
            assert!(text.contains("Save as"), "buttons present");
        }
    }

    #[test]
    fn overlays_and_small_sizes_do_not_panic() {
        let theme = Theme::mc();
        // Non-truecolor swatch picker + both overlays, on a small screen.
        let mut ed = ThemeEditor::new("Midnight Commander", false);
        ed.overlay = Overlay::ConfirmSwitch { target: 1, button: 0 };
        let mut t = Terminal::new(TestBackend::new(48, 14)).unwrap();
        t.draw(|f| render::render(f, f.area(), &mut ed, &theme)).unwrap();
        assert!(buffer_text(&t).contains("Unsaved changes"));

        ed.overlay = Overlay::SaveAs { name: "New".into(), cursor: 3 };
        t.draw(|f| render::render(f, f.area(), &mut ed, &theme)).unwrap();
        assert!(buffer_text(&t).contains("Save theme as"));

        ed.overlay = Overlay::ConfirmExit { button: 0 };
        t.draw(|f| render::render(f, f.area(), &mut ed, &theme)).unwrap();
        let text = buffer_text(&t);
        assert!(text.contains("Unsaved changes") && text.contains("Discard"));
    }
}
