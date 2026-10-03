//! OpenDocument text (`.odt`) and presentations (`.odp`): everything is in the
//! package's `content.xml`.

use super::md::Md;
use super::xml::{El, Node};
use super::{MAX_OUTPUT, open_zip, part_xml};

/// The most a repeated row or column is expanded. Generated files mark the
/// rest of a sheet's width as one cell repeated a thousand-odd times.
const REPEAT_MAX: usize = 1024;

pub fn odt(bytes: &[u8]) -> Result<String, String> {
    let mut zip = open_zip(bytes)?;
    let content = part_xml(&mut zip, "content.xml", false).ok_or("no content.xml")?;
    let text = content.find("text").filter(|t| t.name == "text").ok_or("no document text")?;
    let mut md = Md::new(MAX_OUTPUT);
    blocks(text, 0, &mut md);
    Ok(md.finish())
}

pub fn odp(bytes: &[u8]) -> Result<String, String> {
    let mut zip = open_zip(bytes)?;
    let content = part_xml(&mut zip, "content.xml", false).ok_or("no content.xml")?;
    let pres = content.find("presentation").ok_or("no presentation")?;
    let mut md = Md::new(MAX_OUTPUT);
    let slide_word = crate::l10n::tr("Slide");
    for (i, page) in pres.elements().filter(|e| e.name == "page").enumerate() {
        if md.full() {
            break;
        }
        // A page named by the author (not the default "page3") titles it.
        let name = page.attr("name").filter(|n| !is_default_page_name(n));
        match name {
            Some(n) => md.heading(2, &format!("{slide_word} {}: {n}", i + 1)),
            None => md.heading(2, &format!("{slide_word} {}", i + 1)),
        }
        for frame in page.elements().filter(|e| e.name != "notes") {
            blocks(frame, 0, &mut md);
        }
    }
    Ok(md.finish())
}

fn is_default_page_name(n: &str) -> bool {
    let lower = n.to_ascii_lowercase();
    ["page", "slide"].iter().any(|p| {
        lower.strip_prefix(p).is_some_and(|rest| rest.trim().chars().all(|c| c.is_ascii_digit()))
    })
}

/// Block content: headings, paragraphs, lists, tables, and the text inside
/// frames and sections. `depth` is the list nesting.
fn blocks(parent: &El, depth: usize, md: &mut Md) {
    for e in parent.elements() {
        if md.full() {
            return;
        }
        match e.name.as_str() {
            "h" => {
                let level = e.attr("outline-level").and_then(|l| l.parse().ok()).unwrap_or(1);
                md.heading(level, &inline(e));
            }
            "p" => md.para(&inline(e)),
            "list" => list(e, depth, md),
            "table" => md.table(&table_rows(e)),
            "tracked-changes" | "notes" | "annotation" | "sequence-decls" | "variable-decls"
            | "user-field-decls" | "forms" => {}
            // Sections, frames, text boxes, shapes, index bodies…
            _ => blocks(e, depth, md),
        }
    }
}

fn list(l: &El, depth: usize, md: &mut Md) {
    for item in l.elements().filter(|e| e.name == "list-item" || e.name == "list-header") {
        let mut first = true;
        for c in item.elements() {
            match c.name.as_str() {
                "p" | "h" => {
                    let text = inline(c);
                    if first {
                        md.item(depth, "-", &text);
                        first = false;
                    } else {
                        // A further paragraph of the same item.
                        md.item(depth + 1, " ", &text);
                    }
                }
                "list" => list(c, depth + 1, md),
                _ => blocks(c, depth + 1, md),
            }
        }
    }
}

/// A paragraph's text: spans and links descended into, `text:s` as its count
/// of spaces, tabs as a space, line breaks kept. Notes and annotations are
/// left out of the running text.
fn inline(e: &El) -> String {
    let mut s = String::new();
    collect(e, &mut s);
    s
}

fn collect(e: &El, s: &mut String) {
    for n in &e.children {
        match n {
            Node::Text(t) => s.push_str(t),
            Node::El(c) => match c.name.as_str() {
                "s" => {
                    let n = c.attr("c").and_then(|n| n.parse::<usize>().ok()).unwrap_or(1);
                    s.push_str(&" ".repeat(n.min(64)));
                }
                "tab" => s.push(' '),
                "line-break" => s.push('\n'),
                "note" | "annotation" | "annotation-end" | "soft-page-break" | "bookmark-ref" => {}
                _ => collect(c, s),
            },
        }
    }
}

fn table_rows(t: &El) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    collect_rows(t, &mut rows);
    // Generated files pad with empty repeated cells; drop the empty tail.
    for r in &mut rows {
        while r.last().is_some_and(|c| c.trim().is_empty()) {
            r.pop();
        }
    }
    while rows.last().is_some_and(|r| r.is_empty()) {
        rows.pop();
    }
    rows
}

fn collect_rows(e: &El, rows: &mut Vec<Vec<String>>) {
    for c in e.elements() {
        match c.name.as_str() {
            "table-row" => {
                let mut row = Vec::new();
                for cell in c.elements() {
                    let text = match cell.name.as_str() {
                        "table-cell" => cell_text(cell),
                        "covered-table-cell" => String::new(),
                        _ => continue,
                    };
                    let repeat = repeated(cell, "number-columns-repeated");
                    for _ in 0..repeat {
                        if row.len() >= REPEAT_MAX {
                            break;
                        }
                        row.push(text.clone());
                    }
                }
                let repeat = repeated(c, "number-rows-repeated");
                for _ in 0..repeat {
                    if rows.len() >= REPEAT_MAX * 64 {
                        return;
                    }
                    rows.push(row.clone());
                }
            }
            "table-header-rows" | "table-rows" | "table-row-group" => collect_rows(c, rows),
            _ => {}
        }
    }
}

fn repeated(e: &El, attr: &str) -> usize {
    e.attr(attr).and_then(|n| n.parse::<usize>().ok()).unwrap_or(1).clamp(1, REPEAT_MAX)
}

fn cell_text(cell: &El) -> String {
    let mut ps = Vec::new();
    for c in cell.elements() {
        match c.name.as_str() {
            "p" | "h" => ps.push(inline(c)),
            _ => {
                let mut inner = Vec::new();
                c.find_all("p", &mut inner);
                ps.extend(inner.iter().map(|p| inline(p)));
            }
        }
    }
    ps.join("\n")
}

#[cfg(test)]
mod tests {
    use super::super::testkit::zip;
    use super::*;

    fn odt_of(body: &str) -> Vec<u8> {
        let content = format!(
            r#"<office:document-content xmlns:office="o" xmlns:text="t" xmlns:table="tb"><office:body><office:text>{body}</office:text></office:body></office:document-content>"#
        );
        zip(&[("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", &content)])
    }

    #[test]
    fn odt_headings_paragraphs_and_nested_lists() {
        let out = odt(&odt_of(
            r#"<text:h text:outline-level="2">Part</text:h>
               <text:p>a<text:s text:c="3"/>b<text:line-break/>c<text:note><text:note-body><text:p>fn</text:p></text:note-body></text:note></text:p>
               <text:list><text:list-item><text:p>one</text:p><text:list><text:list-item><text:p>inner</text:p></text:list-item></text:list></text:list-item></text:list>"#,
        ))
        .unwrap();
        assert_eq!(out, "## Part\n\na   b\nc\n\n- one\n  - inner\n");
    }

    #[test]
    fn odt_tables_expand_repeats_and_trim_padding() {
        let out = odt(&odt_of(
            r#"<table:table><table:table-row><table:table-cell><text:p>x</text:p></table:table-cell><table:table-cell table:number-columns-repeated="2"><text:p>y</text:p></table:table-cell><table:table-cell table:number-columns-repeated="1000"/></table:table-row></table:table>"#,
        ))
        .unwrap();
        assert!(out.contains("│ x │ y │ y │"), "{out}");
    }

    #[test]
    fn odp_pages_become_slides() {
        let content = r#"<office:document-content xmlns:office="o" xmlns:draw="d" xmlns:text="t" xmlns:presentation="p"><office:body><office:presentation>
            <draw:page draw:name="page1"><draw:frame><draw:text-box><text:p>Hello</text:p></draw:text-box></draw:frame><presentation:notes><text:p>secret</text:p></presentation:notes></draw:page>
            <draw:page draw:name="Summary"><draw:frame><draw:text-box><text:p>Bye</text:p></draw:text-box></draw:frame></draw:page>
            </office:presentation></office:body></office:document-content>"#;
        let out = odp(&zip(&[("content.xml", content)])).unwrap();
        assert_eq!(out, "## Slide 1\n\nHello\n\n## Slide 2: Summary\n\nBye\n");
    }
}
