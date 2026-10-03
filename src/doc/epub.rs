//! EPUB books: `META-INF/container.xml` names the package document (OPF),
//! whose spine lists the chapters in reading order; each chapter is XHTML.

use super::md::{Md, one_line};
use super::xml::{El, Node};
use super::{MAX_OUTPUT, dir_of, join_path, open_zip, part, part_xml};
use std::collections::HashMap;

pub fn epub(bytes: &[u8]) -> Result<String, String> {
    let mut zip = open_zip(bytes)?;
    let container =
        part_xml(&mut zip, "META-INF/container.xml", false).ok_or("no container.xml")?;
    let opf_path = container
        .find("rootfile")
        .and_then(|r| r.attr("full-path"))
        .ok_or("no package document")?
        .to_string();
    let opf = part_xml(&mut zip, &opf_path, false).ok_or("no package document")?;
    let base = dir_of(&opf_path).to_string();

    let mut manifest: HashMap<&str, (&str, &str)> = HashMap::new();
    if let Some(m) = opf.find("manifest") {
        for item in m.elements().filter(|e| e.name == "item") {
            if let (Some(id), Some(href)) = (item.attr("id"), item.attr("href")) {
                manifest.insert(id, (href, item.attr("media-type").unwrap_or("")));
            }
        }
    }
    let mut md = Md::new(MAX_OUTPUT);
    if let Some(title) = opf.find("title").map(|t| one_line(&t.text())).filter(|t| !t.is_empty()) {
        md.heading(1, &title);
    }
    let spine: Vec<&str> = opf
        .find("spine")
        .map(|s| {
            s.elements().filter(|e| e.name == "itemref").filter_map(|e| e.attr("idref")).collect()
        })
        .unwrap_or_default();
    let mut first = true;
    for idref in spine {
        if md.full() {
            break;
        }
        let Some((href, media)) = manifest.get(idref) else { continue };
        if !(media.contains("html") || media.is_empty()) {
            continue;
        }
        let path = join_path(&base, href);
        let Some(doc) = part(&mut zip, &path).and_then(|b| super::xml::parse(&b, true).ok()) else {
            continue;
        };
        let Some(body) = doc.find("body") else { continue };
        if !first {
            md.rule();
        }
        first = false;
        Html { md: &mut md }.blocks(body, 0);
    }
    Ok(md.finish())
}

struct Html<'a> {
    md: &'a mut Md,
}

impl Html<'_> {
    /// Block-level content. Inline content met directly in a block container
    /// (text loose in a `<div>`) is gathered into paragraphs.
    fn blocks(&mut self, e: &El, depth: usize) {
        let mut loose = String::new();
        for n in &e.children {
            if self.md.full() {
                return;
            }
            let c = match n {
                Node::Text(t) => {
                    push_source(&mut loose, t);
                    continue;
                }
                Node::El(c) => c,
            };
            if !is_block(&c.name) {
                inline(c, &mut loose);
                continue;
            }
            self.flush(&mut loose);
            match c.name.as_str() {
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    let level = c.name[1..].parse().unwrap_or(1);
                    self.md.heading(level, &text_of(c));
                }
                "p" => self.md.para(&text_of(c)),
                "ul" | "ol" => self.list(c, depth),
                "pre" => self.md.code(&c.text()),
                "blockquote" => self.md.quote(&text_of(c)),
                "table" => self.md.table(&table_rows(c)),
                "hr" => self.md.rule(),
                "head" | "script" | "style" => {}
                "nav" if c.attr("type").is_some_and(|t| t.contains("toc")) => {}
                _ => self.blocks(c, depth),
            }
        }
        self.flush(&mut loose);
    }

    fn flush(&mut self, loose: &mut String) {
        let t = collapse(loose);
        if !t.trim().is_empty() {
            self.md.para(&t);
        }
        loose.clear();
    }

    fn list(&mut self, l: &El, depth: usize) {
        let ordered = l.name == "ol";
        let first = l.attr("start").and_then(|s| s.parse::<usize>().ok()).unwrap_or(1);
        for (n, li) in (first..).zip(l.elements().filter(|e| e.name == "li")) {
            let mut text = String::new();
            let mut nested = Vec::new();
            for c in &li.children {
                match c {
                    Node::El(e) if e.name == "ul" || e.name == "ol" => nested.push(e),
                    Node::El(e) => inline(e, &mut text),
                    Node::Text(t) => push_source(&mut text, t),
                }
            }
            let marker = if ordered { format!("{n}.") } else { "-".into() };
            self.md.item(depth, &marker, &collapse(&text));
            for sub in nested {
                self.list(sub, depth + 1);
            }
        }
    }
}

fn is_block(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "section"
            | "article"
            | "aside"
            | "header"
            | "footer"
            | "main"
            | "nav"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "ul"
            | "ol"
            | "pre"
            | "blockquote"
            | "table"
            | "hr"
            | "figure"
            | "figcaption"
            | "dl"
            | "dt"
            | "dd"
            | "head"
            | "script"
            | "style"
            | "body"
            | "center"
    )
}

/// Inline text, HTML-style: `<br>` a line break, a picture its alt text (books
/// often set chapter titles as images), everything else its text.
fn inline(e: &El, out: &mut String) {
    match e.name.as_str() {
        "br" => out.push('\n'),
        "img" => push_source(out, e.attr("alt").unwrap_or("")),
        "script" | "style" => {}
        _ => {
            for n in &e.children {
                match n {
                    Node::Text(t) => push_source(out, t),
                    Node::El(c) => inline(c, out),
                }
            }
        }
    }
}

/// Source text, whose line breaks are only formatting: they read as spaces,
/// so the only `\n` in gathered text is a `<br>`.
fn push_source(out: &mut String, t: &str) {
    out.extend(t.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }));
}

fn text_of(e: &El) -> String {
    let mut s = String::new();
    inline(e, &mut s);
    collapse(&s)
}

/// Whitespace collapsed as HTML does, keeping `<br>`'s line breaks (the only
/// `\n` left by [`push_source`]). No-break spaces stay.
fn collapse(s: &str) -> String {
    s.split('\n')
        .map(|seg| {
            seg.split([' ', '\t', '\r']).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn table_rows(t: &El) -> Vec<Vec<String>> {
    let mut trs = Vec::new();
    t.find_all("tr", &mut trs);
    trs.iter()
        .map(|tr| tr.elements().filter(|c| c.name == "td" || c.name == "th").map(text_of).collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::testkit::zip;
    use super::*;

    #[test]
    fn chapters_follow_the_spine_and_html_maps_to_markdown() {
        let container = r#"<container><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#;
        let opf = r#"<package xmlns:dc="dc"><metadata><dc:title>My Book</dc:title></metadata>
            <manifest><item id="a" href="Text/a.xhtml" media-type="application/xhtml+xml"/><item id="b" href="Text/b.xhtml" media-type="application/xhtml+xml"/></manifest>
            <spine><itemref idref="b"/><itemref idref="a"/></spine></package>"#;
        let a = r#"<html><head><title>x</title></head><body><h2><img src="t.jpg" alt="Second"/></h2><ol><li>one</li><li>two</li></ol><blockquote><p>Quoted.</p></blockquote></body></html>"#;
        let b = r#"<html><body><h2>First</h2><p>Hello&nbsp;there,
            <em>world</em>.<br/>Next line</p></body></html>"#;
        let bytes = zip(&[
            ("mimetype", "application/epub+zip"),
            ("META-INF/container.xml", container),
            ("OEBPS/content.opf", opf),
            ("OEBPS/Text/a.xhtml", a),
            ("OEBPS/Text/b.xhtml", b),
        ]);
        let out = epub(&bytes).unwrap();
        assert_eq!(
            out,
            "# My Book\n\n## First\n\nHello\u{a0}there, world.\nNext line\n\n---\n\n## Second\n\n1. one\n2. two\n\n> Quoted.\n"
        );
    }
}
