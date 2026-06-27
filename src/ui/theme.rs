//! Color themes.
//!
//! A [`Palette`] is a classic 16-ANSI-color terminal scheme (plus bg/fg). The
//! [`Theme`] is built from a palette via [`Theme::from_palette`], mapping the
//! palette onto every UI element. A curated set of well-known schemes from
//! terminalcolors.com is provided in [`PALETTES`]; more can be added by
//! appending palette literals.

use ratatui::style::{Color, Modifier, Style};

const fn rgb(h: u32) -> Color {
    Color::Rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
}

/// A 16-color terminal palette plus background/foreground.
#[derive(Clone, Copy)]
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

/// Centralized styles for every UI element, derived from a palette.
#[derive(Clone)]
pub struct Theme {
    pub name: String,
    pub truecolor: bool,
    pub panel_bg: Color,
    pub panel_fg: Color,
    pub panel_border: Color,
    pub panel_border_active: Color,
    pub header_fg: Color,
    pub cursor: Style,
    pub cursor_inactive: Style,
    pub cursor_fg: Color,
    pub marked_fg: Color,
    pub dir_fg: Color,
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
        Theme::from_palette(&PALETTES[0], true)
    }

    /// Build a theme from a palette. `truecolor` enables RGB gradients.
    pub fn from_palette(p: &Palette, truecolor: bool) -> Self {
        let surface = if truecolor {
            mix(p.bg, p.fg, 0.12)
        } else {
            p.bright_black
        };
        // Classic Midnight Commander uses a cyan selection bar with black text;
        // other themes use the (gradient-friendly) bright-blue cursor.
        let (cursor_bg, cursor_fg) = if p.name == "MidnightCommander Classic" {
            (p.cyan, p.black)
        } else {
            (p.bright_blue, best_contrast(p.bright_blue, p.bg, p.bright_white))
        };
        // Borders/column separators must contrast with the panel background on
        // every theme (e.g. MC's blue border would vanish on its blue bg), so
        // derive them from a bg↔fg mix rather than a palette hue.
        let border = mix(p.bg, p.fg, 0.45);

        Theme {
            name: p.name.to_string(),
            truecolor,
            panel_bg: p.bg,
            panel_fg: p.fg,
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
            exec_fg: p.bright_green,
            symlink_fg: p.bright_cyan,
            // Archives = purple, documents = (dark) yellow, images = cyan,
            // audio/video = green — matching Midnight Commander's scheme.
            archive_fg: p.bright_magenta,
            doc_fg: p.yellow,
            image_fg: p.bright_cyan,
            media_fg: p.bright_green,
            menubar: Style::default().bg(p.cyan).fg(p.bg),
            fkey_label: Style::default().bg(p.cyan).fg(p.bg),
            fkey_num: Style::default().bg(p.bg).fg(p.bright_white),
            dialog_bg: surface,
            dialog_fg: p.fg,
            dialog_title: p.bright_cyan,
            input_bg: p.blue,
            input_fg: best_contrast(p.blue, p.bg, p.bright_white),
            button: Style::default().bg(surface).fg(p.fg),
            button_focused: Style::default()
                .bg(p.bright_cyan)
                .fg(p.bg)
                .add_modifier(Modifier::BOLD),
            error_fg: p.bright_red,
            // Vivid gradient endpoints keep the bars/cursor bright and modern.
            bar_fg: best_contrast(mix(p.bright_blue, p.bright_magenta, 0.5), p.black, p.bright_white),
            anim: 0,
            animated: false,
            grad_a: to_rgb(p.bright_blue),
            grad_b: to_rgb(p.bright_magenta),
        }
    }

    /// Look up a theme by palette name (case-insensitive), falling back to mc.
    pub fn by_name(name: &str, truecolor: bool) -> Self {
        find_palette(name)
            .map(|p| Theme::from_palette(p, truecolor))
            .unwrap_or_else(|| Theme::from_palette(&PALETTES[0], truecolor))
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
    let (r, g, b) = to_rgb(bg);
    // Rec. 601 luma.
    let luma = 0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64;
    if luma > 140.0 { dark } else { light }
}

/// Find a palette by name (case-insensitive, ignoring spaces).
pub fn find_palette(name: &str) -> Option<&'static Palette> {
    let key = name.to_ascii_lowercase().replace([' ', '-', '_'], "");
    PALETTES
        .iter()
        .find(|p| p.name.to_ascii_lowercase().replace([' ', '-', '_'], "") == key)
}

/// All theme names, in menu order.
pub fn palette_names() -> Vec<String> {
    PALETTES.iter().map(|p| p.name.to_string()).collect()
}

/// Curated terminal color schemes (a subset of terminalcolors.com). Each is a
/// standard 16-ANSI palette; the list is data-driven so more can be appended.
pub static PALETTES: &[Palette] = &[
    Palette {
        name: "Midnight Commander",
        bg: rgb(0x0000cd), fg: rgb(0xc6c6c6),
        black: rgb(0x000000), red: rgb(0xaa0000), green: rgb(0x00aa00), yellow: rgb(0xaa5500),
        blue: rgb(0x0000aa), magenta: rgb(0xaa00aa), cyan: rgb(0x00aaaa), white: rgb(0xc6c6c6),
        bright_black: rgb(0x555555), bright_red: rgb(0xff5555), bright_green: rgb(0x55ff55),
        bright_yellow: rgb(0xffff55), bright_blue: rgb(0x5555ff), bright_magenta: rgb(0xff55ff),
        bright_cyan: rgb(0x55ffff), bright_white: rgb(0xffffff),
    },
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
    // The classic Midnight Commander look, but with brighter, more saturated
    // accents. `yellow` is intentionally light so documents render like normal
    // files (classic MC doesn't tint them); headers/marks use `bright_yellow`.
    Palette {
        name: "MidnightCommander Classic",
        bg: rgb(0x1818d4), fg: rgb(0xe8e8e8),
        black: rgb(0x000000), red: rgb(0xcc0000), green: rgb(0x00cc00), yellow: rgb(0xe8e8e8),
        blue: rgb(0x0000cc), magenta: rgb(0xcc44cc), cyan: rgb(0x00c8c8), white: rgb(0xffffff),
        bright_black: rgb(0x808080), bright_red: rgb(0xff6464), bright_green: rgb(0x4cff4c),
        bright_yellow: rgb(0xffff44), bright_blue: rgb(0x6c6cff), bright_magenta: rgb(0xff55ff),
        bright_cyan: rgb(0x4cffff), bright_white: rgb(0xffffff),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_lookup_is_fuzzy() {
        assert!(find_palette("Dracula").is_some());
        assert!(find_palette("tokyo night").is_some());
        assert!(find_palette("rose-pine").is_some());
        assert!(find_palette("nonsense").is_none());
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
    fn classic_theme_uses_bright_classic_colors() {
        assert!(find_palette("MidnightCommander Classic").is_some());
        let t = Theme::by_name("MidnightCommander Classic", true);
        assert_eq!(t.name, "MidnightCommander Classic");
        // Cyan selection bar with black text (classic MC).
        assert_eq!(t.cursor.bg, Some(rgb(0x00c8c8)));
        assert_eq!(t.cursor.fg, Some(rgb(0x000000)));
        // Bright, saturated accents.
        assert_eq!(t.exec_fg, rgb(0x4cff4c));
        assert_eq!(t.archive_fg, rgb(0xff55ff));
        assert_eq!(t.header_fg, rgb(0xffff44));
        // Documents render like normal files (not tinted) in the classic look.
        assert_eq!(t.doc_fg, t.panel_fg);
    }
}

