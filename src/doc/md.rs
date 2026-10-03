//! The text the document view shows, written as the viewer's own Markdown:
//! `#` headings (so they colour and fill the F6 outline), `-` and `1.` list
//! items, and tables already laid out in box-drawing characters, since the
//! Markdown renderer has no tables of its own. The view turns inline markup off
//! (see `ViewerState::markdown_inline`), so a document's own `*` and backticks
//! show as written; only what starts a line can be taken for markup, and
//! [`sanitize`] defuses that.

use unicode_width::UnicodeWidthStr;

/// Widest a table column is drawn before its cells wrap.
const COL_MAX: usize = 40;
/// A table wider than this even with capped columns is written one row per
/// line instead, with cells separated by a bar.
const TABLE_MAX: usize = 200;

/// Accumulates a document's text, keeping one blank line between blocks and
/// none between the items of a list.
#[derive(Default)]
pub struct Md {
    out: String,
    /// Whether the last block was a list item, which the next item joins.
    in_list: bool,
    /// Stop growing past this many bytes (the document is then marked cut).
    limit: usize,
    pub truncated: bool,
}

impl Md {
    pub fn new(limit: usize) -> Self {
        Md { limit, ..Default::default() }
    }

    pub fn full(&self) -> bool {
        self.truncated
    }

    pub fn finish(mut self) -> String {
        if self.truncated {
            self.block();
            self.out.push_str(&format!("— {} —\n", crate::l10n::tr("(truncated)")));
        }
        self.out
    }

    fn room(&mut self) -> bool {
        if self.out.len() >= self.limit {
            self.truncated = true;
        }
        !self.truncated
    }

    /// Start a new block: a blank line after whatever came before.
    fn block(&mut self) {
        if !self.out.is_empty() && !self.out.ends_with("\n\n") {
            if !self.out.ends_with('\n') {
                self.out.push('\n');
            }
            self.out.push('\n');
        }
        self.in_list = false;
    }

    pub fn heading(&mut self, level: usize, text: &str) {
        let text = one_line(text);
        if text.is_empty() || !self.room() {
            return;
        }
        self.block();
        self.out.push_str(&"#".repeat(level.clamp(1, 6)));
        self.out.push(' ');
        self.out.push_str(&text);
        self.out.push('\n');
    }

    /// A paragraph; `\n` inside it is a line break.
    pub fn para(&mut self, text: &str) {
        let lines: Vec<String> = text.lines().map(|l| sanitize(l.trim())).collect();
        if lines.iter().all(|l| l.is_empty()) || !self.room() {
            return;
        }
        self.block();
        for l in trim_blank(&lines) {
            self.out.push_str(l);
            self.out.push('\n');
        }
    }

    /// A list item at nesting `depth`, marked `-` or with its number (`"3."`).
    pub fn item(&mut self, depth: usize, marker: &str, text: &str) {
        let lines: Vec<String> = text.lines().map(|l| sanitize(l.trim())).collect();
        if !self.room() {
            return;
        }
        if !self.in_list {
            self.block();
        }
        let indent = "  ".repeat(depth.min(8));
        let lines = trim_blank(&lines);
        let first = lines.first().map_or("", |s| s.as_str());
        self.out.push_str(&format!("{indent}{marker} {first}\n"));
        // Later lines line up under the text (the renderer keeps indentation).
        let pad = " ".repeat(indent.len() + marker.chars().count() + 1);
        for l in lines.iter().skip(1) {
            self.out.push_str(&pad);
            self.out.push_str(l);
            self.out.push('\n');
        }
        self.in_list = true;
    }

    /// A quoted block: each line behind the renderer's quote bar.
    pub fn quote(&mut self, text: &str) {
        let lines: Vec<String> = text.lines().map(|l| l.trim().to_string()).collect();
        if lines.iter().all(|l| l.is_empty()) || !self.room() {
            return;
        }
        self.block();
        for l in trim_blank(&lines) {
            self.out.push_str("> ");
            self.out.push_str(l);
            self.out.push('\n');
        }
    }

    /// A horizontal rule (between chapters).
    pub fn rule(&mut self) {
        if !self.room() {
            return;
        }
        self.block();
        self.out.push_str("---\n");
    }

    /// Text shown as-is in a code box (a `<pre>` block).
    pub fn code(&mut self, text: &str) {
        if !self.room() {
            return;
        }
        self.block();
        self.out.push_str("```\n");
        for l in text.trim_matches('\n').lines() {
            // A fence inside would close the box early.
            let l = if l.trim_start().starts_with("```") { format!("\u{a0}{l}") } else { l.into() };
            self.out.push_str(&l);
            self.out.push('\n');
        }
        self.out.push_str("```\n");
    }

    /// A table of cells (each may hold line breaks), the first row its header.
    pub fn table(&mut self, rows: &[Vec<String>]) {
        let rows: Vec<&Vec<String>> =
            rows.iter().filter(|r| r.iter().any(|c| !c.trim().is_empty())).collect();
        if rows.is_empty() || !self.room() {
            return;
        }
        self.block();
        let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        // Each cell's text as wrapped lines, once the column widths are known.
        let mut widths = vec![1usize; cols];
        for r in &rows {
            for (i, c) in r.iter().enumerate() {
                for l in c.lines() {
                    widths[i] = widths[i].max(l.trim().width().min(COL_MAX));
                }
            }
        }
        let total: usize = widths.iter().sum::<usize>() + 3 * cols + 1;
        if total > TABLE_MAX {
            for r in &rows {
                let cells: Vec<String> = r.iter().map(|c| one_line(c)).collect();
                self.out.push_str(&sanitize(&cells.join(" │ ")));
                self.out.push('\n');
            }
            return;
        }
        let border = |l: char, m: char, r: char| {
            let mut s = String::from(l);
            for (i, w) in widths.iter().enumerate() {
                s.push_str(&"─".repeat(w + 2));
                s.push(if i + 1 == cols { r } else { m });
            }
            s.push('\n');
            s
        };
        self.out.push_str(&border('┌', '┬', '┐'));
        for (ri, r) in rows.iter().enumerate() {
            let wrapped: Vec<Vec<String>> = (0..cols)
                .map(|i| {
                    let cell = r.get(i).map_or("", |s| s.as_str());
                    cell.lines().flat_map(|l| wrap(l.trim(), widths[i])).collect()
                })
                .collect();
            let height = wrapped.iter().map(|c| c.len()).max().unwrap_or(1).max(1);
            for line in 0..height {
                let mut s = String::from("│");
                for (i, cell) in wrapped.iter().enumerate() {
                    let text = cell.get(line).map_or("", |s| s.as_str());
                    s.push(' ');
                    s.push_str(text);
                    s.push_str(&" ".repeat(widths[i].saturating_sub(text.width()) + 1));
                    s.push('│');
                }
                s.push('\n');
                self.out.push_str(&s);
            }
            if ri == 0 && rows.len() > 1 {
                self.out.push_str(&border('├', '┼', '┤'));
            }
        }
        self.out.push_str(&border('└', '┴', '┘'));
    }
}

/// Lines without the blank ones at either end.
fn trim_blank(lines: &[String]) -> &[String] {
    let start = lines.iter().position(|l| !l.is_empty()).unwrap_or(lines.len());
    let end = lines.iter().rposition(|l| !l.is_empty()).map_or(start, |e| e + 1);
    &lines[start..end]
}

/// `text` on one line, its whitespace runs collapsed.
pub fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Keep a line of document text from reading as Markdown block syntax — a
/// heading, quote, rule or code fence it never was — by starting it with a
/// no-break space, which the renderer does not count as indentation.
pub fn sanitize(line: &str) -> String {
    let t = line.trim_start();
    let first = t.chars().next();
    let hr = t.chars().filter(|c| !c.is_whitespace()).count() >= 3
        && t.chars().all(|c| c.is_whitespace() || c == '-')
        || t.chars().all(|c| c.is_whitespace() || c == '*') && t.contains("***")
        || t.chars().all(|c| c.is_whitespace() || c == '_') && t.contains("___");
    if matches!(first, Some('#' | '>')) || t.starts_with("```") || t.starts_with("~~~") || hr {
        format!("\u{a0}{line}")
    } else {
        line.to_string()
    }
}

/// Word-wrap `text` to `width` display columns, breaking inside a word only
/// when the word alone is wider.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let sep = usize::from(!cur.is_empty());
        if cur.width() + sep + word.width() <= width {
            if sep == 1 {
                cur.push(' ');
            }
            cur.push_str(word);
            continue;
        }
        if !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
        }
        for c in word.chars() {
            if cur.width() + c.to_string().width() > width && !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
            }
            cur.push(c);
        }
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_are_separated_and_list_items_are_not() {
        let mut md = Md::new(usize::MAX);
        md.heading(2, "Title");
        md.para("first\nsecond");
        md.item(0, "-", "a");
        md.item(1, "1.", "b\nmore");
        md.para("after");
        md.quote("# quoted\nline");
        assert_eq!(
            md.finish(),
            "## Title\n\nfirst\nsecond\n\n- a\n  1. b\n     more\n\nafter\n\n> # quoted\n> line\n"
        );
    }

    #[test]
    fn markdown_lookalikes_are_defused() {
        assert_eq!(sanitize("# not a heading"), "\u{a0}# not a heading");
        assert_eq!(sanitize("---"), "\u{a0}---");
        assert_eq!(sanitize("```"), "\u{a0}```");
        assert_eq!(sanitize("plain - text"), "plain - text");
    }

    #[test]
    fn tables_are_boxed_with_aligned_columns() {
        let mut md = Md::new(usize::MAX);
        md.table(&[vec!["Name".into(), "Qty".into()], vec!["Apple".into(), "3".into()]]);
        let out = md.finish();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "┌───────┬─────┐");
        assert_eq!(lines[1], "│ Name  │ Qty │");
        assert_eq!(lines[2], "├───────┼─────┤");
        assert_eq!(lines[3], "│ Apple │ 3   │");
        assert_eq!(lines[4], "└───────┴─────┘");
    }

    #[test]
    fn long_cells_wrap_inside_their_column() {
        let long = "word ".repeat(20);
        let mut md = Md::new(usize::MAX);
        md.table(&[vec![long.clone(), "x".into()]]);
        let out = md.finish();
        assert!(out.lines().count() > 3, "{out}");
        assert!(out.lines().all(|l| l.width() <= COL_MAX + 10), "{out}");
    }

    #[test]
    fn output_stops_at_the_limit() {
        let mut md = Md::new(10);
        md.para("0123456789abc");
        md.para("never");
        assert!(md.full());
        let out = md.finish();
        assert!(!out.contains("never") && out.contains("(truncated)"), "{out}");
    }
}
