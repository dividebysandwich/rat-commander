//! Word (`.docx`) and PowerPoint (`.pptx`): Office Open XML packages.
//!
//! A Word document is `word/document.xml`, whose paragraphs name a style
//! (`styles.xml` says which styles are headings) and maybe a numbering
//! (`numbering.xml` says whether that is bullets or numbers). A presentation's
//! slides are listed, in order, in `ppt/presentation.xml` — not in the order
//! their file names sort.

use super::md::Md;
use super::xml::{El, Node};
use super::{MAX_OUTPUT, Zip, open_zip, part_xml, rels};
use std::collections::HashMap;

pub fn docx(bytes: &[u8]) -> Result<String, String> {
    let mut zip = open_zip(bytes)?;
    let doc = part_xml(&mut zip, "word/document.xml", false).ok_or("no document body")?;
    let body = doc.find("body").ok_or("no document body")?;
    let mut w = Word {
        headings: heading_styles(&mut zip),
        numbering: Numbering::read(&mut zip),
        counters: HashMap::new(),
        md: Md::new(MAX_OUTPUT),
    };
    w.blocks(body);
    if let Some(notes) = part_xml(&mut zip, "word/footnotes.xml", false) {
        let texts: Vec<String> = notes
            .find("footnotes")
            .into_iter()
            .flat_map(|f| f.elements())
            // Ids 0 and -1 are the separator lines, not notes.
            .filter(|n| n.attr("id").and_then(|i| i.parse::<i64>().ok()).is_some_and(|i| i > 0))
            .map(paragraphs_text)
            .filter(|t| !t.trim().is_empty())
            .collect();
        if !texts.is_empty() {
            w.md.rule();
            for (i, t) in texts.iter().enumerate() {
                w.md.item(0, &format!("{}.", i + 1), t);
            }
        }
    }
    Ok(w.md.finish())
}

/// Paragraph style id → heading level, from `styles.xml`. A style is a heading
/// when it carries an outline level, or is named "heading N" / "Title" — style
/// *ids* are localized ("berschrift1"), the names are not.
fn heading_styles(zip: &mut Zip) -> HashMap<String, usize> {
    let mut map = HashMap::new();
    let Some(root) = part_xml(zip, "word/styles.xml", false) else { return map };
    let mut styles = Vec::new();
    root.find_all("style", &mut styles);
    for s in styles {
        let Some(id) = s.attr("styleId") else { continue };
        let outline = s
            .child("pPr")
            .and_then(|p| p.child("outlineLvl"))
            .and_then(|o| o.attr("val"))
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|&l| l < 9)
            .map(|l| l + 1);
        let name = s.child("name").and_then(|n| n.attr("val")).unwrap_or("").to_ascii_lowercase();
        let by_name = if name == "title" {
            Some(1)
        } else {
            name.strip_prefix("heading ").and_then(|n| n.trim().parse::<usize>().ok())
        };
        if let Some(level) = outline.or(by_name) {
            map.insert(id.to_string(), level.clamp(1, 6));
        }
    }
    map
}

/// Which lists are bulleted and which numbered: numId → (level → is bullet).
struct Numbering {
    bullets: HashMap<String, HashMap<String, bool>>,
}

impl Numbering {
    fn read(zip: &mut Zip) -> Self {
        let mut bullets = HashMap::new();
        if let Some(root) = part_xml(zip, "word/numbering.xml", false)
            && let Some(numbering) = root.find("numbering")
        {
            let mut abstracts: HashMap<&str, HashMap<String, bool>> = HashMap::new();
            for a in numbering.elements().filter(|e| e.name == "abstractNum") {
                let Some(id) = a.attr("abstractNumId") else { continue };
                let levels = a
                    .elements()
                    .filter(|l| l.name == "lvl")
                    .filter_map(|l| {
                        let fmt = l.child("numFmt").and_then(|f| f.attr("val")).unwrap_or("bullet");
                        Some((l.attr("ilvl")?.to_string(), fmt == "bullet" || fmt == "none"))
                    })
                    .collect();
                abstracts.insert(id, levels);
            }
            for n in numbering.elements().filter(|e| e.name == "num") {
                let (Some(id), Some(abs)) =
                    (n.attr("numId"), n.child("abstractNumId").and_then(|a| a.attr("val")))
                else {
                    continue;
                };
                if let Some(levels) = abstracts.get(abs) {
                    bullets.insert(id.to_string(), levels.clone());
                }
            }
        }
        Numbering { bullets }
    }

    fn is_bullet(&self, num: &str, level: &str) -> bool {
        self.bullets.get(num).and_then(|l| l.get(level)).copied().unwrap_or(true)
    }
}

struct Word {
    headings: HashMap<String, usize>,
    numbering: Numbering,
    /// The next number of each (list, level).
    counters: HashMap<(String, usize), usize>,
    md: Md,
}

impl Word {
    /// The block content of the body, a cell, or a content control.
    fn blocks(&mut self, parent: &El) {
        for e in parent.elements() {
            if self.md.full() {
                return;
            }
            match e.name.as_str() {
                "p" => self.paragraph(e),
                "tbl" => {
                    let rows = table_rows(e);
                    self.md.table(&rows);
                }
                "sdt" => {
                    if let Some(c) = e.child("sdtContent") {
                        self.blocks(c);
                    }
                }
                "customXml" | "ins" | "smartTag" => self.blocks(e),
                _ => {}
            }
        }
    }

    fn paragraph(&mut self, p: &El) {
        let text = run_text(p);
        let ppr = p.child("pPr");
        let level = ppr
            .and_then(|pp| pp.child("outlineLvl"))
            .and_then(|o| o.attr("val"))
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|&l| l < 9)
            .map(|l| l + 1)
            .or_else(|| {
                let style = ppr.and_then(|pp| pp.child("pStyle")).and_then(|s| s.attr("val"))?;
                self.headings.get(style).copied()
            });
        if let Some(level) = level {
            self.md.heading(level, &text);
            return;
        }
        let num = ppr.and_then(|pp| pp.child("numPr"));
        let num_id = num.and_then(|n| n.child("numId")).and_then(|i| i.attr("val"));
        if let Some(num_id) = num_id.filter(|id| *id != "0") {
            let ilvl = num.and_then(|n| n.child("ilvl")).and_then(|i| i.attr("val")).unwrap_or("0");
            let depth = ilvl.parse::<usize>().unwrap_or(0);
            // A shallower item restarts the numbering of the levels below it.
            self.counters.retain(|(id, d), _| id != num_id || *d <= depth);
            let marker = if self.numbering.is_bullet(num_id, ilvl) {
                "-".to_string()
            } else {
                let n = self.counters.entry((num_id.to_string(), depth)).or_insert(0);
                *n += 1;
                format!("{n}.")
            };
            if !text.trim().is_empty() {
                self.md.item(depth, &marker, &text);
            }
            return;
        }
        self.md.para(&text);
    }
}

/// The text of a paragraph's runs. Deleted text (tracked changes), field codes
/// and the fallback copy of alternate content are not part of what reads.
fn run_text(p: &El) -> String {
    let mut s = String::new();
    collect_runs(p, &mut s);
    s
}

fn collect_runs(e: &El, s: &mut String) {
    for n in &e.children {
        let Node::El(c) = n else { continue };
        match c.name.as_str() {
            "t" => s.push_str(&c.text()),
            "tab" | "ptab" => s.push(' '),
            "br" | "cr" => s.push('\n'),
            "noBreakHyphen" => s.push('‑'),
            "del" | "delText" | "instrText" | "Fallback" | "pPr" | "rPr" => {}
            // A text box's paragraphs: their own lines.
            "txbxContent" => {
                for p in c.elements().filter(|p| p.name == "p") {
                    s.push('\n');
                    collect_runs(p, s);
                }
            }
            _ => collect_runs(c, s),
        }
    }
}

/// The paragraphs below `e` (a note, a cell), one per line.
fn paragraphs_text(e: &El) -> String {
    let mut ps = Vec::new();
    e.find_all("p", &mut ps);
    ps.iter().map(|p| run_text(p)).collect::<Vec<_>>().join("\n")
}

fn table_rows(tbl: &El) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    for tr in tbl.elements().filter(|e| e.name == "tr") {
        let mut row = Vec::new();
        for tc in tr.elements().filter(|e| e.name == "tc") {
            let tcpr = tc.child("tcPr");
            let continued = tcpr
                .and_then(|p| p.child("vMerge"))
                .is_some_and(|v| v.attr("val").is_none_or(|v| v == "continue"));
            row.push(if continued { String::new() } else { paragraphs_text(tc) });
            let span = tcpr
                .and_then(|p| p.child("gridSpan"))
                .and_then(|g| g.attr("val"))
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(1);
            row.extend(std::iter::repeat_n(String::new(), span.clamp(1, 64) - 1));
        }
        rows.push(row);
    }
    rows
}

pub fn pptx(bytes: &[u8]) -> Result<String, String> {
    let mut zip = open_zip(bytes)?;
    let pres = part_xml(&mut zip, "ppt/presentation.xml", false).ok_or("no presentation")?;
    let rel = rels(&mut zip, "ppt/presentation.xml");
    let mut ids = Vec::new();
    if let Some(list) = pres.find("sldIdLst") {
        ids = list.elements().filter_map(|s| s.attr("r:id")).collect();
    }
    let mut md = Md::new(MAX_OUTPUT);
    let slide_word = crate::l10n::tr("Slide");
    for (i, id) in ids.iter().enumerate() {
        if md.full() {
            break;
        }
        let Some(path) = rel.get(*id) else { continue };
        let Some(slide) = part_xml(&mut zip, path, false) else { continue };
        let mut shapes = Vec::new();
        if let Some(tree) = slide.find("spTree") {
            collect_shapes(tree, &mut shapes);
        }
        let title = shapes.iter().find_map(|s| match s {
            Shape::Text { kind: Placeholder::Title, paras } => {
                Some(paras.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(" "))
            }
            _ => None,
        });
        match title.as_deref().map(super::md::one_line).filter(|t| !t.is_empty()) {
            Some(t) => md.heading(2, &format!("{slide_word} {}: {t}", i + 1)),
            None => md.heading(2, &format!("{slide_word} {}", i + 1)),
        }
        for s in &shapes {
            match s {
                Shape::Text { kind: Placeholder::Title, .. } => {}
                Shape::Text { kind: Placeholder::Body, paras } => {
                    for (lvl, t) in paras {
                        if !t.trim().is_empty() {
                            md.item(*lvl, "-", t);
                        }
                    }
                }
                Shape::Text { kind: Placeholder::None, paras } => {
                    let text: Vec<&str> = paras.iter().map(|(_, t)| t.as_str()).collect();
                    md.para(&text.join("\n"));
                }
                Shape::Table(rows) => md.table(rows),
            }
        }
    }
    Ok(md.finish())
}

#[derive(Clone, Copy, PartialEq)]
enum Placeholder {
    Title,
    Body,
    None,
}

enum Shape {
    /// A text shape's paragraphs, each with its outline level.
    Text {
        kind: Placeholder,
        paras: Vec<(usize, String)>,
    },
    Table(Vec<Vec<String>>),
}

fn collect_shapes(tree: &El, out: &mut Vec<Shape>) {
    for e in tree.elements() {
        match e.name.as_str() {
            "sp" => {
                let ph =
                    e.child("nvSpPr").and_then(|n| n.child("nvPr")).and_then(|n| n.child("ph"));
                let kind = match ph.map(|p| p.attr("type").unwrap_or("body")) {
                    Some("title" | "ctrTitle") => Placeholder::Title,
                    Some("body" | "obj" | "subTitle") => Placeholder::Body,
                    // Footers, slide numbers and dates repeat on every slide.
                    Some("ftr" | "sldNum" | "dt" | "hdr") => continue,
                    _ => Placeholder::None,
                };
                if let Some(body) = e.child("txBody") {
                    let paras = drawing_paras(body);
                    if paras.iter().any(|(_, t)| !t.trim().is_empty()) {
                        out.push(Shape::Text { kind, paras });
                    }
                }
            }
            "grpSp" => collect_shapes(e, out),
            "graphicFrame" => {
                if let Some(tbl) = e.find("tbl") {
                    let rows = tbl
                        .elements()
                        .filter(|r| r.name == "tr")
                        .map(|tr| {
                            tr.elements()
                                .filter(|c| c.name == "tc")
                                .map(|tc| {
                                    let paras =
                                        tc.child("txBody").map(drawing_paras).unwrap_or_default();
                                    paras.into_iter().map(|(_, t)| t).collect::<Vec<_>>().join("\n")
                                })
                                .collect()
                        })
                        .collect();
                    out.push(Shape::Table(rows));
                }
            }
            "AlternateContent" => {
                if let Some(choice) = e.child("Choice") {
                    collect_shapes(choice, out);
                }
            }
            _ => {}
        }
    }
}

/// DrawingML paragraphs (`a:p`): their level and text.
fn drawing_paras(body: &El) -> Vec<(usize, String)> {
    body.elements()
        .filter(|p| p.name == "p")
        .map(|p| {
            let lvl = p
                .child("pPr")
                .and_then(|pp| pp.attr("lvl"))
                .and_then(|l| l.parse::<usize>().ok())
                .unwrap_or(0);
            let mut s = String::new();
            for r in p.elements() {
                match r.name.as_str() {
                    "r" | "fld" => {
                        if let Some(t) = r.child("t") {
                            s.push_str(&t.text());
                        }
                    }
                    "br" => s.push('\n'),
                    _ => {}
                }
            }
            (lvl, s)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::testkit::zip;
    use super::*;

    const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006""#;

    fn docx_of(body: &str, styles: &str, numbering: &str) -> Vec<u8> {
        let doc =
            format!(r#"<?xml version="1.0"?><w:document {W}><w:body>{body}</w:body></w:document>"#);
        let styles = format!(r#"<w:styles {W}>{styles}</w:styles>"#);
        let numbering = format!(r#"<w:numbering {W}>{numbering}</w:numbering>"#);
        zip(&[
            ("word/document.xml", &doc),
            ("word/styles.xml", &styles),
            ("word/numbering.xml", &numbering),
        ])
    }

    fn p(style: &str, text: &str) -> String {
        format!(
            r#"<w:p><w:pPr><w:pStyle w:val="{style}"/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#
        )
    }

    #[test]
    fn word_headings_come_from_styles_by_name_or_outline_level() {
        let styles = r#"
            <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/></w:style>
            <w:style w:type="paragraph" w:styleId="berschrift2"><w:name w:val="Überschrift 2"/><w:pPr><w:outlineLvl w:val="1"/></w:pPr></w:style>"#;
        let body =
            [p("Heading1", "Intro"), p("berschrift2", "Teil"), p("Normal", "Body text")].concat();
        let out = docx(&docx_of(&body, styles, "")).unwrap();
        assert_eq!(out, "# Intro\n\n## Teil\n\nBody text\n");
    }

    #[test]
    fn word_lists_number_or_bullet_by_their_numbering() {
        let numbering = r#"
            <w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:numFmt w:val="decimal"/></w:lvl><w:lvl w:ilvl="1"><w:numFmt w:val="bullet"/></w:lvl></w:abstractNum>
            <w:num w:numId="5"><w:abstractNumId w:val="0"/></w:num>"#;
        let item = |lvl: u8, t: &str| {
            format!(
                r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="{lvl}"/><w:numId w:val="5"/></w:numPr></w:pPr><w:r><w:t>{t}</w:t></w:r></w:p>"#
            )
        };
        let body = [item(0, "one"), item(1, "sub"), item(0, "two")].concat();
        let out = docx(&docx_of(&body, "", numbering)).unwrap();
        assert_eq!(out, "1. one\n  - sub\n2. two\n");
    }

    #[test]
    fn word_skips_deleted_text_field_codes_and_fallbacks() {
        let body = r#"<w:p>
            <w:r><w:t xml:space="preserve">kept </w:t></w:r>
            <w:del><w:r><w:delText>gone</w:delText></w:r></w:del>
            <w:r><w:instrText>PAGE</w:instrText></w:r>
            <w:r><w:tab/><w:t>A&amp;B &#x263A;</w:t></w:r>
            <mc:AlternateContent><mc:Choice><w:r><w:t>once</w:t></w:r></mc:Choice><mc:Fallback><w:r><w:t>twice</w:t></w:r></mc:Fallback></mc:AlternateContent>
        </w:p>"#;
        let out = docx(&docx_of(body, "", "")).unwrap();
        assert_eq!(out, "kept  A&B ☺once\n");
    }

    #[test]
    fn word_tables_are_drawn() {
        let cell = |t: &str| format!("<w:tc><w:p><w:r><w:t>{t}</w:t></w:r></w:p></w:tc>");
        let body = format!(
            "<w:tbl><w:tr>{}{}</w:tr><w:tr>{}{}</w:tr></w:tbl>",
            cell("a"),
            cell("bb"),
            cell("c"),
            cell("d")
        );
        let out = docx(&docx_of(&body, "", "")).unwrap();
        assert!(out.contains("│ a │ bb │") && out.contains("│ c │ d  │"), "{out}");
    }

    #[test]
    fn slides_follow_the_presentation_order_with_titles_and_bullets() {
        let pres = r#"<p:presentation xmlns:p="p" xmlns:r="r"><p:sldIdLst>
            <p:sldId id="256" r:id="rId3"/><p:sldId id="257" r:id="rId2"/></p:sldIdLst></p:presentation>"#;
        let rels = r#"<Relationships><Relationship Id="rId2" Target="slides/slide1.xml"/><Relationship Id="rId3" Target="slides/slide2.xml"/></Relationships>"#;
        let slide = |title: &str, body: &str| {
            format!(
                r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree>
                <p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>{title}</a:t></a:r></a:p></p:txBody></p:sp>
                <p:sp><p:nvSpPr><p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr><p:txBody>{body}</p:txBody></p:sp>
                </p:spTree></p:cSld></p:sld>"#
            )
        };
        let s1 = slide("Later", "<a:p><a:r><a:t>x</a:t></a:r></a:p>");
        let s2 = slide(
            "First",
            r#"<a:p><a:r><a:t>point</a:t></a:r></a:p><a:p><a:pPr lvl="1"/><a:r><a:t>detail</a:t></a:r></a:p>"#,
        );
        let bytes = zip(&[
            ("ppt/presentation.xml", pres),
            ("ppt/_rels/presentation.xml.rels", rels),
            ("ppt/slides/slide1.xml", &s1),
            ("ppt/slides/slide2.xml", &s2),
        ]);
        let out = pptx(&bytes).unwrap();
        assert_eq!(out, "## Slide 1: First\n\n- point\n  - detail\n\n## Slide 2: Later\n\n- x\n");
    }
}
