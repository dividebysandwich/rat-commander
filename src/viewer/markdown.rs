//! A lightweight Markdown "render" approximation for the viewer. Each source
//! line is turned into *display* text with the markup removed — `##`, `**`, `*`,
//! `` ` ``, `[text](url)` etc. are stripped — and per-character styles that color
//! headings by level, emphasize bold/italic/code, accent lists and links.
//!
//! It is purely line-local (no multi-line fenced-code tracking) so it fits the
//! viewer's paged line model. The source line index is unchanged (scrolling /
//! goto / search still work on the raw bytes); only each line's *rendering* is
//! transformed.

use crate::ui::theme::Theme;
use ratatui::style::{Color, Modifier, Style};

/// Render one Markdown source line into `(display_chars, per-char styles)` with
/// the markup markers removed.
pub fn render_line(chars: &[char], theme: &Theme) -> (Vec<char>, Vec<Style>) {
    let base = Style::default().fg(theme.text_fg).bg(theme.panel_bg);
    let dim = base.fg(theme.panel_border);
    let mut out = Out { c: Vec::with_capacity(chars.len()), s: Vec::with_capacity(chars.len()) };
    if chars.is_empty() {
        return (out.c, out.s);
    }

    // Preserve leading indentation (matters for nested lists).
    let indent = chars.iter().take_while(|c| **c == ' ' || **c == '\t').count();
    for &c in &chars[..indent] {
        out.push(c, base);
    }
    let body = &chars[indent..];

    // ATX heading: 1–6 '#' then a space (or the whole line). Drop the markers,
    // color the text by level and bold it.
    let hashes = body.iter().take_while(|c| **c == '#').count();
    if (1..=6).contains(&hashes)
        && body.get(hashes).map(|c| *c == ' ').unwrap_or(body.len() == hashes)
    {
        let head = base.fg(heading_color(hashes, theme)).add_modifier(Modifier::BOLD);
        let mut start = indent + hashes;
        if chars.get(start) == Some(&' ') {
            start += 1;
        }
        emit_inline(chars, start, head, &mut out, theme);
        return (out.c, out.s);
    }

    // Horizontal rule: render a thin line instead of the markers.
    if is_hr(body) {
        for &c in body {
            if c == ' ' {
                out.push(' ', base);
            } else {
                out.push('─', dim);
            }
        }
        return (out.c, out.s);
    }

    // Fenced-code marker (``` / ~~~): drop the fence, show any info string dim.
    if body.starts_with(&['`', '`', '`']) || body.starts_with(&['~', '~', '~']) {
        let fence = body.iter().take_while(|c| **c == body[0]).count();
        for &c in &chars[indent + fence..] {
            out.push(c, dim);
        }
        return (out.c, out.s);
    }

    // Blockquote: a bar prefix, then the (dimmed, italic) text.
    if body.first() == Some(&'>') {
        out.push('▌', dim);
        out.push(' ', dim);
        let mut start = indent + 1;
        if chars.get(start) == Some(&' ') {
            start += 1;
        }
        let quote = dim.add_modifier(Modifier::ITALIC);
        emit_inline(chars, start, quote, &mut out, theme);
        return (out.c, out.s);
    }

    // List item: a bullet for unordered lists, the number kept for ordered ones.
    if let Some(n) = list_marker_len(body) {
        let accent = base.fg(theme.marked_fg).add_modifier(Modifier::BOLD);
        if matches!(body[0], '-' | '*' | '+') {
            out.push('•', accent);
            out.push(' ', base);
        } else {
            for &c in &body[..n - 1] {
                out.push(c, accent); // the "1." / "1)"
            }
            out.push(' ', base);
        }
        emit_inline(chars, indent + n, base, &mut out, theme);
        return (out.c, out.s);
    }

    emit_inline(chars, indent, base, &mut out, theme);
    (out.c, out.s)
}

/// Accumulates the rendered characters and their styles.
struct Out {
    c: Vec<char>,
    s: Vec<Style>,
}

impl Out {
    fn push(&mut self, c: char, s: Style) {
        self.c.push(c);
        self.s.push(s);
    }
}

/// Emit `chars[start..]` with inline markup applied (markers dropped):
/// `` `code` ``, `**bold**`, `*italic*`, `[text](url)`. `_`/`__` are left alone
/// so `snake_case` isn't treated as emphasis.
fn emit_inline(chars: &[char], start: usize, base: Style, out: &mut Out, theme: &Theme) {
    let code = base.fg(theme.doc_fg);
    let link = base.fg(theme.symlink_fg).add_modifier(Modifier::UNDERLINED);
    let n = chars.len();
    let mut i = start;
    while i < n {
        match chars[i] {
            '`' => {
                if let Some(j) = (i + 1..n).find(|&j| chars[j] == '`') {
                    for &c in &chars[i + 1..j] {
                        out.push(c, code);
                    }
                    i = j + 1;
                    continue;
                }
            }
            '*' => {
                let bold = chars.get(i + 1) == Some(&'*');
                let mlen = if bold { 2 } else { 1 };
                if let Some(j) = find_run(chars, i + mlen, '*', mlen) {
                    let m = if bold { Modifier::BOLD } else { Modifier::ITALIC };
                    let style = base.add_modifier(m);
                    for &c in &chars[i + mlen..j] {
                        out.push(c, style);
                    }
                    i = j + mlen;
                    continue;
                }
            }
            '[' => {
                if let Some(close) = (i + 1..n).find(|&j| chars[j] == ']')
                    && chars.get(close + 1) == Some(&'(')
                    && let Some(end) = (close + 2..n).find(|&j| chars[j] == ')')
                {
                    for &c in &chars[i + 1..close] {
                        out.push(c, link);
                    }
                    i = end + 1; // skip the `](url)` part entirely
                    continue;
                }
            }
            _ => {}
        }
        out.push(chars[i], base);
        i += 1;
    }
}

/// The first index `j ≥ from` where `chars[j..j+len]` is all `marker`.
fn find_run(chars: &[char], from: usize, marker: char, len: usize) -> Option<usize> {
    (from..=chars.len().saturating_sub(len)).find(|&j| chars[j..j + len].iter().all(|c| *c == marker))
}

pub(crate) fn heading_color(level: usize, theme: &Theme) -> Color {
    match level {
        1 => theme.header_fg,
        2 => theme.symlink_fg,
        3 => theme.exec_fg,
        4 => theme.archive_fg,
        5 => theme.dir_fg,
        _ => theme.marked_fg,
    }
}

/// Length of a leading list marker (`- `, `* `, `+ `, `1. `, `1) `), if any.
fn list_marker_len(body: &[char]) -> Option<usize> {
    if matches!(body.first(), Some('-') | Some('*') | Some('+')) && body.get(1) == Some(&' ') {
        return Some(2);
    }
    let digits = body.iter().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0
        && matches!(body.get(digits), Some('.') | Some(')'))
        && body.get(digits + 1) == Some(&' ')
    {
        return Some(digits + 2);
    }
    None
}

/// A horizontal rule: ≥3 of only `-`, `*`, or `_` (ignoring spaces).
fn is_hr(body: &[char]) -> bool {
    let marks: Vec<char> = body.iter().copied().filter(|c| *c != ' ').collect();
    marks.len() >= 3
        && (marks.iter().all(|c| *c == '-')
            || marks.iter().all(|c| *c == '*')
            || marks.iter().all(|c| *c == '_'))
}

// ---------------------------------------------------------------------------
// Document outline (the viewer's F6 heading navigator)
// ---------------------------------------------------------------------------

/// One entry in the document outline: a heading's nesting level (1–6), its
/// display text (markup stripped) and the source line it starts on.
#[derive(Debug, Clone)]
pub struct OutlineItem {
    pub level: usize,
    pub text: String,
    pub line: usize,
}

/// If `line` is an ATX heading (1–6 `#` then a space), return its level and the
/// heading text with the markers and inline markup removed. Mirrors the heading
/// rule in [`render_line`]. Headings with no text (a bare `##`) return `None`,
/// since they make no useful navigation target.
pub fn heading_of(line: &str) -> Option<(usize, String)> {
    let chars: Vec<char> = line.chars().collect();
    let indent = chars.iter().take_while(|c| **c == ' ' || **c == '\t').count();
    let body = &chars[indent..];
    let hashes = body.iter().take_while(|c| **c == '#').count();
    if !(1..=6).contains(&hashes)
        || !body.get(hashes).map(|c| *c == ' ').unwrap_or(body.len() == hashes)
    {
        return None;
    }
    let mut start = hashes;
    if body.get(start) == Some(&' ') {
        start += 1;
    }
    let text = strip_inline(&body[start..]);
    let text = text.trim().to_string();
    if text.is_empty() {
        return None;
    }
    Some((hashes, text))
}

/// Whether `line` opens or closes a fenced code block (` ``` ` or `~~~`). The
/// outline scanner toggles on each such line so `#` lines inside code fences are
/// not mistaken for headings.
pub fn is_fence(line: &str) -> bool {
    let chars: Vec<char> = line.chars().collect();
    let indent = chars.iter().take_while(|c| **c == ' ' || **c == '\t').count();
    let body = &chars[indent..];
    body.starts_with(&['`', '`', '`']) || body.starts_with(&['~', '~', '~'])
}

/// If `line` is a code fence, the trimmed info string (the language after the
/// ` ``` `/`~~~` markers) — empty when there is none. `None` if `line` is not a
/// fence. Used to label the opening border of the rendered code box.
pub fn fence_info(line: &str) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();
    let indent = chars.iter().take_while(|c| **c == ' ' || **c == '\t').count();
    let body = &chars[indent..];
    let marker = *body.first()?;
    if !(body.starts_with(&['`', '`', '`']) || body.starts_with(&['~', '~', '~'])) {
        return None;
    }
    let fence = body.iter().take_while(|c| **c == marker).count();
    Some(body[fence..].iter().collect::<String>().trim().to_string())
}

/// Strip inline markup (`` `code` ``, `**bold**`/`*italic*`, `[text](url)`) from
/// `chars`, returning the plain display text — the same transform
/// [`emit_inline`] applies, but without styling (used for outline labels).
fn strip_inline(chars: &[char]) -> String {
    let n = chars.len();
    let mut out = String::with_capacity(n);
    let mut i = 0;
    while i < n {
        match chars[i] {
            '`' => {
                if let Some(j) = (i + 1..n).find(|&j| chars[j] == '`') {
                    out.extend(&chars[i + 1..j]);
                    i = j + 1;
                    continue;
                }
            }
            '*' => {
                let bold = chars.get(i + 1) == Some(&'*');
                let mlen = if bold { 2 } else { 1 };
                if let Some(j) = find_run(chars, i + mlen, '*', mlen) {
                    out.extend(&chars[i + mlen..j]);
                    i = j + mlen;
                    continue;
                }
            }
            '[' => {
                if let Some(close) = (i + 1..n).find(|&j| chars[j] == ']')
                    && chars.get(close + 1) == Some(&'(')
                    && let Some(end) = (close + 2..n).find(|&j| chars[j] == ')')
                {
                    out.extend(&chars[i + 1..close]);
                    i = end + 1; // skip the `](url)` part entirely
                    continue;
                }
            }
            _ => {}
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(s: &str) -> (String, Vec<Style>) {
        let (c, st) = render_line(&s.chars().collect::<Vec<_>>(), &Theme::mc());
        (c.into_iter().collect(), st)
    }

    #[test]
    fn heading_strips_markers_and_colors_text() {
        let th = Theme::mc();
        let (text, st) = render("## Title");
        assert_eq!(text, "Title", "the '##' marker is removed");
        assert_eq!(st[0].fg, Some(heading_color(2, &th)));
        assert!(st[0].add_modifier.contains(Modifier::BOLD));
        // Different levels use different colors.
        assert_ne!(render("# A").1[0].fg, render("### A").1[0].fg);
        // Not a heading without the space.
        assert_eq!(render("##x").0, "##x");
    }

    #[test]
    fn inline_markers_are_removed_and_styled() {
        let th = Theme::mc();
        let (text, st) = render("a **b** *c* `d` e");
        assert_eq!(text, "a b c d e", "**, * and ` markers are gone");
        let at = |ch: char| text.find(ch).unwrap();
        assert!(st[at('b')].add_modifier.contains(Modifier::BOLD));
        assert!(st[at('c')].add_modifier.contains(Modifier::ITALIC));
        assert_eq!(st[at('d')].fg, Some(th.doc_fg));
    }

    #[test]
    fn link_shows_only_its_text() {
        let th = Theme::mc();
        let (text, st) = render("see [the docs](http://x) now");
        assert_eq!(text, "see the docs now");
        assert_eq!(st[text.find('t').unwrap()].fg, Some(th.symlink_fg));
    }

    #[test]
    fn lists_quotes_rules_and_snake_case() {
        assert_eq!(render("- item").0, "• item");
        assert_eq!(render("* item").0, "• item");
        assert_eq!(render("1. item").0, "1. item"); // ordered number kept
        assert_eq!(render("> quoted").0, "▌ quoted");
        assert_eq!(render("---").0, "───");
        // Underscores are not emphasis, so identifiers pass through untouched.
        assert_eq!(render("foo_bar_baz").0, "foo_bar_baz");
    }

    #[test]
    fn heading_of_detects_levels_and_strips_markup() {
        assert_eq!(heading_of("# Title"), Some((1, "Title".to_string())));
        assert_eq!(heading_of("###   Spaced  "), Some((3, "Spaced".to_string())));
        // Inline markup and a link are reduced to plain text.
        assert_eq!(
            heading_of("## The `code` and [a link](http://x)"),
            Some((2, "The code and a link".to_string()))
        );
        // Not a heading without a following space, beyond level 6, or with no text.
        assert_eq!(heading_of("##x"), None);
        assert_eq!(heading_of("####### too deep"), None);
        assert_eq!(heading_of("##"), None);
        assert_eq!(heading_of("plain text"), None);
    }

    #[test]
    fn is_fence_matches_backticks_and_tildes() {
        assert!(is_fence("```"));
        assert!(is_fence("```rust"));
        assert!(is_fence("  ~~~"));
        assert!(!is_fence("`inline`"));
        assert!(!is_fence("text"));
    }

    #[test]
    fn fence_info_extracts_the_language() {
        assert_eq!(fence_info("```rust"), Some("rust".to_string()));
        assert_eq!(fence_info("~~~  python  "), Some("python".to_string()));
        assert_eq!(fence_info("```"), Some(String::new())); // no language
        assert_eq!(fence_info("  ```toml"), Some("toml".to_string())); // indented
        assert_eq!(fence_info("text"), None);
        assert_eq!(fence_info("`inline`"), None);
    }
}
