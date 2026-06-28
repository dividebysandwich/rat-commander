//! Syntax highlighting via [`syntect`] (the engine behind `bat`).
//!
//! A single [`Highlighter`] highlights one document incrementally: it caches the
//! parser/highlighter state at the start of every processed line plus that
//! line's color runs, so the viewer/editor only ever highlight lines up to what
//! they display, and an edit invalidates just the affected suffix.
//!
//! The bundled Sublime syntaxes and themes are used as-is; the design is
//! extensible — extra `.sublime-syntax` and `.tmTheme` files could be loaded
//! into the static sets later without touching callers.

use ratatui::style::Color;
use std::sync::LazyLock;
use syntect::highlighting::{
    HighlightIterator, HighlightState, Highlighter as SynHighlighter, Theme, ThemeSet,
};
use syntect::parsing::{ParseState, ScopeStack, SyntaxSet};

/// Documents larger than this are rendered without highlighting (keeps the
/// incremental caches and per-line CPU cost bounded).
pub const HL_MAX_BYTES: usize = 2 * 1024 * 1024;

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEMES: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

/// A run of `count` characters sharing one foreground color.
pub type ColorRun = (u32, Color);

/// Incremental highlighter for a single document.
pub struct Highlighter {
    hl: SynHighlighter<'static>,
    /// State at the *start* of each line (`states[0]` is the initial state);
    /// `states[i+1]` is produced after processing line `i`.
    states: Vec<(ParseState, HighlightState)>,
    /// Color runs for each processed line (aligned to the display string fed in).
    colors: Vec<Vec<ColorRun>>,
}

impl Highlighter {
    /// Build a highlighter for `file_name` (matched by extension), choosing a
    /// bundled theme to suit a dark or light UI. Returns `None` when no syntax
    /// matches — callers then render plain text.
    pub fn for_file(file_name: &str, dark: bool) -> Option<Highlighter> {
        let ext = file_name.rsplit('.').next().unwrap_or("");
        let syntax = SYNTAXES.find_syntax_by_extension(ext)?;
        let theme_name = if dark { "base16-ocean.dark" } else { "InspiredGitHub" };
        let theme: &'static Theme = THEMES.themes.get(theme_name)?;
        let hl = SynHighlighter::new(theme);
        let initial = (
            ParseState::new(syntax),
            HighlightState::new(&hl, ScopeStack::new()),
        );
        Some(Highlighter {
            hl,
            states: vec![initial],
            colors: Vec::new(),
        })
    }

    /// Number of lines highlighted so far.
    pub fn processed(&self) -> usize {
        self.colors.len()
    }

    /// Highlight the next line (its `display` text, exactly as it will be drawn,
    /// without a trailing newline). Must be called in order from `processed()`.
    pub fn process_next(&mut self, display: &str) {
        let i = self.colors.len();
        let (mut parse, mut hstate) = self.states[i].clone();
        // syntect wants the newline for correct multi-line context tracking.
        let line = format!("{display}\n");
        let ops = parse.parse_line(&line, &SYNTAXES).unwrap_or_default();
        let mut runs: Vec<ColorRun> = Vec::new();
        for (style, text) in HighlightIterator::new(&mut hstate, &ops, &line, &self.hl) {
            let c = style.foreground;
            let color = Color::Rgb(c.r, c.g, c.b);
            let n = text.chars().count() as u32;
            if n == 0 {
                continue;
            }
            // Merge adjacent runs of the same color.
            match runs.last_mut() {
                Some(last) if last.1 == color => last.0 += n,
                _ => runs.push((n, color)),
            }
        }
        // Drop the trailing '\n' we appended (one char) so runs align to `display`.
        trim_one(&mut runs);
        self.colors.push(runs);
        self.states.push((parse, hstate));
    }

    /// Color runs for an already-processed line (empty if not processed).
    pub fn line(&self, li: usize) -> &[ColorRun] {
        self.colors.get(li).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Drop cached results from `from_line` onward (after an edit there). The
    /// state at the start of `from_line` is unaffected, so it is kept.
    pub fn invalidate(&mut self, from_line: usize) {
        self.colors.truncate(from_line);
        self.states.truncate(from_line + 1);
    }

    /// Expand this line's runs into a per-character foreground color list of
    /// length `len` (padding with `default` past the highlighted run length).
    pub fn line_fg(&self, li: usize, len: usize, default: Color) -> Vec<Color> {
        let mut out = Vec::with_capacity(len);
        for &(n, color) in self.line(li) {
            for _ in 0..n {
                if out.len() >= len {
                    return out;
                }
                out.push(color);
            }
        }
        out.resize(len, default);
        out
    }
}

/// Remove one trailing character's worth from the last run (the appended '\n').
fn trim_one(runs: &mut Vec<ColorRun>) {
    if let Some(last) = runs.last_mut() {
        if last.0 <= 1 {
            runs.pop();
        } else {
            last.0 -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_rust_keyword_distinctly() {
        let mut h = Highlighter::for_file("a.rs", true).expect("rust syntax exists");
        // `fn` should be colored differently from the identifier after it.
        h.process_next("fn main() {}");
        let fg = h.line_fg(0, "fn main() {}".chars().count(), Color::Rgb(0, 0, 0));
        assert_eq!(fg.len(), "fn main() {}".chars().count());
        let kw = fg[0]; // 'f' of `fn`
        let ident = fg[3]; // 'm' of `main`
        assert_ne!(kw, ident, "keyword and identifier should differ in color");
    }

    #[test]
    fn unknown_extension_has_no_highlighter() {
        assert!(Highlighter::for_file("notes.unknownext", true).is_none());
    }

    #[test]
    fn invalidate_drops_suffix() {
        let mut h = Highlighter::for_file("a.rs", true).unwrap();
        h.process_next("let a = 1;");
        h.process_next("let b = 2;");
        assert_eq!(h.processed(), 2);
        h.invalidate(1);
        assert_eq!(h.processed(), 1, "lines from the edit point are dropped");
    }
}
