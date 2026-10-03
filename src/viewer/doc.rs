//! The document view: what a Word, PowerPoint, OpenDocument, EPUB or PDF file
//! reads as, or a spreadsheet's sheets as tables — shown in place of the
//! file's bytes, which F8 (and F4, for hex) still reach.
//!
//! The content is held as viewers of its own: one in-memory Markdown viewer
//! for a text document, one table viewer per sheet. The outer viewer hands
//! them its keys, mouse, search and goto while the document shows, so a
//! document scrolls, searches, wraps and has an F6 outline exactly as a
//! Markdown or CSV file does — and each sheet keeps its own place.

use super::{GotoMode, ViewMode, ViewerSignal, ViewerState};
use crate::doc::{DocKind, Extracted};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub struct DocView {
    parts: Vec<ViewerState>,
    cur: usize,
    sheets: bool,
}

impl DocView {
    /// The view of `file_name`, a document of `kind` read as `x`.
    pub fn new(file_name: &str, kind: DocKind, x: Extracted) -> Self {
        match x {
            Extracted::Text(md) => {
                let mut v =
                    ViewerState::new(format!("{file_name} [{}]", kind.label()), md.into_bytes());
                v.is_markdown = true;
                v.markdown_render = true;
                v.prose = true;
                v.wrap = true;
                DocView { parts: vec![v], cur: 0, sheets: false }
            }
            Extracted::Sheets(sheets) => {
                let n = sheets.len();
                let parts = sheets
                    .into_iter()
                    .enumerate()
                    .map(|(i, s)| {
                        // Named `.tsv` while the table opens, so it is read
                        // as tab-separated; then named for the header.
                        let mut v = ViewerState::new(format!("{i}.tsv"), s.tsv);
                        v.ensure_table();
                        v.name = format!("{file_name} › {} ({}/{n})", s.name, i + 1);
                        v
                    })
                    .collect();
                DocView { parts, cur: 0, sheets: true }
            }
        }
    }

    pub(crate) fn current(&self) -> &ViewerState {
        &self.parts[self.cur]
    }

    pub(crate) fn current_mut(&mut self) -> &mut ViewerState {
        &mut self.parts[self.cur]
    }

    /// Whether there are sheets to switch between.
    fn multi(&self) -> bool {
        self.sheets && self.parts.len() > 1
    }

    fn cycle(&mut self, delta: isize) {
        let n = self.parts.len() as isize;
        self.cur = (self.cur as isize + delta).rem_euclid(n.max(1)) as usize;
    }
}

impl ViewerState {
    /// Attach a document and switch to showing it (F3 on a document).
    pub fn set_doc(&mut self, mut d: DocView) {
        for p in &mut d.parts {
            p.search_seed = self.search_seed.clone();
        }
        self.doc = Some(Box::new(d));
        self.show_doc = true;
        self.mode = ViewMode::Text;
    }

    /// The document, when it is showing (rather than the raw bytes).
    #[allow(dead_code)] // accessor used by tests
    pub(crate) fn active_doc(&self) -> Option<&DocView> {
        self.doc.as_deref().filter(|_| self.show_doc)
    }

    /// The viewer of the part on screen, when the document is showing.
    pub(crate) fn doc_part(&self) -> Option<&ViewerState> {
        self.doc.as_deref().filter(|_| self.show_doc).map(DocView::current)
    }

    pub(crate) fn doc_part_mut(&mut self) -> Option<&mut ViewerState> {
        if !self.show_doc {
            return None;
        }
        self.doc.as_deref_mut().map(DocView::current_mut)
    }

    /// Keys while the document shows: F8 and F4 leave it for the raw bytes,
    /// F6 / `]` / Ctrl-PgDn and Shift-F6 / `[` / Ctrl-PgUp switch sheets, and
    /// everything else is the part's own.
    pub(super) fn handle_doc_key(&mut self, key: KeyEvent) -> ViewerSignal {
        let Some(d) = self.doc.as_deref_mut() else { return ViewerSignal::Stay };
        if !d.current().outline_open {
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            let shift = key.modifiers.contains(KeyModifiers::SHIFT);
            match key.code {
                KeyCode::F(8) => {
                    self.show_doc = false;
                    return ViewerSignal::Stay;
                }
                KeyCode::F(4) => {
                    self.show_doc = false;
                    self.mode = ViewMode::Hex;
                    self.top = 0;
                    return ViewerSignal::Stay;
                }
                KeyCode::F(6) | KeyCode::F(18) if d.multi() => {
                    d.cycle(if shift || key.code == KeyCode::F(18) { -1 } else { 1 });
                    return ViewerSignal::Stay;
                }
                KeyCode::Char(']') if d.multi() => {
                    d.cycle(1);
                    return ViewerSignal::Stay;
                }
                KeyCode::Char('[') if d.multi() => {
                    d.cycle(-1);
                    return ViewerSignal::Stay;
                }
                KeyCode::PageDown if ctrl && d.multi() => {
                    d.cycle(1);
                    return ViewerSignal::Stay;
                }
                KeyCode::PageUp if ctrl && d.multi() => {
                    d.cycle(-1);
                    return ViewerSignal::Stay;
                }
                _ => {}
            }
        }
        d.current_mut().handle_key(key)
    }

    /// The F-key labels while the document shows: the part's own, with F4 and
    /// F8 leaving for the bytes and F6 switching sheets.
    pub(super) fn doc_footer_labels(&self) -> Option<[&'static str; 10]> {
        let d = self.doc.as_deref().filter(|_| self.show_doc)?;
        let mut labels = d.current().footer_labels();
        labels[3] = "Hex";
        labels[7] = "Raw";
        if d.multi() {
            labels[5] = "Sheet";
        }
        Some(labels)
    }

    /// Goto in the document's part, when it shows.
    pub(super) fn doc_goto(&mut self, value: &str, mode: GotoMode) -> Option<bool> {
        Some(self.doc_part_mut()?.goto(value, mode))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Sheet;
    use crate::ui::dialog::SearchReplaceParams;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn text_doc(md: &str) -> ViewerState {
        let mut v = ViewerState::new("report.docx".into(), b"PK\x03\x04raw bytes".to_vec());
        v.set_doc(DocView::new("report.docx", DocKind::Docx, Extracted::Text(md.into())));
        v
    }

    fn sheet_doc() -> ViewerState {
        let mut v = ViewerState::new("book.xlsx".into(), b"PK\x03\x04".to_vec());
        let sheets = vec![
            Sheet { name: "One".into(), tsv: b"a\tb\n1\t2\n3\t4\n".to_vec() },
            Sheet { name: "Two".into(), tsv: b"x\ny\n".to_vec() },
        ];
        v.set_doc(DocView::new("book.xlsx", DocKind::Sheet, Extracted::Sheets(sheets)));
        v
    }

    fn screen(v: &mut ViewerState, w: u16, h: u16) -> String {
        let theme = crate::ui::theme::Theme::mc();
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| super::super::render::render(f, f.area(), v, &theme, None)).unwrap();
        let b = t.backend().buffer();
        (0..h)
            .map(|y| (0..w).map(|x| b[(x, y)].symbol().to_string()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_text_document_renders_its_markdown_and_f8_shows_the_bytes() {
        let mut v = text_doc("# Title\n\nSome *starred* text\n");
        let s = screen(&mut v, 60, 8);
        assert!(s.contains("Title") && !s.contains("# Title"), "{s}");
        assert!(s.contains("Some *starred* text"), "inline markup is left as written: {s}");
        assert!(s.contains("report.docx [Word]"), "{s}");
        assert_eq!(v.footer_labels()[7], "Raw");

        v.handle_key(key(KeyCode::F(8)));
        assert!(v.active_doc().is_none());
        assert!(screen(&mut v, 60, 8).contains("raw bytes"));
        assert_eq!(v.footer_labels()[7], "Document");
        v.handle_key(key(KeyCode::F(8)));
        assert!(v.active_doc().is_some(), "F8 goes back to the document");

        v.handle_key(key(KeyCode::F(4)));
        assert!(v.is_hex() && v.active_doc().is_none(), "F4 shows the file's bytes in hex");
    }

    #[test]
    fn prose_wraps_at_word_boundaries() {
        let mut v = text_doc("alpha beta gamma delta epsilon\n");
        let s = screen(&mut v, 12, 8);
        let rows: Vec<&str> = s.lines().skip(1).take(3).map(str::trim_end).collect();
        assert_eq!(rows, ["alpha beta", "gamma delta", "epsilon"], "{s}");
    }

    #[test]
    fn outline_search_and_goto_act_on_the_document() {
        let mut v = text_doc("# One\n\ntext\n\n# Two\n\nneedle here\n");
        screen(&mut v, 60, 6);
        v.handle_key(key(KeyCode::F(6)));
        assert!(v.doc_part().unwrap().is_outline_open(), "F6 opens the document's outline");
        v.handle_key(key(KeyCode::Esc));
        assert!(v.active_doc().is_some(), "Esc closes the outline, not the view");

        let p = SearchReplaceParams {
            replace: false,
            search: "needle".into(),
            replacement: String::new(),
            regex: false,
            case_sensitive: false,
            whole_words: false,
            backwards: false,
            hex: false,
            find_all: false,
        };
        v.apply_search(&p);
        assert_eq!(v.search_seed(), "needle");
        assert!(v.doc_part().unwrap().last_match.is_some(), "the hit is in the document text");

        assert!(v.goto("5", GotoMode::Line));
        assert_eq!(v.doc_part().unwrap().top, 4);
    }

    #[test]
    fn sheets_switch_and_keep_their_own_cursor() {
        let mut v = sheet_doc();
        let s = screen(&mut v, 60, 8);
        assert!(s.contains("book.xlsx › One (1/2)"), "{s}");
        assert_eq!(v.footer_labels()[5], "Sheet");
        v.handle_key(key(KeyCode::Down));
        let row = |v: &ViewerState| v.doc_part().unwrap().table.as_ref().unwrap().grid.row;
        let before = row(&v);
        v.handle_key(key(KeyCode::Char(']')));
        assert!(screen(&mut v, 60, 8).contains("book.xlsx › Two (2/2)"));
        v.handle_key(key(KeyCode::F(6)));
        assert!(screen(&mut v, 60, 8).contains("One (1/2)"), "F6 wraps round to the first sheet");
        assert_eq!(row(&v), before, "the first sheet kept its cursor");
    }

    #[test]
    fn a_footer_click_reaches_the_document_keys() {
        use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let mut v = text_doc("# T\n");
        screen(&mut v, 80, 6);
        let f = v.footer_area;
        // The F8 slot: the bar divides the width into ten equal buttons.
        let col = f.x + f.width * 7 / 10 + 2;
        v.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row: f.y,
            modifiers: KeyModifiers::NONE,
        });
        assert!(v.active_doc().is_none(), "clicking F8 Raw leaves the document");
    }
}
