//! Documents read for the viewer's document view: the text of Word, PowerPoint,
//! OpenDocument, EPUB and PDF files as the viewer's Markdown, and the sheets of
//! a spreadsheet as tables.
//!
//! Nothing here knows about the viewer — [`extract`] turns a file's bytes into
//! an [`Extracted`], which `viewer::doc` shows. The office formats are zip
//! packages of XML parts, read with the zip and quick-xml crates the program
//! already has; spreadsheets go through calamine, PDFs through pdf-extract.

mod epub;
mod md;
mod odf;
mod ooxml;
mod pdf;
mod sheet;
mod xml;

use std::io::Read;

/// Largest file the document view reads.
pub const MAX_INPUT: u64 = 64 * 1024 * 1024;
/// Most bytes inflated from one part of a package — a zip bomb's ceiling.
const MAX_PART: u64 = 32 * 1024 * 1024;
/// Most text the view is given; longer documents are cut with a note.
const MAX_OUTPUT: usize = 32 * 1024 * 1024;
/// Longest an extraction may run before the view gives up on it.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// What kind of document a file is, by its extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    Docx,
    Pptx,
    Odt,
    Odp,
    Epub,
    Pdf,
    /// Any spreadsheet calamine reads: xlsx, xlsm, xlsb, xls, ods.
    Sheet,
}

impl DocKind {
    /// The format's name, for the viewer's header.
    pub fn label(self) -> &'static str {
        match self {
            DocKind::Docx => "Word",
            DocKind::Pptx => "PowerPoint",
            DocKind::Odt => "OpenDocument Text",
            DocKind::Odp => "OpenDocument Presentation",
            DocKind::Epub => "EPUB",
            DocKind::Pdf => "PDF",
            DocKind::Sheet => "Spreadsheet",
        }
    }
}

/// The document kind of `name`, when the document view can read it.
pub fn doc_kind(name: &str) -> Option<DocKind> {
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    Some(match ext.to_ascii_lowercase().as_str() {
        "docx" | "docm" | "dotx" | "dotm" => DocKind::Docx,
        "pptx" | "pptm" | "ppsx" | "ppsm" | "potx" => DocKind::Pptx,
        "odt" | "ott" => DocKind::Odt,
        "odp" | "otp" => DocKind::Odp,
        "epub" => DocKind::Epub,
        "pdf" => DocKind::Pdf,
        "xlsx" | "xlsm" | "xlsb" | "xls" | "xla" | "xlam" | "ods" => DocKind::Sheet,
        _ => return None,
    })
}

/// A document as the view shows it.
#[derive(Debug)]
pub enum Extracted {
    /// The text, as the viewer's Markdown.
    Text(String),
    /// A spreadsheet's sheets, each as tab-separated text.
    Sheets(Vec<Sheet>),
}

#[derive(Debug)]
pub struct Sheet {
    pub name: String,
    pub tsv: Vec<u8>,
}

/// Read a document of `kind` from its bytes. Runs synchronously and can take a
/// while on a large file; [`extract_guarded`] is the way to call it from the
/// app.
pub fn extract(bytes: &[u8], kind: DocKind) -> Result<Extracted, String> {
    match kind {
        DocKind::Docx => ooxml::docx(bytes).map(Extracted::Text),
        DocKind::Pptx => ooxml::pptx(bytes).map(Extracted::Text),
        DocKind::Odt => odf::odt(bytes).map(Extracted::Text),
        DocKind::Odp => odf::odp(bytes).map(Extracted::Text),
        DocKind::Epub => epub::epub(bytes).map(Extracted::Text),
        DocKind::Pdf => pdf::pdf(bytes).map(Extracted::Text),
        DocKind::Sheet => sheet::sheets(bytes).map(Extracted::Sheets),
    }
}

/// [`extract`] on a thread of its own, safe against what a hostile or broken
/// file can do to a parser: a panic becomes an error (the PDF reader in
/// particular panics on malformed fonts), deep recursion has a large stack to
/// spend, and a file that never finishes is abandoned after [`TIMEOUT`] — its
/// thread runs on, but the viewer no longer waits for it.
pub async fn extract_guarded(bytes: Vec<u8>, kind: DocKind) -> Result<Extracted, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("doc-extract".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| extract(&bytes, kind)))
                    .unwrap_or_else(|_| Err("the reader failed on this file".into()));
            let _ = tx.send(result);
        });
    if spawned.is_err() {
        return Err("could not start the reader".into());
    }
    match tokio::time::timeout(TIMEOUT, rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err("the reader failed on this file".into()),
        Err(_) => Err("reading the document took too long".into()),
    }
}

type Zip<'a> = zip::ZipArchive<std::io::Cursor<&'a [u8]>>;

fn open_zip(bytes: &[u8]) -> Result<Zip<'_>, String> {
    zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("not a valid package: {e}"))
}

/// The bytes of the package part at `path`, inflated up to [`MAX_PART`].
fn part(zip: &mut Zip, path: &str) -> Option<Vec<u8>> {
    let path = path.trim_start_matches('/');
    let entry = zip.by_name(path).ok()?;
    let mut out = Vec::new();
    entry.take(MAX_PART).read_to_end(&mut out).ok()?;
    Some(out)
}

/// The part at `path`, parsed.
fn part_xml(zip: &mut Zip, path: &str, html: bool) -> Option<xml::El> {
    xml::parse(&part(zip, path)?, html).ok()
}

/// `target` resolved against the directory `base` (which ends in `/` or is
/// empty), with `.` and `..` segments folded and any `#fragment` dropped.
fn join_path(base: &str, target: &str) -> String {
    let target = target.split('#').next().unwrap_or("");
    let target = percent_decode(target);
    let full = if let Some(abs) = target.strip_prefix('/') {
        abs.to_string()
    } else {
        format!("{base}{target}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for seg in full.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// The directory part of `path`, with its trailing slash (empty for none).
fn dir_of(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..=i])
}

/// `%20` and friends decoded, as EPUB hrefs write them.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A package's relationships part (`_rels/x.rels`): id → target, resolved
/// against the directory of the part it belongs to.
fn rels(zip: &mut Zip, part_path: &str) -> std::collections::HashMap<String, String> {
    let dir = dir_of(part_path);
    let file = &part_path[dir.len()..];
    let mut map = std::collections::HashMap::new();
    if let Some(root) = part_xml(zip, &format!("{dir}_rels/{file}.rels"), false) {
        let mut found = Vec::new();
        root.find_all("Relationship", &mut found);
        for r in found {
            if let (Some(id), Some(target)) = (r.attr("Id"), r.attr("Target")) {
                if r.attr("TargetMode") == Some("External") {
                    continue;
                }
                map.insert(id.to_string(), join_path(dir, target));
            }
        }
    }
    map
}

#[cfg(test)]
pub(crate) mod testkit {
    //! Building small packages in memory for the extractor tests.
    use std::io::Write;

    pub fn zip(parts: &[(&str, &str)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        for (name, body) in parts {
            w.start_file(*name, opts).unwrap();
            w.write_all(body.as_bytes()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_extension() {
        assert_eq!(doc_kind("Report.DOCX"), Some(DocKind::Docx));
        assert_eq!(doc_kind("deck.pptx"), Some(DocKind::Pptx));
        assert_eq!(doc_kind("a.ods"), Some(DocKind::Sheet));
        assert_eq!(doc_kind("b.xls"), Some(DocKind::Sheet));
        assert_eq!(doc_kind("book.epub"), Some(DocKind::Epub));
        assert_eq!(doc_kind("paper.pdf"), Some(DocKind::Pdf));
        assert_eq!(doc_kind(".pdf"), None);
        assert_eq!(doc_kind("notes.txt"), None);
    }

    #[test]
    fn paths_resolve_against_their_part() {
        assert_eq!(join_path("ppt/", "slides/slide1.xml"), "ppt/slides/slide1.xml");
        assert_eq!(join_path("ppt/slides/", "../media/a.png"), "ppt/media/a.png");
        assert_eq!(join_path("OEBPS/", "/abs/x.html"), "abs/x.html");
        assert_eq!(join_path("OEBPS/", "Text/ch%201.xhtml#p3"), "OEBPS/Text/ch 1.xhtml");
    }

    #[tokio::test]
    async fn a_broken_file_is_an_error_not_a_crash() {
        assert!(extract_guarded(b"garbage".to_vec(), DocKind::Docx).await.is_err());
        assert!(extract_guarded(b"%PDF-1.4 garbage".to_vec(), DocKind::Pdf).await.is_err());
        assert!(extract_guarded(b"garbage".to_vec(), DocKind::Sheet).await.is_err());
    }
}
