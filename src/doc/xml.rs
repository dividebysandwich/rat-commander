//! A small element tree over quick-xml, which is all the document formats need:
//! each is a few XML parts walked from the top, and walking a tree is far
//! plainer than keeping a stack of "where am I" flags over a stream of events.
//!
//! Element names are kept as *local* names (`w:p` → `p`), since the prefixes
//! are only conventions; attributes keep theirs, for the few elements that
//! carry the same local name twice. Parsing is lenient — mismatched end tags and undeclared
//! entities do not stop it — because a viewer that refuses a slightly broken
//! file is worse than one that shows most of it.

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

#[derive(Debug, Default)]
pub struct El {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

#[derive(Debug)]
pub enum Node {
    El(El),
    Text(String),
}

impl El {
    /// The value of attribute `name`: matched as written (`r:id`) first, then
    /// by local name (`val` finds `w:val`). Elements like PowerPoint's
    /// `<p:sldId id="256" r:id="rId2">` carry both forms.
    pub fn attr(&self, name: &str) -> Option<&str> {
        let local = |k: &str| k.rsplit(':').next() == Some(name);
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .or_else(|| self.attrs.iter().find(|(k, _)| local(k)))
            .map(|(_, v)| v.as_str())
    }

    /// Child elements, in order.
    pub fn elements(&self) -> impl Iterator<Item = &El> {
        self.children.iter().filter_map(|n| match n {
            Node::El(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    /// The first child element named `name`.
    pub fn child(&self, name: &str) -> Option<&El> {
        self.elements().find(|e| e.name == name)
    }

    /// The first element named `name` anywhere below (depth first).
    pub fn find(&self, name: &str) -> Option<&El> {
        for e in self.elements() {
            if e.name == name {
                return Some(e);
            }
            if let Some(f) = e.find(name) {
                return Some(f);
            }
        }
        None
    }

    /// Every element named `name` below, outermost first and not descending
    /// into a match.
    pub fn find_all<'a>(&'a self, name: &str, out: &mut Vec<&'a El>) {
        for e in self.elements() {
            if e.name == name {
                out.push(e);
            } else {
                e.find_all(name, out);
            }
        }
    }

    /// All text below, concatenated.
    pub fn text(&self) -> String {
        let mut s = String::new();
        self.collect_text(&mut s);
        s
    }

    fn collect_text(&self, s: &mut String) {
        for n in &self.children {
            match n {
                Node::Text(t) => s.push_str(t),
                Node::El(e) => e.collect_text(s),
            }
        }
    }
}

/// Parse `xml` into a synthetic root element holding the document's top-level
/// nodes. `html` additionally resolves the common HTML named entities (EPUB's
/// XHTML uses `&nbsp;` and friends without declaring them).
pub fn parse(xml: &[u8], html: bool) -> Result<El, String> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().check_end_names = false;
    reader.config_mut().allow_unmatched_ends = true;
    let mut stack: Vec<El> = vec![El::default()];
    let mut buf = Vec::new();
    loop {
        let event = reader.read_event_into(&mut buf).map_err(|e| format!("XML: {e}"))?;
        match event {
            Event::Start(ref e) => stack.push(element(e)),
            Event::Empty(ref e) => push(&mut stack, Node::El(element(e))),
            Event::End(_) => {
                if stack.len() > 1 {
                    let done = stack.pop().unwrap_or_default();
                    push(&mut stack, Node::El(done));
                }
            }
            Event::Text(t) => {
                if let Ok(s) = t.xml10_content() {
                    push_text(&mut stack, &s);
                }
            }
            Event::CData(c) => {
                if let Ok(s) = c.decode() {
                    push_text(&mut stack, &s);
                }
            }
            Event::GeneralRef(r) => {
                let resolved = match r.resolve_char_ref() {
                    Ok(Some(c)) => Some(c.to_string()),
                    _ => r.decode().ok().and_then(|name| entity(&name, html).map(str::to_string)),
                };
                if let Some(s) = resolved {
                    push_text(&mut stack, &s);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    // Close whatever was left open by a truncated or sloppy document.
    while stack.len() > 1 {
        let done = stack.pop().unwrap_or_default();
        push(&mut stack, Node::El(done));
    }
    Ok(stack.pop().unwrap_or_default())
}

fn push(stack: &mut [El], node: Node) {
    if let Some(top) = stack.last_mut() {
        top.children.push(node);
    }
}

fn push_text(stack: &mut [El], s: &str) {
    let Some(top) = stack.last_mut() else { return };
    // Adjacent text (split around an entity reference) is one run.
    if let Some(Node::Text(prev)) = top.children.last_mut() {
        prev.push_str(s);
    } else {
        top.children.push(Node::Text(s.to_string()));
    }
}

fn element(e: &BytesStart) -> El {
    let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
    let attrs = e
        .attributes()
        .flatten()
        .map(|a| {
            let key = String::from_utf8_lossy(a.key.as_ref()).into_owned();
            let value = a
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map(|v| v.into_owned())
                .unwrap_or_else(|_| String::from_utf8_lossy(&a.value).into_owned());
            (key, value)
        })
        .collect();
    El { name, attrs, children: Vec::new() }
}

/// A named entity: XML's five, and with `html` the handful of HTML ones books
/// actually use.
fn entity(name: &str, html: bool) -> Option<&'static str> {
    let xml = match name {
        "lt" => Some("<"),
        "gt" => Some(">"),
        "amp" => Some("&"),
        "apos" => Some("'"),
        "quot" => Some("\""),
        _ => None,
    };
    if xml.is_some() || !html {
        return xml;
    }
    Some(match name {
        "nbsp" => "\u{a0}",
        "shy" => "",
        "ndash" => "–",
        "mdash" => "—",
        "hellip" => "…",
        "lsquo" => "‘",
        "rsquo" => "’",
        "ldquo" => "“",
        "rdquo" => "”",
        "laquo" => "«",
        "raquo" => "»",
        "bull" => "•",
        "middot" => "·",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "deg" => "°",
        "times" => "×",
        "eacute" => "é",
        "egrave" => "è",
        "agrave" => "à",
        "auml" => "ä",
        "ouml" => "ö",
        "uuml" => "ü",
        "szlig" => "ß",
        "thinsp" | "ensp" | "emsp" => " ",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_tree_with_local_names_and_entities() {
        let root = parse(
            br#"<?xml version="1.0"?><w:doc xmlns:w="x"><w:p w:val="a&amp;b">x &lt; y&#x263A;<w:br/></w:p></w:doc>"#,
            false,
        )
        .unwrap();
        let doc = root.child("doc").unwrap();
        let p = doc.child("p").unwrap();
        assert_eq!(p.attr("val"), Some("a&b"));
        assert_eq!(p.text(), "x < y☺");
        assert!(p.child("br").is_some());
    }

    #[test]
    fn html_entities_and_sloppy_markup_still_parse() {
        let root = parse(b"<html><p>a&nbsp;b&mdash;c<br></p></html>", true).unwrap();
        let p = root.find("p").unwrap();
        assert_eq!(p.text(), "a\u{a0}b—c");
    }
}
