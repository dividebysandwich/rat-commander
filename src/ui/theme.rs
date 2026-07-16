//! Color themes.
//!
//! A [`Palette`] is a classic 16-ANSI-color terminal scheme (plus bg/fg). The
//! [`Theme`] is built from a palette via [`Theme::from_palette`], mapping the
//! palette onto every UI element. A curated set of well-known schemes from
//! terminalcolors.com is provided in [`PALETTES`]; more can be added by
//! appending palette literals.

use ratatui::style::{Color, Modifier, Style};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{LazyLock, RwLock};

const fn rgb(h: u32) -> Color {
    Color::Rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
}

/// The signature Rat/Midnight Commander teal used for the selection bar and
/// menu / function-key bars (matching the real program).
#[allow(dead_code)] // referenced by theme tests
const MC_TEAL: Color = rgb(0x00a3a3);

/// A 16-color terminal palette plus background/foreground.
#[derive(Clone, Copy)]
// The full 16-color ANSI model; the current styles don't read every slot.
#[allow(dead_code)]
pub struct Palette {
    pub name: &'static str,
    pub bg: Color,
    pub fg: Color,
    pub black: Color,
    pub red: Color,
    pub green: Color,
    pub yellow: Color,
    pub blue: Color,
    pub magenta: Color,
    pub cyan: Color,
    pub white: Color,
    pub bright_black: Color,
    pub bright_red: Color,
    pub bright_green: Color,
    pub bright_yellow: Color,
    pub bright_blue: Color,
    pub bright_magenta: Color,
    pub bright_cyan: Color,
    pub bright_white: Color,
}

// ---------------------------------------------------------------------------
// User-editable themes (themes.toml)
// ---------------------------------------------------------------------------

/// A theme stored in `themes.toml`: an explicit color for every UI element
/// (background/foreground pairs where applicable). This is the form edited by
/// the user and used at runtime — colors map straight onto the [`Theme`] with no
/// hue mixing. The built-in [`PALETTES`] seed it (their well-known schemes are
/// derived once into these component colors) and are the fallback if the file is
/// missing or invalid. Colors are `#rrggbb` hex.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemeSpec {
    pub name: String,

    // -- Panels --
    #[serde(with = "hex_color")] pub panel_bg: Color,
    #[serde(with = "hex_color")] pub panel_fg: Color,
    /// Body text in the editor/viewer (usually higher contrast than `panel_fg`).
    #[serde(with = "hex_color")] pub text_fg: Color,
    #[serde(with = "hex_color")] pub panel_border: Color,
    #[serde(with = "hex_color")] pub panel_border_active: Color,
    /// Column headers (Name/Size/…).
    #[serde(with = "hex_color")] pub header_fg: Color,

    // -- Cursor (the selection bar over the focused file) --
    #[serde(with = "hex_color")] pub cursor_bg: Color,
    #[serde(with = "hex_color")] pub cursor_fg: Color,
    /// Cursor on the inactive panel.
    #[serde(with = "hex_color")] pub cursor_inactive_bg: Color,
    #[serde(with = "hex_color")] pub cursor_inactive_fg: Color,

    // -- File-type name colors --
    #[serde(with = "hex_color")] pub marked_fg: Color,
    #[serde(with = "hex_color")] pub dir_fg: Color,
    /// Regular files with no special type. Defaulted for `themes.toml` files
    /// written before this field existed (so they keep loading).
    #[serde(default = "default_file_fg", with = "hex_color")] pub file_fg: Color,
    #[serde(with = "hex_color")] pub exec_fg: Color,
    #[serde(with = "hex_color")] pub symlink_fg: Color,
    #[serde(with = "hex_color")] pub archive_fg: Color,
    #[serde(with = "hex_color")] pub doc_fg: Color,
    #[serde(with = "hex_color")] pub image_fg: Color,
    #[serde(with = "hex_color")] pub media_fg: Color,

    // -- Top menu bar + bottom F-key bar --
    #[serde(with = "hex_color")] pub menubar_bg: Color,
    #[serde(with = "hex_color")] pub menubar_fg: Color,
    #[serde(with = "hex_color")] pub fkey_label_bg: Color,
    #[serde(with = "hex_color")] pub fkey_label_fg: Color,
    #[serde(with = "hex_color")] pub fkey_num_bg: Color,
    #[serde(with = "hex_color")] pub fkey_num_fg: Color,

    // -- Dialogs --
    #[serde(with = "hex_color")] pub dialog_bg: Color,
    #[serde(with = "hex_color")] pub dialog_fg: Color,
    #[serde(with = "hex_color")] pub dialog_title: Color,
    #[serde(with = "hex_color")] pub dialog_border_fg: Color,
    #[serde(with = "hex_color")] pub dialog_border_bg: Color,
    /// Focused control / selected row inside a dialog.
    #[serde(with = "hex_color")] pub dialog_selection_bg: Color,
    #[serde(with = "hex_color")] pub dialog_selection_fg: Color,

    // -- Pulldown menus --
    #[serde(with = "hex_color")] pub menu_bg: Color,
    #[serde(with = "hex_color")] pub menu_fg: Color,
    #[serde(with = "hex_color")] pub menu_selection_bg: Color,
    #[serde(with = "hex_color")] pub menu_selection_fg: Color,
    /// Underlined accelerator letters in menus.
    #[serde(with = "hex_color")] pub hotkey_fg: Color,

    // -- Text inputs + buttons --
    #[serde(with = "hex_color")] pub input_bg: Color,
    #[serde(with = "hex_color")] pub input_fg: Color,
    #[serde(with = "hex_color")] pub button_bg: Color,
    #[serde(with = "hex_color")] pub button_fg: Color,
    #[serde(with = "hex_color")] pub button_focused_bg: Color,
    #[serde(with = "hex_color")] pub button_focused_fg: Color,

    // -- Misc --
    #[serde(with = "hex_color")] pub error_fg: Color,
    /// Text drawn over animated gradient bars.
    #[serde(with = "hex_color")] pub bar_fg: Color,
    /// Animated gradient endpoints (bars/cursor on truecolor terminals).
    #[serde(with = "hex_color")] pub gradient_from: Color,
    #[serde(with = "hex_color")] pub gradient_to: Color,
}

/// Which live-preview surface best exercises a given color, so the visual theme
/// editor can show the relevant chrome while that color is selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewKind {
    /// The two file panels plus the menu bar, function-key bar and pulldown menu.
    Panels,
    /// A demo dialog with a text input and buttons.
    Dialog,
    /// A small editor / viewer with body text.
    Editor,
}

/// One editable color in a [`ThemeSpec`]: a human label and the preview surface
/// it drives. Entries are shown, in order, in the theme editor's item list.
pub struct ThemeFieldMeta {
    pub group: &'static str,
    pub label: &'static str,
    pub preview: PreviewKind,
}

/// Generate the editable-field table and indexed color accessors from a single
/// ordered list, so [`THEME_FIELDS`] and [`ThemeSpec::color_at`] can never drift
/// out of sync.
macro_rules! theme_fields {
    ( $( $group:literal, $label:literal, $preview:ident, $field:ident ; )* ) => {
        /// Every editable color, in item-list display order. The index into this
        /// table matches [`ThemeSpec::color_at`] / [`ThemeSpec::set_color_at`].
        pub static THEME_FIELDS: &[ThemeFieldMeta] = &[
            $( ThemeFieldMeta { group: $group, label: $label, preview: PreviewKind::$preview }, )*
        ];
        impl ThemeSpec {
            /// The color of the editable field at display index `i` (falls back to
            /// the panel background for an out-of-range index).
            pub fn color_at(&self, i: usize) -> Color {
                let mut n = 0usize;
                $( if n == i { return self.$field; } n += 1; )*
                let _ = n;
                self.panel_bg
            }
            /// Replace the color of the editable field at display index `i`.
            pub fn set_color_at(&mut self, i: usize, c: Color) {
                let mut n = 0usize;
                $( if n == i { self.$field = c; return; } n += 1; )*
                let _ = n;
            }
        }
    };
}

theme_fields! {
    // -- Panels & chrome (previewed on the two-panel view) --
    "Panel", "Background", Panels, panel_bg;
    "Panel", "Foreground", Panels, panel_fg;
    "Panel", "Border", Panels, panel_border;
    "Panel", "Active border", Panels, panel_border_active;
    "Panel", "Column header", Panels, header_fg;
    "Cursor", "Background", Panels, cursor_bg;
    "Cursor", "Foreground", Panels, cursor_fg;
    "Cursor", "Inactive background", Panels, cursor_inactive_bg;
    "Cursor", "Inactive foreground", Panels, cursor_inactive_fg;
    "File types", "Marked", Panels, marked_fg;
    "File types", "Directory", Panels, dir_fg;
    "File types", "File", Panels, file_fg;
    "File types", "Executable", Panels, exec_fg;
    "File types", "Symlink", Panels, symlink_fg;
    "File types", "Archive", Panels, archive_fg;
    "File types", "Document", Panels, doc_fg;
    "File types", "Image", Panels, image_fg;
    "File types", "Media", Panels, media_fg;
    "Menu bar", "Background", Panels, menubar_bg;
    "Menu bar", "Foreground", Panels, menubar_fg;
    "Function keys", "Label background", Panels, fkey_label_bg;
    "Function keys", "Label foreground", Panels, fkey_label_fg;
    "Function keys", "Number background", Panels, fkey_num_bg;
    "Function keys", "Number foreground", Panels, fkey_num_fg;
    "Function keys", "Gradient text", Panels, bar_fg;
    "Gradient", "From", Panels, gradient_from;
    "Gradient", "To", Panels, gradient_to;
    "Pulldown menu", "Background", Panels, menu_bg;
    "Pulldown menu", "Foreground", Panels, menu_fg;
    "Pulldown menu", "Selection background", Panels, menu_selection_bg;
    "Pulldown menu", "Selection foreground", Panels, menu_selection_fg;
    "Pulldown menu", "Hotkey letter", Panels, hotkey_fg;
    // -- Dialogs, inputs & buttons (previewed on the demo dialog) --
    "Dialog", "Background", Dialog, dialog_bg;
    "Dialog", "Foreground", Dialog, dialog_fg;
    "Dialog", "Title", Dialog, dialog_title;
    "Dialog", "Border", Dialog, dialog_border_fg;
    "Dialog", "Border background", Dialog, dialog_border_bg;
    "Dialog", "Selection background", Dialog, dialog_selection_bg;
    "Dialog", "Selection foreground", Dialog, dialog_selection_fg;
    "Dialog", "Error text", Dialog, error_fg;
    "Input", "Background", Dialog, input_bg;
    "Input", "Foreground", Dialog, input_fg;
    "Button", "Background", Dialog, button_bg;
    "Button", "Foreground", Dialog, button_fg;
    "Button", "Focused background", Dialog, button_focused_bg;
    "Button", "Focused foreground", Dialog, button_focused_fg;
    // -- Editor / viewer --
    "Editor / Viewer", "Body text", Editor, text_fg;
}

/// A clone of every active theme spec, in file order — the editable source for
/// the visual theme editor's picker.
pub fn active_specs() -> Vec<ThemeSpec> {
    ACTIVE.read().unwrap().clone()
}

/// Insert or replace `spec` (matched by name) in the active set and persist the
/// whole set to `themes.toml`. The in-memory set is updated even if the file
/// write fails; the error string is for surfacing to the user.
pub fn save_spec(spec: ThemeSpec) -> Result<(), String> {
    {
        let mut active = ACTIVE.write().unwrap();
        let key = norm_name(&spec.name);
        match active.iter_mut().find(|p| norm_name(&p.name) == key) {
            Some(slot) => *slot = spec,
            None => active.push(spec),
        }
    }
    let specs = active_specs();
    let path = crate::config::paths::themes_file().ok_or("no config directory available")?;
    write_themes(&path, &specs).map_err(|e| e.to_string())
}

/// Extract the per-component colors from a (derived) [`Theme`] into a [`ThemeSpec`]
/// — how the built-in schemes become editable component colors in `themes.toml`.
fn theme_to_spec(t: &Theme) -> ThemeSpec {
    let fg = |s: &Style| s.fg.unwrap_or(t.panel_fg);
    let bg = |s: &Style| s.bg.unwrap_or(t.panel_bg);
    ThemeSpec {
        name: t.name.clone(),
        panel_bg: t.panel_bg,
        panel_fg: t.panel_fg,
        text_fg: t.text_fg,
        panel_border: t.panel_border,
        panel_border_active: t.panel_border_active,
        header_fg: t.header_fg,
        cursor_bg: bg(&t.cursor),
        cursor_fg: fg(&t.cursor),
        cursor_inactive_bg: bg(&t.cursor_inactive),
        cursor_inactive_fg: fg(&t.cursor_inactive),
        marked_fg: t.marked_fg,
        dir_fg: t.dir_fg,
        file_fg: t.file_fg,
        exec_fg: t.exec_fg,
        symlink_fg: t.symlink_fg,
        archive_fg: t.archive_fg,
        doc_fg: t.doc_fg,
        image_fg: t.image_fg,
        media_fg: t.media_fg,
        menubar_bg: bg(&t.menubar),
        menubar_fg: fg(&t.menubar),
        fkey_label_bg: bg(&t.fkey_label),
        fkey_label_fg: fg(&t.fkey_label),
        fkey_num_bg: bg(&t.fkey_num),
        fkey_num_fg: fg(&t.fkey_num),
        dialog_bg: t.dialog_bg,
        dialog_fg: t.dialog_fg,
        dialog_title: t.dialog_title,
        dialog_border_fg: t.dialog_border_fg,
        dialog_border_bg: t.dialog_border_bg,
        dialog_selection_bg: bg(&t.dialog_selection),
        dialog_selection_fg: fg(&t.dialog_selection),
        menu_bg: t.menu_bg,
        menu_fg: t.menu_fg,
        menu_selection_bg: bg(&t.menu_selection),
        menu_selection_fg: fg(&t.menu_selection),
        hotkey_fg: t.hotkey_fg,
        input_bg: t.input_bg,
        input_fg: t.input_fg,
        button_bg: bg(&t.button),
        button_fg: fg(&t.button),
        button_focused_bg: bg(&t.button_focused),
        button_focused_fg: fg(&t.button_focused),
        error_fg: t.error_fg,
        bar_fg: t.bar_fg,
        gradient_from: Color::Rgb(t.grad_a.0, t.grad_a.1, t.grad_a.2),
        gradient_to: Color::Rgb(t.grad_b.0, t.grad_b.1, t.grad_b.2),
    }
}

/// TOML wrapper: `[[theme]]` array-of-tables.
#[derive(Default, Serialize, Deserialize)]
struct ThemesFile {
    #[serde(default, rename = "theme")]
    theme: Vec<ThemeSpec>,
}

/// (De)serialize a [`Color`] as a `#rrggbb` hex string.
mod hex_color {
    use ratatui::style::Color;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(c: &Color, s: S) -> Result<S::Ok, S::Error> {
        let (r, g, b) = super::to_rgb(*c);
        s.serialize_str(&format!("#{r:02x}{g:02x}{b:02x}"))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Color, D::Error> {
        let s = String::deserialize(d)?;
        super::parse_hex(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("expected #rrggbb color, got {s:?}")))
    }
}

/// Default for [`ThemeSpec::file_fg`] (added after the initial release) so older
/// `themes.toml` files without the field still deserialize; a neutral light gray
/// like most themes' normal-file text. Regenerated presets set a per-theme value.
fn default_file_fg() -> Color {
    rgb(0xc6c6c6)
}

/// Parse `#rrggbb` / `rrggbb` / `0xrrggbb` into an RGB [`Color`].
fn parse_hex(s: &str) -> Option<Color> {
    let s = s.trim();
    let s = s.strip_prefix('#').or_else(|| s.strip_prefix("0x")).unwrap_or(s);
    if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let n = u32::from_str_radix(s, 16).ok()?;
    Some(rgb(n))
}

const THEMES_HEADER: &str = "\
# Rat Commander themes. Each [[theme]] sets an explicit #rrggbb color for every
# UI element (e.g. menu_bg, dialog_bg, dialog_border_fg, input_bg, cursor_bg).
# Edit any preset, add your own [[theme]] blocks, then pick one in Options →
# Settings (the Theme field). Saving applies the change at once. Delete this file
# to regenerate the presets.\n\n";

/// The signature Rat Commander theme (the default): a deep-blue two-panel look
/// with a teal selection bar and light "paper" dialogs. Defined with explicit
/// component colors rather than derived from an ANSI palette.
fn rat_commander_spec() -> ThemeSpec {
    ThemeSpec {
        name: "Rat Commander".to_string(),
        panel_bg: rgb(0x0000cd),
        panel_fg: rgb(0xc6c6c6),
        text_fg: rgb(0xd7d7d7),
        panel_border: rgb(0x5959ca),
        panel_border_active: rgb(0x55ffff),
        header_fg: rgb(0xffff55),
        cursor_bg: rgb(0x00a3a3),
        cursor_fg: rgb(0x000000),
        cursor_inactive_bg: rgb(0x1818cc),
        cursor_inactive_fg: rgb(0xc6c6c6),
        marked_fg: rgb(0xffff55),
        dir_fg: rgb(0xc6c6c6),
        file_fg: rgb(0xc6c6c6),
        exec_fg: rgb(0x55ff55),
        symlink_fg: rgb(0x55ffff),
        archive_fg: rgb(0xff55ff),
        doc_fg: rgb(0xaa5500),
        image_fg: rgb(0x55ffff),
        media_fg: rgb(0x55ff55),
        menubar_bg: rgb(0x00a3a3),
        menubar_fg: rgb(0x0000cd),
        fkey_label_bg: rgb(0x00a3a3),
        fkey_label_fg: rgb(0x0000cd),
        fkey_num_bg: rgb(0x000000),
        fkey_num_fg: rgb(0xffffff),
        dialog_bg: rgb(0xc6c6c6),
        dialog_fg: rgb(0x000000),
        dialog_title: rgb(0x0000cc),
        dialog_border_fg: rgb(0x0000cc),
        dialog_border_bg: rgb(0xc6c6c6),
        dialog_selection_bg: rgb(0x0dcdcd),
        dialog_selection_fg: rgb(0x000000),
        menu_bg: rgb(0x0dcdcd),
        menu_fg: rgb(0xffffff),
        menu_selection_bg: rgb(0x000000),
        menu_selection_fg: rgb(0xffffff),
        hotkey_fg: rgb(0xffff00),
        input_bg: rgb(0x0dcdcd),
        input_fg: rgb(0x000000),
        button_bg: rgb(0xc6c6c6),
        button_fg: rgb(0x000000),
        button_focused_bg: rgb(0x0dcdcd),
        button_focused_fg: rgb(0x000000),
        error_fg: rgb(0xff5555),
        bar_fg: rgb(0x000000),
        gradient_from: rgb(0x009c9c),
        gradient_to: rgb(0x12baba),
    }
}

/// The classic Midnight Commander look: a lighter blue panel with white text and
/// a bright cyan menu/status bar.
fn midnight_commander_spec() -> ThemeSpec {
    ThemeSpec {
        name: "Midnight Commander".to_string(),
        panel_bg: rgb(0x0d73cc),
        panel_fg: rgb(0xffffff),
        text_fg: rgb(0xd7d7d7),
        panel_border: rgb(0xffffff),
        panel_border_active: rgb(0xffffff),
        header_fg: rgb(0xffff55),
        cursor_bg: rgb(0x0dcdcd),
        cursor_fg: rgb(0x000000),
        cursor_inactive_bg: rgb(0x0d73cc),
        cursor_inactive_fg: rgb(0xffffff),
        marked_fg: rgb(0xffff55),
        dir_fg: rgb(0xffffff),
        file_fg: rgb(0xd2d2d2),
        exec_fg: rgb(0x55ff55),
        symlink_fg: rgb(0x55ffff),
        archive_fg: rgb(0xff55ff),
        doc_fg: rgb(0xe61000),
        image_fg: rgb(0x55ffff),
        media_fg: rgb(0x55ff55),
        menubar_bg: rgb(0x0dcdcd),
        menubar_fg: rgb(0x000000),
        fkey_label_bg: rgb(0x0dcdcd),
        fkey_label_fg: rgb(0x0000cd),
        fkey_num_bg: rgb(0x000000),
        fkey_num_fg: rgb(0xffffff),
        dialog_bg: rgb(0xc6c6c6),
        dialog_fg: rgb(0x000000),
        dialog_title: rgb(0x0d73cc),
        dialog_border_fg: rgb(0x000000),
        dialog_border_bg: rgb(0xc6c6c6),
        dialog_selection_bg: rgb(0x0dcdcd),
        dialog_selection_fg: rgb(0x000000),
        menu_bg: rgb(0x0dcdcd),
        menu_fg: rgb(0xffffff),
        menu_selection_bg: rgb(0x000000),
        menu_selection_fg: rgb(0xffffff),
        hotkey_fg: rgb(0xffff00),
        input_bg: rgb(0x0dcdcd),
        input_fg: rgb(0x000000),
        button_bg: rgb(0xc6c6c6),
        button_fg: rgb(0x000000),
        button_focused_bg: rgb(0x0dcdcd),
        button_focused_fg: rgb(0x000000),
        error_fg: rgb(0xff5555),
        bar_fg: rgb(0x000000),
        gradient_from: rgb(0x0dcdcd),
        gradient_to: rgb(0x0dcdcd),
    }
}

/// A darker Midnight Commander variant: deep indigo panels with a teal accent.
fn midnight_commander_dark_spec() -> ThemeSpec {
    ThemeSpec {
        name: "Midnight Commander Dark".to_string(),
        panel_bg: rgb(0x1818d4),
        panel_fg: rgb(0xe8e8e8),
        text_fg: rgb(0xefefef),
        panel_border: rgb(0x7676dd),
        panel_border_active: rgb(0x4cffff),
        header_fg: rgb(0xffff44),
        cursor_bg: rgb(0x00a3a3),
        cursor_fg: rgb(0x000000),
        cursor_inactive_bg: rgb(0x3131d6),
        cursor_inactive_fg: rgb(0xe8e8e8),
        marked_fg: rgb(0xffff44),
        dir_fg: rgb(0x4cffff),
        file_fg: rgb(0xe8e8e8),
        exec_fg: rgb(0x4cff4c),
        symlink_fg: rgb(0x4cffff),
        archive_fg: rgb(0xff55ff),
        doc_fg: rgb(0xe8e8e8),
        image_fg: rgb(0x4cffff),
        media_fg: rgb(0x4cff4c),
        menubar_bg: rgb(0x00a3a3),
        menubar_fg: rgb(0x1818d4),
        fkey_label_bg: rgb(0x00a3a3),
        fkey_label_fg: rgb(0x1818d4),
        fkey_num_bg: rgb(0x1818d4),
        fkey_num_fg: rgb(0xffffff),
        dialog_bg: rgb(0x3131d6),
        dialog_fg: rgb(0xe8e8e8),
        dialog_title: rgb(0x4cffff),
        dialog_border_fg: rgb(0x4cffff),
        dialog_border_bg: rgb(0x3131d6),
        dialog_selection_bg: rgb(0x4cffff),
        dialog_selection_fg: rgb(0x1818d4),
        menu_bg: rgb(0x0e0ed1),
        menu_fg: rgb(0xffffff),
        menu_selection_bg: rgb(0x6c6cff),
        menu_selection_fg: rgb(0xffffff),
        hotkey_fg: rgb(0xffff44),
        input_bg: rgb(0x0000cc),
        input_fg: rgb(0xffffff),
        button_bg: rgb(0x3131d6),
        button_fg: rgb(0xe8e8e8),
        button_focused_bg: rgb(0x4cffff),
        button_focused_fg: rgb(0x1818d4),
        error_fg: rgb(0xff6464),
        bar_fg: rgb(0x000000),
        gradient_from: rgb(0x009c9c),
        gradient_to: rgb(0x12baba),
    }
}

/// The built-in presets as component specs. The three Rat/Midnight Commander
/// themes are defined explicitly (above); every other well-known scheme is
/// derived once from its ANSI [`Palette`] via [`Theme::from_ansi`]. These seed
/// `themes.toml` and serve as the fallback set. `Rat Commander` is first, so it
/// is the default ([`Theme::mc`], [`BUILTIN`]`[0]`).
fn builtin_specs() -> Vec<ThemeSpec> {
    let mut specs =
        vec![rat_commander_spec(), midnight_commander_spec(), midnight_commander_dark_spec()];
    specs.extend(PALETTES.iter().map(|p| theme_to_spec(&Theme::from_ansi(p, true))));
    specs
}

static BUILTIN: LazyLock<Vec<ThemeSpec>> = LazyLock::new(builtin_specs);
/// The themes currently in effect (built-ins until `themes.toml` is loaded).
static ACTIVE: LazyLock<RwLock<Vec<ThemeSpec>>> = LazyLock::new(|| RwLock::new(builtin_specs()));

/// Replace the active theme set (ignored if empty).
fn set_palettes(specs: Vec<ThemeSpec>) {
    if !specs.is_empty() {
        *ACTIVE.write().unwrap() = specs;
    }
}

/// Add fields introduced after a user's `themes.toml` was first written, so an
/// older file keeps working and gains sensible values on upgrade. Currently the
/// only such field is `file_fg` (the regular-file color): it defaults to each
/// theme's own `panel_fg` — exactly what normal files rendered as before it
/// became themable — so both the presets and any user-made themes look unchanged.
/// Returns the migrated TOML when anything was added, else `None`.
fn migrate_theme_toml(text: &str) -> Option<String> {
    let mut doc = toml::from_str::<toml::Table>(text).ok()?;
    let themes = doc.get_mut("theme")?.as_array_mut()?;
    let mut changed = false;
    for entry in themes.iter_mut() {
        let Some(tbl) = entry.as_table_mut() else { continue };
        if !tbl.contains_key("file_fg") {
            let fallback = tbl
                .get("panel_fg")
                .cloned()
                .unwrap_or_else(|| toml::Value::String("#c6c6c6".to_string()));
            tbl.insert("file_fg".to_string(), fallback);
            changed = true;
        }
    }
    changed.then(|| toml::to_string(&doc).ok()).flatten()
}

/// Load `themes.toml` (generating it from the presets if absent) and make those
/// palettes active. Call once at startup, before deriving the initial theme.
pub fn load_user_themes() {
    let Some(path) = crate::config::paths::themes_file() else {
        return;
    };
    if !path.exists() {
        let _ = write_themes(&path, &builtin_specs());
        return; // built-ins are already active by default
    }
    let Ok(text) = std::fs::read_to_string(&path) else {
        return; // keep the built-ins on a read error rather than clobbering it
    };
    // Upgrade an older file in place: add any newly-introduced color fields with
    // appearance-preserving values, then parse. Keep the built-ins on a parse
    // error rather than overwriting the user's file.
    let migrated = migrate_theme_toml(&text);
    let src = migrated.as_deref().unwrap_or(&text);
    if let Ok(tf) = toml::from_str::<ThemesFile>(src)
        && !tf.theme.is_empty()
    {
        set_palettes(tf.theme.clone());
        // Persist the migration so the file now carries the new fields.
        if migrated.is_some() {
            let _ = write_themes(&path, &tf.theme);
        }
    }
}

/// Re-read `themes.toml` and make it active (after the user edits it). Returns
/// the number of themes, or an error message for a malformed file.
pub fn reload_user_themes() -> Result<usize, String> {
    let path = crate::config::paths::themes_file().ok_or("no config directory available")?;
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let tf: ThemesFile = toml::from_str(&text).map_err(|e| e.to_string())?;
    if tf.theme.is_empty() {
        return Err("themes.toml has no [[theme]] entries".to_string());
    }
    let n = tf.theme.len();
    set_palettes(tf.theme);
    Ok(n)
}

fn write_themes(path: &Path, specs: &[ThemeSpec]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tf = ThemesFile { theme: specs.to_vec() };
    let body = toml::to_string_pretty(&tf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, format!("{THEMES_HEADER}{body}"))
}

/// Centralized styles for every UI element, derived from a palette.
#[derive(Clone)]
pub struct Theme {
    pub name: String,
    pub truecolor: bool,
    pub panel_bg: Color,
    pub panel_fg: Color,
    /// Higher-contrast foreground for dense text views (editor/viewer), pushed
    /// away from the background so body text reads crisply.
    pub text_fg: Color,
    pub panel_border: Color,
    pub panel_border_active: Color,
    pub header_fg: Color,
    pub cursor: Style,
    pub cursor_inactive: Style,
    pub cursor_fg: Color,
    pub marked_fg: Color,
    pub dir_fg: Color,
    pub file_fg: Color,
    pub exec_fg: Color,
    pub symlink_fg: Color,
    /// File-type accent colors (by extension): archives, documents, images, and
    /// audio/video media.
    pub archive_fg: Color,
    pub doc_fg: Color,
    pub image_fg: Color,
    pub media_fg: Color,
    pub menubar: Style,
    pub fkey_label: Style,
    pub fkey_num: Style,
    pub dialog_bg: Color,
    pub dialog_fg: Color,
    pub dialog_title: Color,
    /// The dialog's border (frame) — foreground and background, separate from the
    /// interior so a theme can outline dialogs distinctly.
    pub dialog_border_fg: Color,
    pub dialog_border_bg: Color,
    /// Highlight style for a focused control / selected row inside a dialog.
    pub dialog_selection: Style,
    /// Background/foreground of pulldown menu dropdowns (kept distinct from
    /// dialogs so a theme can dress them differently).
    pub menu_bg: Color,
    pub menu_fg: Color,
    /// Highlight style for the selected item in a pulldown menu.
    pub menu_selection: Style,
    /// Foreground for menu/menu-bar **hotkey** letters (the underlined accelerator
    /// char), drawn over both the bar and the dropdown.
    pub hotkey_fg: Color,
    pub input_bg: Color,
    pub input_fg: Color,
    pub button: Style,
    pub button_focused: Style,
    pub error_fg: Color,
    /// Readable foreground for text drawn over a gradient bar.
    pub bar_fg: Color,
    /// Animation frame (set per-frame by the renderer).
    pub anim: usize,
    /// Whether gradients should animate (slide) this frame.
    pub animated: bool,
    /// Gradient endpoints (RGB) used for bars when `truecolor` is set.
    grad_a: (u8, u8, u8),
    grad_b: (u8, u8, u8),
}

impl Theme {
    /// The default theme (classic Midnight Commander blue).
    pub fn mc() -> Self {
        Theme::from_spec(&BUILTIN[0], true)
    }

    /// Build a [`Theme`] from explicit per-component colors (the `themes.toml`
    /// form). `truecolor` only governs gradient animation; the colors are used
    /// as-is. Structural emphasis (bold cursor/selection/buttons) is applied here.
    pub fn from_spec(s: &ThemeSpec, truecolor: bool) -> Self {
        let bg_fg = |bg: Color, fg: Color| Style::default().bg(bg).fg(fg);
        let bold = |bg: Color, fg: Color| bg_fg(bg, fg).add_modifier(Modifier::BOLD);
        Theme {
            name: s.name.clone(),
            truecolor,
            panel_bg: s.panel_bg,
            panel_fg: s.panel_fg,
            text_fg: s.text_fg,
            panel_border: s.panel_border,
            panel_border_active: s.panel_border_active,
            header_fg: s.header_fg,
            cursor: bold(s.cursor_bg, s.cursor_fg),
            cursor_inactive: bg_fg(s.cursor_inactive_bg, s.cursor_inactive_fg),
            cursor_fg: s.cursor_fg,
            marked_fg: s.marked_fg,
            dir_fg: s.dir_fg,
            file_fg: s.file_fg,
            exec_fg: s.exec_fg,
            symlink_fg: s.symlink_fg,
            archive_fg: s.archive_fg,
            doc_fg: s.doc_fg,
            image_fg: s.image_fg,
            media_fg: s.media_fg,
            menubar: bg_fg(s.menubar_bg, s.menubar_fg),
            fkey_label: bg_fg(s.fkey_label_bg, s.fkey_label_fg),
            fkey_num: bold(s.fkey_num_bg, s.fkey_num_fg),
            dialog_bg: s.dialog_bg,
            dialog_fg: s.dialog_fg,
            dialog_title: s.dialog_title,
            dialog_border_fg: s.dialog_border_fg,
            dialog_border_bg: s.dialog_border_bg,
            dialog_selection: bold(s.dialog_selection_bg, s.dialog_selection_fg),
            menu_bg: s.menu_bg,
            menu_fg: s.menu_fg,
            menu_selection: bold(s.menu_selection_bg, s.menu_selection_fg),
            hotkey_fg: s.hotkey_fg,
            input_bg: s.input_bg,
            input_fg: s.input_fg,
            button: bg_fg(s.button_bg, s.button_fg),
            button_focused: bold(s.button_focused_bg, s.button_focused_fg),
            error_fg: s.error_fg,
            bar_fg: s.bar_fg,
            anim: 0,
            animated: false,
            grad_a: to_rgb(s.gradient_from),
            grad_b: to_rgb(s.gradient_to),
        }
    }

    /// Derive the default component colors for a built-in ANSI scheme. Used only
    /// to seed the editable [`ThemeSpec`]s; the runtime builds themes via
    /// [`from_spec`](Self::from_spec).
    fn from_ansi(p: &Palette, truecolor: bool) -> Self {
        let surface = if truecolor {
            mix(p.bg, p.fg, 0.12)
        } else {
            p.bright_black
        };
        // Derived themes use a gradient-friendly bright-blue cursor. (The teal
        // Commander cursor lives in the explicit specs, not here.)
        let (cursor_bg, cursor_fg) =
            (p.bright_blue, best_contrast(p.bright_blue, p.bg, p.bright_white));
        // Borders/column separators must contrast with the panel background on
        // every theme (e.g. MC's blue border would vanish on its blue bg), so
        // derive them from a bg↔fg mix rather than a palette hue.
        let border = mix(p.bg, p.fg, 0.45);

        // Dialogs sit on a neutral, slightly elevated surface; menus get a
        // clearly distinct blue-tinted panel so the two read as different chrome
        // on every theme. (The MC theme overrides both further down.)
        let dialog_surface = surface;
        let menu_surface = mix(p.bg, p.blue, 0.40);

        // The top menu bar and bottom F-key bar are drawn from the middle of the
        // theme's own accent gradient (the same colour the truecolor bars fade
        // through) so the chrome matches the theme instead of a stock cyan.
        let (bar_bg, bar_bg_fg) = {
            let mid = mix(p.bright_blue, p.bright_magenta, 0.5);
            (mid, best_contrast(mid, p.black, p.bright_white))
        };

        let mut theme = Theme {
            name: p.name.to_string(),
            truecolor,
            panel_bg: p.bg,
            panel_fg: p.fg,
            text_fg: contrast_text(p.fg, p.bg),
            panel_border: border,
            panel_border_active: p.bright_cyan,
            header_fg: p.bright_yellow,
            cursor: Style::default()
                .bg(cursor_bg)
                .fg(cursor_fg)
                .add_modifier(Modifier::BOLD),
            cursor_inactive: Style::default().bg(surface).fg(p.fg),
            cursor_fg,
            marked_fg: p.bright_yellow,
            dir_fg: p.bright_blue,
            // Regular files match the normal panel text by default (what they
            // rendered as before this became its own themable color).
            file_fg: p.fg,
            exec_fg: p.bright_green,
            symlink_fg: p.bright_cyan,
            // Archives = purple, documents = (dark) yellow, images = cyan,
            // audio/video = green — matching Midnight Commander's scheme.
            archive_fg: p.bright_magenta,
            doc_fg: p.yellow,
            image_fg: p.bright_cyan,
            media_fg: p.bright_green,
            menubar: Style::default().bg(bar_bg).fg(bar_bg_fg),
            fkey_label: Style::default().bg(bar_bg).fg(bar_bg_fg),
            // Function-key numbers sit on a solid, contrasting "key cap" so they
            // stand out from the colored label cells.
            fkey_num: Style::default()
                .bg(p.bg)
                .fg(best_contrast(p.bg, p.black, p.bright_white))
                .add_modifier(Modifier::BOLD),
            // Dialogs use a neutral surface with cyan title/selection accents…
            dialog_bg: dialog_surface,
            dialog_fg: p.fg,
            dialog_title: p.bright_cyan,
            // The frame matches the title/interior by default (set after the MC
            // override below, so it tracks any per-theme dialog adjustments).
            dialog_border_fg: p.bright_cyan,
            dialog_border_bg: dialog_surface,
            dialog_selection: Style::default()
                .bg(p.bright_cyan)
                .fg(best_contrast(p.bright_cyan, p.bg, p.bright_white))
                .add_modifier(Modifier::BOLD),
            // …while menus get a distinct blue-tinted panel with a blue
            // selection bar, so the two kinds of chrome read differently.
            menu_bg: menu_surface,
            menu_fg: best_contrast(menu_surface, p.black, p.bright_white),
            menu_selection: Style::default()
                .bg(p.bright_blue)
                .fg(best_contrast(p.bright_blue, p.bg, p.bright_white))
                .add_modifier(Modifier::BOLD),
            hotkey_fg: p.bright_yellow,
            input_bg: p.blue,
            input_fg: best_contrast(p.blue, p.bg, p.bright_white),
            button: Style::default().bg(surface).fg(p.fg),
            button_focused: Style::default()
                .bg(p.bright_cyan)
                .fg(p.bg)
                .add_modifier(Modifier::BOLD),
            error_fg: p.bright_red,
            // Derived themes use a vivid blue→magenta gradient; the text over the
            // bars picks whichever of black/white contrasts with its midpoint.
            bar_fg: best_contrast(mix(p.bright_blue, p.bright_magenta, 0.5), p.black, p.bright_white),
            anim: 0,
            animated: false,
            grad_a: to_rgb(p.bright_blue),
            grad_b: to_rgb(p.bright_magenta),
        };

        // The dialog frame matches the title/interior.
        theme.dialog_border_fg = theme.dialog_title;
        theme.dialog_border_bg = theme.dialog_bg;
        theme
    }

    /// Look up an active theme by name (case-insensitive, ignoring spaces and
    /// dashes), falling back to the default (mc) theme.
    pub fn by_name(name: &str, truecolor: bool) -> Self {
        let key = norm_name(name);
        let spec = ACTIVE
            .read()
            .unwrap()
            .iter()
            .find(|p| norm_name(&p.name) == key)
            .cloned()
            .unwrap_or_else(|| BUILTIN[0].clone());
        Theme::from_spec(&spec, truecolor)
    }

    /// Base style for panel content (background + default foreground).
    pub fn panel_base(&self) -> Style {
        Style::default().bg(self.panel_bg).fg(self.panel_fg)
    }

    /// The gradient color at column `i` of `width` cells. Falls back to a solid
    /// accent color when truecolor is unavailable.
    pub fn gradient_at(&self, i: usize, width: usize) -> Color {
        if !self.truecolor {
            return Color::Rgb(self.grad_a.0, self.grad_a.1, self.grad_a.2);
        }
        let base = if width <= 1 {
            0.0
        } else {
            i as f64 / (width - 1) as f64
        };
        // When animated, slide a triangle wave so the gradient bounces a→b→a
        // and shifts over time; otherwise a static linear a→b ramp.
        let t = if self.animated {
            triangle(base * 1.5 + self.anim as f64 * 0.04)
        } else {
            base
        };
        let r = lerp(self.grad_a.0, self.grad_b.0, t);
        let g = lerp(self.grad_a.1, self.grad_b.1, t);
        let b = lerp(self.grad_a.2, self.grad_b.2, t);
        Color::Rgb(r, g, b)
    }

    /// The gradient color (full RGB) at normalized position `t` in `[0, 1]`,
    /// honoring the animation slide but **not** the `truecolor` gate — the
    /// pixel-graphics raster always has full color available (even on a sixel
    /// terminal that doesn't advertise truecolor cells), so it draws the real
    /// gradient rather than falling back to a solid accent.
    pub fn gradient_rgb(&self, t: f64) -> (u8, u8, u8) {
        let tt = if self.animated {
            triangle(t * 1.5 + self.anim as f64 * 0.04)
        } else {
            t.clamp(0.0, 1.0)
        };
        (
            lerp(self.grad_a.0, self.grad_b.0, tt),
            lerp(self.grad_a.1, self.grad_b.1, tt),
            lerp(self.grad_a.2, self.grad_b.2, tt),
        )
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::mc()
    }
}

fn lerp(a: u8, b: u8, t: f64) -> u8 {
    (a as f64 + (b as f64 - a as f64) * t).round().clamp(0.0, 255.0) as u8
}

/// Triangle wave over period 1: 0 → 1 → 0.
fn triangle(x: f64) -> f64 {
    let f = x - x.floor();
    if f < 0.5 { f * 2.0 } else { 2.0 * (1.0 - f) }
}

fn to_rgb(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (128, 128, 128),
    }
}

/// Mix two colors: `t`=0 → a, `t`=1 → b.
fn mix(a: Color, b: Color, t: f64) -> Color {
    let (ar, ag, ab) = to_rgb(a);
    let (br, bg, bb) = to_rgb(b);
    Color::Rgb(lerp(ar, br, t), lerp(ag, bg, t), lerp(ab, bb, t))
}

/// Pick whichever of `dark`/`light` contrasts better against `bg`.
fn best_contrast(bg: Color, dark: Color, light: Color) -> Color {
    if luma(bg) > 140.0 { dark } else { light }
}

/// Rec. 601 luma (0..=255) of an RGB color.
fn luma(c: Color) -> f64 {
    let (r, g, b) = to_rgb(c);
    0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64
}

/// Ensure `fg` is legible on `bg`. When their luma already differs enough the
/// color is returned unchanged (so accent hues survive on dark surfaces); only
/// when contrast is too low is `fg` blended toward black or white — whichever
/// the background is farther from — until it stands out. Used to keep the
/// per-level heading colors readable on bright dialog backgrounds.
pub(crate) fn readable_on(fg: Color, bg: Color) -> Color {
    const MIN_DIFF: f64 = 96.0;
    let bg_luma = luma(bg);
    let target = if bg_luma < 128.0 {
        Color::Rgb(255, 255, 255)
    } else {
        Color::Rgb(0, 0, 0)
    };
    let mut out = fg;
    let mut t = 0.0;
    while (luma(out) - bg_luma).abs() < MIN_DIFF && t < 1.0 {
        t += 0.2;
        out = mix(fg, target, t);
    }
    out
}

/// A higher-contrast version of `fg` for dense text: nudge it away from the
/// background — brighter on dark backgrounds, darker on light ones — so body
/// text in the editor/viewer reads crisply (it's softer by default for chrome).
fn contrast_text(fg: Color, bg: Color) -> Color {
    let target = if luma(bg) < 128.0 {
        Color::Rgb(255, 255, 255)
    } else {
        Color::Rgb(0, 0, 0)
    };
    mix(fg, target, 0.3)
}

/// Normalize a theme name for matching (lower-case, no spaces/dashes).
fn norm_name(name: &str) -> String {
    name.to_ascii_lowercase().replace([' ', '-', '_'], "")
}

/// All active theme names, in file order (built-ins until `themes.toml` loads).
pub fn palette_names() -> Vec<String> {
    ACTIVE.read().unwrap().iter().map(|p| p.name.clone()).collect()
}

/// Whether an active palette matches `name` (fuzzy, like [`Theme::by_name`]).
#[cfg(test)]
fn has_palette(name: &str) -> bool {
    let key = norm_name(name);
    ACTIVE.read().unwrap().iter().any(|p| norm_name(&p.name) == key)
}

/// Curated terminal color schemes (a subset of terminalcolors.com). Each is a
/// standard 16-ANSI palette; the list is data-driven so more can be appended.
// The Rat/Midnight Commander themes are defined explicitly (see
// `rat_commander_spec` and friends), not derived from an ANSI palette, so they
// are intentionally absent from this list.
pub static PALETTES: &[Palette] = &[
    Palette {
        name: "Dracula",
        bg: rgb(0x282a36), fg: rgb(0xf8f8f2),
        black: rgb(0x21222c), red: rgb(0xff5555), green: rgb(0x50fa7b), yellow: rgb(0xf1fa8c),
        blue: rgb(0xbd93f9), magenta: rgb(0xff79c6), cyan: rgb(0x8be9fd), white: rgb(0xf8f8f2),
        bright_black: rgb(0x6272a4), bright_red: rgb(0xff6e6e), bright_green: rgb(0x69ff94),
        bright_yellow: rgb(0xffffa5), bright_blue: rgb(0xd6acff), bright_magenta: rgb(0xff92df),
        bright_cyan: rgb(0xa4ffff), bright_white: rgb(0xffffff),
    },
    Palette {
        name: "Nord",
        bg: rgb(0x2e3440), fg: rgb(0xd8dee9),
        black: rgb(0x3b4252), red: rgb(0xbf616a), green: rgb(0xa3be8c), yellow: rgb(0xebcb8b),
        blue: rgb(0x81a1c1), magenta: rgb(0xb48ead), cyan: rgb(0x88c0d0), white: rgb(0xe5e9f0),
        bright_black: rgb(0x4c566a), bright_red: rgb(0xbf616a), bright_green: rgb(0xa3be8c),
        bright_yellow: rgb(0xebcb8b), bright_blue: rgb(0x81a1c1), bright_magenta: rgb(0xb48ead),
        bright_cyan: rgb(0x8fbcbb), bright_white: rgb(0xeceff4),
    },
    Palette {
        name: "Gruvbox Dark",
        bg: rgb(0x282828), fg: rgb(0xebdbb2),
        black: rgb(0x282828), red: rgb(0xcc241d), green: rgb(0x98971a), yellow: rgb(0xd79921),
        blue: rgb(0x458588), magenta: rgb(0xb16286), cyan: rgb(0x689d6a), white: rgb(0xa89984),
        bright_black: rgb(0x928374), bright_red: rgb(0xfb4934), bright_green: rgb(0xb8bb26),
        bright_yellow: rgb(0xfabd2f), bright_blue: rgb(0x83a598), bright_magenta: rgb(0xd3869b),
        bright_cyan: rgb(0x8ec07c), bright_white: rgb(0xebdbb2),
    },
    Palette {
        name: "Gruvbox Light",
        bg: rgb(0xfbf1c7), fg: rgb(0x3c3836),
        black: rgb(0xfbf1c7), red: rgb(0xcc241d), green: rgb(0x98971a), yellow: rgb(0xd79921),
        blue: rgb(0x458588), magenta: rgb(0xb16286), cyan: rgb(0x689d6a), white: rgb(0x7c6f64),
        bright_black: rgb(0x928374), bright_red: rgb(0x9d0006), bright_green: rgb(0x79740e),
        bright_yellow: rgb(0xb57614), bright_blue: rgb(0x076678), bright_magenta: rgb(0x8f3f71),
        bright_cyan: rgb(0x427b58), bright_white: rgb(0x3c3836),
    },
    Palette {
        name: "Solarized Dark",
        bg: rgb(0x002b36), fg: rgb(0x839496),
        black: rgb(0x073642), red: rgb(0xdc322f), green: rgb(0x859900), yellow: rgb(0xb58900),
        blue: rgb(0x268bd2), magenta: rgb(0xd33682), cyan: rgb(0x2aa198), white: rgb(0xeee8d5),
        bright_black: rgb(0x586e75), bright_red: rgb(0xcb4b16), bright_green: rgb(0x586e75),
        bright_yellow: rgb(0x657b83), bright_blue: rgb(0x839496), bright_magenta: rgb(0x6c71c4),
        bright_cyan: rgb(0x93a1a1), bright_white: rgb(0xfdf6e3),
    },
    Palette {
        name: "Solarized Light",
        bg: rgb(0xfdf6e3), fg: rgb(0x657b83),
        black: rgb(0x073642), red: rgb(0xdc322f), green: rgb(0x859900), yellow: rgb(0xb58900),
        blue: rgb(0x268bd2), magenta: rgb(0xd33682), cyan: rgb(0x2aa198), white: rgb(0xeee8d5),
        bright_black: rgb(0x002b36), bright_red: rgb(0xcb4b16), bright_green: rgb(0x586e75),
        bright_yellow: rgb(0x657b83), bright_blue: rgb(0x268bd2), bright_magenta: rgb(0x6c71c4),
        bright_cyan: rgb(0x2aa198), bright_white: rgb(0x002b36),
    },
    Palette {
        name: "Tokyo Night",
        bg: rgb(0x1a1b26), fg: rgb(0xc0caf5),
        black: rgb(0x15161e), red: rgb(0xf7768e), green: rgb(0x9ece6a), yellow: rgb(0xe0af68),
        blue: rgb(0x7aa2f7), magenta: rgb(0xbb9af7), cyan: rgb(0x7dcfff), white: rgb(0xa9b1d6),
        bright_black: rgb(0x414868), bright_red: rgb(0xf7768e), bright_green: rgb(0x9ece6a),
        bright_yellow: rgb(0xe0af68), bright_blue: rgb(0x7aa2f7), bright_magenta: rgb(0xbb9af7),
        bright_cyan: rgb(0x7dcfff), bright_white: rgb(0xc0caf5),
    },
    Palette {
        name: "Catppuccin Mocha",
        bg: rgb(0x1e1e2e), fg: rgb(0xcdd6f4),
        black: rgb(0x45475a), red: rgb(0xf38ba8), green: rgb(0xa6e3a1), yellow: rgb(0xf9e2af),
        blue: rgb(0x89b4fa), magenta: rgb(0xf5c2e7), cyan: rgb(0x94e2d5), white: rgb(0xbac2de),
        bright_black: rgb(0x585b70), bright_red: rgb(0xf38ba8), bright_green: rgb(0xa6e3a1),
        bright_yellow: rgb(0xf9e2af), bright_blue: rgb(0x89b4fa), bright_magenta: rgb(0xf5c2e7),
        bright_cyan: rgb(0x94e2d5), bright_white: rgb(0xa6adc8),
    },
    Palette {
        name: "One Dark",
        bg: rgb(0x282c34), fg: rgb(0xabb2bf),
        black: rgb(0x282c34), red: rgb(0xe06c75), green: rgb(0x98c379), yellow: rgb(0xe5c07b),
        blue: rgb(0x61afef), magenta: rgb(0xc678dd), cyan: rgb(0x56b6c2), white: rgb(0xabb2bf),
        bright_black: rgb(0x5c6370), bright_red: rgb(0xe06c75), bright_green: rgb(0x98c379),
        bright_yellow: rgb(0xe5c07b), bright_blue: rgb(0x61afef), bright_magenta: rgb(0xc678dd),
        bright_cyan: rgb(0x56b6c2), bright_white: rgb(0xffffff),
    },
    Palette {
        name: "Tomorrow Night",
        bg: rgb(0x1d1f21), fg: rgb(0xc5c8c6),
        black: rgb(0x1d1f21), red: rgb(0xcc6666), green: rgb(0xb5bd68), yellow: rgb(0xf0c674),
        blue: rgb(0x81a2be), magenta: rgb(0xb294bb), cyan: rgb(0x8abeb7), white: rgb(0xc5c8c6),
        bright_black: rgb(0x969896), bright_red: rgb(0xcc6666), bright_green: rgb(0xb5bd68),
        bright_yellow: rgb(0xf0c674), bright_blue: rgb(0x81a2be), bright_magenta: rgb(0xb294bb),
        bright_cyan: rgb(0x8abeb7), bright_white: rgb(0xffffff),
    },
    Palette {
        name: "Cobalt2",
        bg: rgb(0x122738), fg: rgb(0xffffff),
        black: rgb(0x000000), red: rgb(0xff0000), green: rgb(0x38de21), yellow: rgb(0xffe50a),
        blue: rgb(0x1460d2), magenta: rgb(0xff005d), cyan: rgb(0x00bbbb), white: rgb(0xbbbbbb),
        bright_black: rgb(0x555555), bright_red: rgb(0xf40e17), bright_green: rgb(0x3bd01d),
        bright_yellow: rgb(0xedc809), bright_blue: rgb(0x5555ff), bright_magenta: rgb(0xff55ff),
        bright_cyan: rgb(0x6ae3fa), bright_white: rgb(0xffffff),
    },
    Palette {
        name: "Everforest",
        bg: rgb(0x2d353b), fg: rgb(0xd3c6aa),
        black: rgb(0x475258), red: rgb(0xe67e80), green: rgb(0xa7c080), yellow: rgb(0xdbbc7f),
        blue: rgb(0x7fbbb3), magenta: rgb(0xd699b6), cyan: rgb(0x83c092), white: rgb(0xd3c6aa),
        bright_black: rgb(0x475258), bright_red: rgb(0xe67e80), bright_green: rgb(0xa7c080),
        bright_yellow: rgb(0xdbbc7f), bright_blue: rgb(0x7fbbb3), bright_magenta: rgb(0xd699b6),
        bright_cyan: rgb(0x83c092), bright_white: rgb(0xd3c6aa),
    },
    Palette {
        name: "Ayu",
        bg: rgb(0x0a0e14), fg: rgb(0xb3b1ad),
        black: rgb(0x01060e), red: rgb(0xea6c73), green: rgb(0x91b362), yellow: rgb(0xf9af4f),
        blue: rgb(0x53bdfa), magenta: rgb(0xfae994), cyan: rgb(0x90e1c6), white: rgb(0xc7c7c7),
        bright_black: rgb(0x686868), bright_red: rgb(0xf07178), bright_green: rgb(0xc2d94c),
        bright_yellow: rgb(0xffb454), bright_blue: rgb(0x59c2ff), bright_magenta: rgb(0xffee99),
        bright_cyan: rgb(0x95e6cb), bright_white: rgb(0xffffff),
    },
    Palette {
        name: "Nightfox",
        bg: rgb(0x192330), fg: rgb(0xcdcecf),
        black: rgb(0x393b44), red: rgb(0xc94f6d), green: rgb(0x81b29a), yellow: rgb(0xdbc074),
        blue: rgb(0x719cd6), magenta: rgb(0x9d79d6), cyan: rgb(0x63cdcf), white: rgb(0xdfdfe0),
        bright_black: rgb(0x575860), bright_red: rgb(0xd16983), bright_green: rgb(0x8ebaa4),
        bright_yellow: rgb(0xe0c989), bright_blue: rgb(0x86abdc), bright_magenta: rgb(0xbaa1e2),
        bright_cyan: rgb(0x7ad5d6), bright_white: rgb(0xe4e4e5),
    },
    Palette {
        name: "Rose Pine",
        bg: rgb(0x191724), fg: rgb(0xe0def4),
        black: rgb(0x26233a), red: rgb(0xeb6f92), green: rgb(0x31748f), yellow: rgb(0xf6c177),
        blue: rgb(0x9ccfd8), magenta: rgb(0xc4a7e7), cyan: rgb(0xebbcba), white: rgb(0xe0def4),
        bright_black: rgb(0x6e6a86), bright_red: rgb(0xeb6f92), bright_green: rgb(0x31748f),
        bright_yellow: rgb(0xf6c177), bright_blue: rgb(0x9ccfd8), bright_magenta: rgb(0xc4a7e7),
        bright_cyan: rgb(0xebbcba), bright_white: rgb(0xe0def4),
    },
    Palette {
        name: "GitHub Light",
        bg: rgb(0xffffff), fg: rgb(0x24292e),
        black: rgb(0x24292e), red: rgb(0xd73a49), green: rgb(0x28a745), yellow: rgb(0xdbab09),
        blue: rgb(0x0366d6), magenta: rgb(0x5a32a3), cyan: rgb(0x0598bc), white: rgb(0x6a737d),
        bright_black: rgb(0x959da5), bright_red: rgb(0xcb2431), bright_green: rgb(0x22863a),
        bright_yellow: rgb(0xb08800), bright_blue: rgb(0x005cc5), bright_magenta: rgb(0x5a32a3),
        bright_cyan: rgb(0x3192aa), bright_white: rgb(0xd1d5da),
    },
    // Single-hue themes: every color is within the hue family so the whole UI
    // (cursor, bars, gradient) stays monochrome / amber / green.
    Palette {
        name: "Monochrome",
        bg: rgb(0x000000), fg: rgb(0xc6c6c6),
        black: rgb(0x000000), red: rgb(0x5f5f5f), green: rgb(0x8a8a8a), yellow: rgb(0xa8a8a8),
        blue: rgb(0x6c6c6c), magenta: rgb(0x949494), cyan: rgb(0xb0b0b0), white: rgb(0xc6c6c6),
        bright_black: rgb(0x3a3a3a), bright_red: rgb(0x8a8a8a), bright_green: rgb(0xb0b0b0),
        bright_yellow: rgb(0xffffff), bright_blue: rgb(0xbdbdbd), bright_magenta: rgb(0xf0f0f0),
        bright_cyan: rgb(0xe0e0e0), bright_white: rgb(0xffffff),
    },
    Palette {
        name: "Amber CRT",
        bg: rgb(0x160d00), fg: rgb(0xffb000),
        black: rgb(0x160d00), red: rgb(0xcc7000), green: rgb(0xd98a00), yellow: rgb(0xe0a000),
        blue: rgb(0xb36b00), magenta: rgb(0xc98200), cyan: rgb(0xe0a040), white: rgb(0xffb000),
        bright_black: rgb(0x5a3c00), bright_red: rgb(0xff9030), bright_green: rgb(0xffc060),
        bright_yellow: rgb(0xffd000), bright_blue: rgb(0xffb000), bright_magenta: rgb(0xff8000),
        bright_cyan: rgb(0xffe0a0), bright_white: rgb(0xfff0d0),
    },
    Palette {
        name: "Green CRT",
        bg: rgb(0x001000), fg: rgb(0x33ff33),
        black: rgb(0x001000), red: rgb(0x00aa00), green: rgb(0x11cc11), yellow: rgb(0x66dd33),
        blue: rgb(0x009900), magenta: rgb(0x22bb22), cyan: rgb(0x55dd55), white: rgb(0x33ff33),
        bright_black: rgb(0x004d00), bright_red: rgb(0x55ff55), bright_green: rgb(0x88ff88),
        bright_yellow: rgb(0xaaffaa), bright_blue: rgb(0x55ff55), bright_magenta: rgb(0x00bb00),
        bright_cyan: rgb(0xaaffcc), bright_white: rgb(0xccffcc),
    },
    // Rainbow: every ANSI slot is a different hue of the spectrum (red → orange
    // → yellow → green → blue → indigo → violet) over a deep indigo backdrop, so
    // the file list and gradient bars cycle through the full rainbow.
    Palette {
        name: "Rainbow",
        bg: rgb(0x1a1a2e), fg: rgb(0xf0f0f0),
        black: rgb(0x1a1a2e), red: rgb(0xff3b30), green: rgb(0x34c759), yellow: rgb(0xffcc00),
        blue: rgb(0x007aff), magenta: rgb(0xaf52de), cyan: rgb(0x00c7be), white: rgb(0xf0f0f0),
        bright_black: rgb(0x4a4a6a), bright_red: rgb(0xff6b5e), bright_green: rgb(0x5ee87a),
        bright_yellow: rgb(0xffe14d), bright_blue: rgb(0x4d9fff), bright_magenta: rgb(0xd16bff),
        bright_cyan: rgb(0x4de1d8), bright_white: rgb(0xffffff),
    },
    // Candy: a light, pastel sweet-shop palette — mint greens, caramel yellows,
    // peach oranges and grape purples on a pale candy-pink background. The
    // "bright" tints stay medium-saturated so accents read on the light bg.
    Palette {
        name: "Candy",
        bg: rgb(0xfdeef7), fg: rgb(0x5d4470),
        black: rgb(0x3a2a4a), red: rgb(0xe85d9a), green: rgb(0x3fa86a), yellow: rgb(0xc8881f),
        blue: rgb(0x7b5fd0), magenta: rgb(0xb24fc4), cyan: rgb(0x2fa896), white: rgb(0x5d4470),
        bright_black: rgb(0xa98fc0), bright_red: rgb(0xf26faa), bright_green: rgb(0x4fc47e),
        bright_yellow: rgb(0xd99a1f), bright_blue: rgb(0x8a6fe0), bright_magenta: rgb(0xc45fd6),
        bright_cyan: rgb(0x3fc0a8), bright_white: rgb(0x3a2a4a),
    },
    // Neon: saturated electric blues, cyans, reds and greens glowing against a
    // near-black backdrop.
    Palette {
        name: "Neon",
        bg: rgb(0x0a0a12), fg: rgb(0xe6f7ff),
        black: rgb(0x0a0a12), red: rgb(0xff2d6f), green: rgb(0x39ff14), yellow: rgb(0xffe93b),
        blue: rgb(0x2d9bff), magenta: rgb(0xc724ff), cyan: rgb(0x18f0ff), white: rgb(0xe6f7ff),
        bright_black: rgb(0x2a2a3a), bright_red: rgb(0xff5c8a), bright_green: rgb(0x6dff5c),
        bright_yellow: rgb(0xfff45c), bright_blue: rgb(0x5cb8ff), bright_magenta: rgb(0xe05cff),
        bright_cyan: rgb(0x5cf7ff), bright_white: rgb(0xffffff),
    },
    // Forest: earthy browns and a spread of dark-to-light greens (bark, moss,
    // leaf, sage) over a deep woodland backdrop.
    Palette {
        name: "Forest",
        bg: rgb(0x1a2417), fg: rgb(0xd8e0c8),
        black: rgb(0x14180f), red: rgb(0xb5532e), green: rgb(0x5a8c3a), yellow: rgb(0xb08540),
        blue: rgb(0x4a7d6a), magenta: rgb(0x8a6d4a), cyan: rgb(0x6fa86b), white: rgb(0xd8e0c8),
        bright_black: rgb(0x4a5a3a), bright_red: rgb(0xd57a4a), bright_green: rgb(0x8fc46a),
        bright_yellow: rgb(0xd4a85a), bright_blue: rgb(0x6fa88c), bright_magenta: rgb(0xb08d63),
        bright_cyan: rgb(0x9fd49a), bright_white: rgb(0xeef0e0),
    },
    // Freedom: mostly blues and golds over a deep-navy field, with just a touch
    // of red.
    Palette {
        name: "Freedom",
        bg: rgb(0x0a1a3f), fg: rgb(0xf0f4ff),
        black: rgb(0x081230), red: rgb(0xd83a4a), green: rgb(0x4a9d6a), yellow: rgb(0xffd23f),
        blue: rgb(0x2b6cff), magenta: rgb(0x6d7de0), cyan: rgb(0x3fb0e0), white: rgb(0xf0f4ff),
        bright_black: rgb(0x3a4a6f), bright_red: rgb(0xff5c6a), bright_green: rgb(0x6fc78a),
        bright_yellow: rgb(0xffe066), bright_blue: rgb(0x5c9bff), bright_magenta: rgb(0x8a9bf0),
        bright_cyan: rgb(0x6fd0ff), bright_white: rgb(0xffffff),
    },
    // Movienight: the cinematic teal-and-orange grade — deep orange and cyan
    // playing off each other against a dark theatre backdrop.
    Palette {
        name: "Movienight",
        bg: rgb(0x0d1417), fg: rgb(0xdfe8ea),
        black: rgb(0x0a0f11), red: rgb(0xff6a2b), green: rgb(0x3fa890), yellow: rgb(0xffa033),
        blue: rgb(0x1f9bb3), magenta: rgb(0xe0843f), cyan: rgb(0x22c8d8), white: rgb(0xdfe8ea),
        bright_black: rgb(0x2a3a3f), bright_red: rgb(0xff8c4d), bright_green: rgb(0x4fd0b0),
        bright_yellow: rgb(0xffb84d), bright_blue: rgb(0x33c0d8), bright_magenta: rgb(0xff9a4d),
        bright_cyan: rgb(0x4fe0ee), bright_white: rgb(0xf0f8fa),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readable_on_only_adjusts_low_contrast_colors() {
        // A bright accent on a dark background already contrasts — left as-is.
        let accent = rgb(0xffd75f); // light yellow
        assert_eq!(readable_on(accent, rgb(0x101010)), accent, "kept on a dark bg");

        // The same bright accent on a bright dialog background is illegible, so it
        // is darkened until it stands out.
        let bright_bg = rgb(0xf5f5f5);
        let fixed = readable_on(accent, bright_bg);
        assert_ne!(fixed, accent, "adjusted on a bright bg");
        assert!(
            (luma(fixed) - luma(bright_bg)).abs() >= 96.0,
            "the result has adequate contrast with the background"
        );
    }

    #[test]
    fn editing_one_component_changes_only_that_element() {
        let mut spec = builtin_specs()[0].clone(); // Rat Commander (the default)
        let base = Theme::from_spec(&spec, true);
        // Give the dialog a completely different background — directly, no mixing.
        spec.dialog_bg = rgb(0x123456);
        let edited = Theme::from_spec(&spec, true);
        assert_eq!(edited.dialog_bg, rgb(0x123456), "dialog bg follows the spec verbatim");
        // Unrelated elements are untouched.
        assert_eq!(edited.panel_bg, base.panel_bg);
        assert_eq!(edited.menu_bg, base.menu_bg);
        assert_eq!(edited.cursor.bg, base.cursor.bg);
        assert_eq!(edited.input_bg, base.input_bg);
    }

    #[test]
    fn migration_fills_missing_file_fg_from_panel_fg() {
        // Simulate a pre-upgrade file by stripping the `file_fg` lines.
        let spec = builtin_specs()[0].clone();
        let full = toml::to_string_pretty(&ThemesFile { theme: vec![spec.clone()] }).unwrap();
        let old: String = full
            .lines()
            .filter(|l| !l.trim_start().starts_with("file_fg"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!old.contains("file_fg"), "precondition: field removed");

        let migrated = migrate_theme_toml(&old).expect("an old file should migrate");
        let back: ThemesFile = toml::from_str(&migrated).unwrap();
        assert_eq!(
            back.theme[0].file_fg, spec.panel_fg,
            "file_fg is migrated to the theme's own panel_fg"
        );
        // A file that already has the field is left untouched.
        assert!(migrate_theme_toml(&full).is_none(), "no-op when nothing is missing");
    }

    #[test]
    fn builtin_themes_serialize_and_reparse() {
        let specs = builtin_specs();
        assert!(specs.len() >= 10, "expected the full preset set");
        let body = toml::to_string_pretty(&ThemesFile { theme: specs.clone() }).unwrap();
        let back: ThemesFile = toml::from_str(&body).unwrap();
        assert_eq!(back.theme.len(), specs.len());
        for (a, b) in specs.iter().zip(&back.theme) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.panel_bg, b.panel_bg, "{} panel_bg", a.name);
            assert_eq!(a.menu_bg, b.menu_bg, "{} menu_bg", a.name);
            assert_eq!(a.dialog_border_fg, b.dialog_border_fg, "{} dialog_border_fg", a.name);
            assert_eq!(a.cursor_bg, b.cursor_bg, "{} cursor_bg", a.name);
        }
    }

    #[test]
    fn generated_file_has_header_and_reparses() {
        let path = std::env::temp_dir().join(format!("rc_themes_test_{}.toml", std::process::id()));
        write_themes(&path, &builtin_specs()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# Rat Commander themes"), "has a header comment");
        assert!(text.contains("[[theme]]") && text.contains("dialog_bg = \"#"));
        let tf: ThemesFile = toml::from_str(&text).unwrap();
        assert_eq!(tf.theme.len(), builtin_specs().len());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn hex_parsing_accepts_common_forms() {
        assert_eq!(parse_hex("#00a3a3"), Some(rgb(0x00a3a3)));
        assert_eq!(parse_hex("00a3a3"), Some(rgb(0x00a3a3)));
        assert_eq!(parse_hex("0x00A3A3"), Some(rgb(0x00a3a3)));
        assert_eq!(parse_hex("  #ffffff "), Some(rgb(0xffffff)));
        assert_eq!(parse_hex("#fff"), None, "3-digit not supported");
        assert_eq!(parse_hex("#gggggg"), None);
        assert_eq!(parse_hex(""), None);
    }

    #[test]
    fn palette_lookup_is_fuzzy() {
        assert!(has_palette("Dracula"));
        assert!(has_palette("tokyo night"));
        assert!(has_palette("rose-pine"));
        assert!(!has_palette("nonsense"));
    }

    #[test]
    fn gradient_interpolates_endpoints() {
        let t = Theme::by_name("Dracula", true);
        let a = t.gradient_at(0, 10);
        let b = t.gradient_at(9, 10);
        assert!(matches!(a, Color::Rgb(..)));
        assert_ne!(a, b, "gradient should vary across the width");
    }

    #[test]
    fn no_truecolor_means_solid_bar() {
        let t = Theme::by_name("Nord", false);
        assert_eq!(t.gradient_at(0, 10), t.gradient_at(9, 10));
    }

    #[test]
    fn non_mc_menu_bar_follows_the_accent_gradient_not_raw_cyan() {
        // Non-cyan themes no longer paint the menu/F-key bar with their raw
        // `cyan` palette slot; it sits at the middle of the theme's accent
        // gradient (the colour the truecolor bars fade through).
        for name in ["Dracula", "Nord", "Gruvbox Dark", "Tokyo Night"] {
            let t = Theme::by_name(name, true);
            let p = PALETTES.iter().find(|p| p.name == name).unwrap();
            assert_ne!(t.menubar.bg, Some(p.cyan), "{name} menu bar should not use the raw cyan slot");
            // It sits at the middle of the theme's accent gradient, so the F9 bar
            // reads like the rest of the theme's chrome — and matches the F-key bar.
            assert_eq!(t.menubar.bg, t.fkey_label.bg, "{name} menu and F-key bars match");
            assert_eq!(
                t.menubar.bg,
                Some(mix(p.bright_blue, p.bright_magenta, 0.5)),
                "{name} bar = accent-gradient midpoint",
            );
        }
    }

    #[test]
    fn both_mc_themes_use_signature_teal() {
        for name in ["Rat Commander", "Midnight Commander Dark"] {
            let t = Theme::by_name(name, true);
            assert_eq!(t.cursor.bg, Some(MC_TEAL), "{name} cursor bg");
            assert_eq!(t.cursor.fg, Some(rgb(0x000000)), "{name} cursor fg");
            assert_eq!(t.menubar.bg, Some(MC_TEAL), "{name} menubar bg");
            assert_eq!(t.fkey_label.bg, Some(MC_TEAL), "{name} fkey bar bg");
            // In truecolor the bars/cursor are drawn via the gradient. It should
            // still shift (some gradient) but stay in the teal family (g ≈ b,
            // red kept low) so it reads as cyan, not blue→magenta.
            let (a, b) = (t.gradient_at(0, 20), t.gradient_at(19, 20));
            assert_ne!(a, b, "{name} gradient should still vary");
            for c in [a, b] {
                if let Color::Rgb(r, g, bl) = c {
                    assert!(r < g && r < bl, "{name} gradient stop {c:?} not teal");
                    assert!(g.abs_diff(bl) < 40, "{name} gradient stop {c:?} not cyan-ish");
                }
            }
        }
    }

    #[test]
    fn mc_theme_uses_classic_two_tone_chrome() {
        let t = Theme::by_name("Midnight Commander", true);
        let cyan = rgb(0x0dcdcd);
        let black = rgb(0x000000);
        // Dialogs: light "paper" background, black text, blue titles.
        assert_eq!(t.dialog_bg, rgb(0xc6c6c6));
        assert_eq!(t.dialog_fg, black);
        assert_eq!(t.dialog_title, rgb(0x0d73cc));
        // Teal selection bars / input fields inside dialogs.
        assert_eq!(t.dialog_selection.bg, Some(cyan));
        assert_eq!(t.button_focused.bg, Some(cyan));
        assert_eq!(t.input_bg, cyan);
        assert_eq!(t.input_fg, black);
        // Menus stay bright cyan with white text and a black selection bar.
        assert_eq!(t.menu_bg, cyan);
        assert_eq!(t.menu_fg, rgb(0xffffff));
        assert_eq!(t.menu_selection.bg, Some(black));
    }

    #[test]
    fn text_fg_is_more_contrasty_than_panel_fg() {
        // For both dark and light themes, the editor/viewer text color should be
        // further (in luma) from the background than the default panel foreground.
        for name in ["Dracula", "Nord", "Gruvbox Dark", "Gruvbox Light", "Solarized Light"] {
            let t = Theme::by_name(name, true);
            let d_text = (luma(t.text_fg) - luma(t.panel_bg)).abs();
            let d_panel = (luma(t.panel_fg) - luma(t.panel_bg)).abs();
            assert!(
                d_text >= d_panel,
                "{name}: text_fg ({d_text}) should contrast at least as much as panel_fg ({d_panel})"
            );
            assert_ne!(t.text_fg, t.panel_fg, "{name}: text_fg should differ from panel_fg");
        }
    }

    #[test]
    fn non_mc_themes_distinguish_menus_from_dialogs() {
        for name in ["Dracula", "Nord", "Gruvbox Dark", "Gruvbox Light", "Tokyo Night", "Ayu"] {
            let t = Theme::by_name(name, true);
            assert_ne!(t.menu_bg, t.dialog_bg, "{name} menu/dialog bg identical");
            assert_ne!(
                t.menu_selection.bg, t.dialog_selection.bg,
                "{name} menu/dialog selection identical"
            );
        }
    }

    #[test]
    fn new_themes_are_registered_and_build() {
        for name in ["Rainbow", "Candy", "Neon", "Forest", "Freedom", "Movienight"] {
            assert!(has_palette(name), "{name} palette missing");
            let t = Theme::by_name(name, true);
            assert_eq!(t.name, name);
            // Sanity: distinct bg/fg and a non-trivial gradient.
            assert_ne!(t.panel_bg, t.panel_fg, "{name} bg == fg");
            assert_ne!(t.gradient_at(0, 10), t.gradient_at(9, 10), "{name} flat gradient");
        }
    }

    #[test]
    fn rat_commander_is_default_and_commander_themes_are_registered() {
        // All three adopted themes are present and build, and the old no-space
        // "MidnightCommander Classic" is gone (replaced by Rat Commander).
        for name in ["Rat Commander", "Midnight Commander", "Midnight Commander Dark"] {
            assert!(has_palette(name), "{name} missing");
            assert_eq!(Theme::by_name(name, true).name, name);
        }
        assert!(!has_palette("MidnightCommander Classic"), "old classic theme should be gone");

        // Rat Commander is the first built-in, hence the default.
        assert_eq!(builtin_specs()[0].name, "Rat Commander");
        assert_eq!(Theme::mc().name, "Rat Commander");
        assert_eq!(crate::config::Config::default().theme, "Rat Commander");

        // Its signature colors: deep-blue panels, teal selection bar, light dialogs.
        let t = Theme::by_name("Rat Commander", true);
        assert_eq!(t.panel_bg, rgb(0x0000cd));
        assert_eq!(t.cursor.bg, Some(MC_TEAL));
        assert_eq!(t.cursor.fg, Some(rgb(0x000000)));
        assert_eq!(t.dialog_bg, rgb(0xc6c6c6));
        assert_eq!(t.archive_fg, rgb(0xff55ff));
    }
}
