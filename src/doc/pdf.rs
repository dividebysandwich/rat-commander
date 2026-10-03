//! PDF text, page by page, through pdf-extract. Text only: a scanned page is
//! a picture and reads as having none.

use super::MAX_OUTPUT;
use super::md::{Md, sanitize};

pub fn pdf(bytes: &[u8]) -> Result<String, String> {
    let pages = pdf_extract::extract_text_from_mem_by_pages(bytes).map_err(|e| e.to_string())?;
    if pages.is_empty() {
        return Err("no pages".into());
    }
    let page_word = crate::l10n::tr("Page");
    let empty = crate::l10n::tr("(no text on this page)");
    let mut md = Md::new(MAX_OUTPUT);
    for (i, page) in pages.iter().enumerate() {
        if md.full() {
            break;
        }
        md.heading(2, &format!("{page_word} {}", i + 1));
        let text = tidy(page);
        if text.is_empty() {
            md.para(&empty);
        } else {
            md.para(&text);
        }
    }
    Ok(md.finish())
}

/// A page's text with form feeds dropped, trailing space trimmed, runs of
/// blank lines folded to one, and each line kept from reading as Markdown.
/// Blank lines become a no-break space so the page stays one paragraph block
/// with its own spacing.
fn tidy(page: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for line in page.replace('\u{c}', "").lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            if out.last().is_some_and(|l| l != "\u{a0}") {
                out.push("\u{a0}".into());
            }
            continue;
        }
        out.push(sanitize(line));
    }
    while out.last().is_some_and(|l| l == "\u{a0}") {
        out.pop();
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-page PDF drawing `text` in Helvetica, with a correct xref table.
    fn minimal_pdf(text: &str) -> Vec<u8> {
        let stream = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
        for off in offsets {
            out.extend(format!("{off:010} 00000 n \n").bytes());
        }
        out.extend(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .bytes(),
        );
        out
    }

    #[test]
    fn pages_are_headed_and_hold_their_text() {
        let out = pdf(&minimal_pdf("Hello PDF")).unwrap();
        assert!(out.starts_with("## Page 1\n\n"), "{out}");
        assert!(out.contains("Hello PDF"), "{out}");
    }

    #[test]
    fn blank_runs_fold_and_lookalikes_are_defused() {
        assert_eq!(tidy("a\n\n\n\n# b\n\u{c}\n"), "a\n\u{a0}\n\u{a0}# b");
    }
}
