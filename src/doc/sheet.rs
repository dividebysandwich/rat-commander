//! Spreadsheets through calamine: each sheet becomes tab-separated text, which
//! the viewer's table view already knows how to page, scroll and search.

use super::Sheet;
use calamine::{Data, Reader};
use std::io::Write;

/// Most TSV bytes made from one workbook; the sheet that crosses it is cut.
const MAX_TSV: usize = 64 * 1024 * 1024;
/// A sheet whose used range starts further out than this is not padded to its
/// position (columns and rows of nothing).
const PAD_MAX: u32 = 1000;

pub fn sheets(bytes: &[u8]) -> Result<Vec<Sheet>, String> {
    let mut book = calamine::open_workbook_auto_from_rs(std::io::Cursor::new(bytes))
        .map_err(|e| e.to_string())?;
    let names = book.sheet_names();
    if names.is_empty() {
        return Err("the workbook has no sheets".into());
    }
    let mut total = 0usize;
    let mut out = Vec::new();
    for name in names {
        let tsv = if total >= MAX_TSV {
            note(&crate::l10n::tr("(truncated)"))
        } else {
            match book.worksheet_range(&name) {
                Ok(range) => to_tsv(&range, MAX_TSV - total),
                // A chart sheet, or one calamine cannot read: say so in place.
                Err(e) => note(&e.to_string()),
            }
        };
        total += tsv.len();
        out.push(Sheet { name, tsv });
    }
    Ok(out)
}

fn note(text: &str) -> Vec<u8> {
    let mut v = field(text).into_bytes();
    v.push(b'\n');
    v
}

fn to_tsv(range: &calamine::Range<Data>, budget: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let (row0, col0) = range.start().unwrap_or((0, 0));
    let pad_cols = if col0 <= PAD_MAX { col0 as usize } else { 0 };
    if row0 <= PAD_MAX {
        out.extend(std::iter::repeat_n(b'\n', row0 as usize));
    }
    for row in range.rows() {
        if out.len() >= budget {
            out.extend(note(&crate::l10n::tr("(truncated)")));
            break;
        }
        let mut cells: Vec<String> = std::iter::repeat_n(String::new(), pad_cols).collect();
        cells.extend(row.iter().map(|c| field(&cell(c))));
        while cells.last().is_some_and(|c| c.is_empty()) {
            cells.pop();
        }
        let _ = writeln!(out, "{}", cells.join("\t"));
    }
    out
}

/// A cell as text: numbers as written (`3`, not `3.0`), dates as ISO 8601.
fn cell(c: &Data) -> String {
    match c {
        Data::Empty => String::new(),
        Data::Float(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", *f as i64),
        Data::DateTime(d) => {
            let (y, mo, da, h, mi, s, _) = d.to_ymd_hms_milli();
            if d.is_duration() {
                format!("{}:{mi:02}:{s:02}", (d.as_f64() * 24.0).floor() as i64)
            } else if (h, mi, s) == (0, 0, 0) {
                format!("{y:04}-{mo:02}-{da:02}")
            } else if d.as_f64() < 1.0 {
                format!("{h:02}:{mi:02}:{s:02}")
            } else {
                format!("{y:04}-{mo:02}-{da:02} {h:02}:{mi:02}:{s:02}")
            }
        }
        other => other.to_string(),
    }
}

/// A TSV field: quoted, with quotes doubled, when it holds a tab, a line break
/// or a quote of its own.
fn field(s: &str) -> String {
    if s.contains(['\t', '\n', '\r', '"']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::super::testkit::zip;
    use super::*;

    fn xlsx() -> Vec<u8> {
        let ct = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
            <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
            <Default Extension="xml" ContentType="application/xml"/>
            <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
            <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
            <Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
            </Types>"#;
        let rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
            <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
        let wb = r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
            <sheets><sheet name="Fruit" sheetId="1" r:id="rId1"/><sheet name="Other" sheetId="2" r:id="rId2"/></sheets></workbook>"#;
        let wb_rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
            <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
            <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/></Relationships>"#;
        let s1 = r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
            <row r="1"><c r="A1" t="inlineStr"><is><t>Name</t></is></c><c r="B1" t="inlineStr"><is><t>Qty</t></is></c></row>
            <row r="2"><c r="A2" t="inlineStr"><is><t>a	tab</t></is></c><c r="B2"><v>3</v></c></row>
            </sheetData></worksheet>"#;
        let s2 = r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
            <row r="1"><c r="A1"><v>1.5</v></c></row></sheetData></worksheet>"#;
        zip(&[
            ("[Content_Types].xml", ct),
            ("_rels/.rels", rels),
            ("xl/workbook.xml", wb),
            ("xl/_rels/workbook.xml.rels", wb_rels),
            ("xl/worksheets/sheet1.xml", s1),
            ("xl/worksheets/sheet2.xml", s2),
        ])
    }

    #[test]
    fn each_sheet_becomes_tab_separated_text() {
        let sheets = sheets(&xlsx()).unwrap();
        let names: Vec<&str> = sheets.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Fruit", "Other"]);
        assert_eq!(String::from_utf8_lossy(&sheets[0].tsv), "Name\tQty\n\"a\ttab\"\t3\n");
        assert_eq!(String::from_utf8_lossy(&sheets[1].tsv), "1.5\n");
    }

    #[test]
    fn fields_quote_only_when_they_must() {
        assert_eq!(field("plain"), "plain");
        assert_eq!(field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(field("two\nlines"), "\"two\nlines\"");
    }
}
