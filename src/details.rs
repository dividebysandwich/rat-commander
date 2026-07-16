//! The "details" panel view: render-ready data describing whatever the *other*
//! panel currently points at, plus its renderer.
//!
//! The data is built by the app (where uid/gid name lookups live) so this module
//! only formats and draws it. A directory's (or a selection's) recursive size is
//! filled in by a background task and refreshed in place via
//! [`AppEvent::DetailsTally`](crate::app::event::AppEvent::DetailsTally).

use crate::ui::theme::Theme;
use crate::util::bytes::{format_time, human_size};
use crate::vfs::VfsKind;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use crate::syntax::ColorRun;
use std::time::SystemTime;

/// Per-panel details state: what to show and the background scan bookkeeping.
#[derive(Default)]
pub struct DetailsData {
    /// Signature of the source (cwd / cursor / selection) this was built for, so
    /// the app can tell when it must be recomputed.
    pub(crate) key: String,
    pub kind: DetailsKind,
    /// Background size-scan generation; tally events with a stale generation are
    /// ignored.
    pub(crate) generation: u64,
    /// Cancel handle for the running scan, if any.
    pub(crate) cancel: Option<crate::ops::CancelToken>,
    /// A preview of the item under the other panel's cursor, loaded in the
    /// background (shares `generation` with the size scan).
    pub preview: Preview,
}

/// A background-loaded preview of the item the Details view describes, drawn
/// beneath the metadata.
#[derive(Default, Debug, Clone)]
pub enum Preview {
    /// Nothing to preview (empty, a multi-item selection, or unsupported).
    #[default]
    None,
    /// A background load is in flight.
    Loading,
    /// Syntax-highlighted head of a text file.
    Text(Vec<PreviewLine>),
    /// A decoded thumbnail of an image file.
    Image(PreviewImage),
    /// An archive's top-level entry names.
    Archive(Vec<String>),
    /// A shallow tree of a directory's contents.
    Tree(Vec<PreviewTreeLine>),
}

/// One highlighted source line: its text and per-run foreground colours.
#[derive(Debug, Clone)]
pub struct PreviewLine {
    pub text: String,
    pub runs: Vec<crate::syntax::ColorRun>,
}

/// A decoded thumbnail plus a cheap content signature for the graphics cache and
/// a short EXIF summary (`(label, value)` pairs; empty when the image has none).
#[derive(Debug, Clone)]
pub struct PreviewImage {
    pub img: image::RgbaImage,
    pub sig: u64,
    pub exif: Vec<(String, String)>,
}

/// One row of a directory-tree preview.
#[derive(Debug, Clone)]
pub struct PreviewTreeLine {
    pub depth: u16,
    pub name: String,
    pub is_dir: bool,
}

#[derive(Default)]
pub enum DetailsKind {
    /// Nothing to show (the other panel is on `..`, empty, or also in Details).
    #[default]
    Empty,
    /// Stats for a single file under the other panel's cursor.
    File(FileInfo),
    /// A running size/count tally for a directory or a multi-item selection.
    Tally(Tally),
}

/// Snapshot of a single file's metadata, with names already resolved.
pub struct FileInfo {
    pub name: String,
    pub dir: String,
    pub kind: VfsKind,
    pub size: u64,
    pub mode: Option<u32>,
    pub owner: String,
    pub group: String,
    pub mtime: Option<SystemTime>,
    pub atime: Option<SystemTime>,
    pub ctime: Option<SystemTime>,
    pub inode: Option<u64>,
    pub symlink_target: Option<String>,
}

/// A recursively-accumulated size and file/dir count.
pub struct Tally {
    /// Heading, e.g. a directory name or `"5 items selected"`.
    pub label: String,
    pub total: u64,
    pub files: u64,
    pub dirs: u64,
    /// True while the background scan is still walking the tree.
    pub scanning: bool,
}

/// Draw the details body into `area`. Returns the cell rect where a pixel-image
/// preview should be drawn by the caller (only when the preview is an image and
/// `graphics` is available); `None` otherwise (all other previews draw inline).
pub fn render(
    f: &mut Frame,
    area: Rect,
    data: &DetailsData,
    theme: &Theme,
    graphics: bool,
) -> Option<Rect> {
    if area.width < 4 || area.height < 2 {
        return None;
    }
    let body = Rect { x: area.x + 1, width: area.width.saturating_sub(2), ..area };
    let rows = match &data.kind {
        DetailsKind::Empty => {
            let dim = Style::default().fg(theme.panel_border).bg(theme.panel_bg);
            let msg = crate::l10n::trd("No file under the other panel's cursor");
            let y = body.y + body.height / 2;
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(msg, dim)))
                    .alignment(ratatui::layout::Alignment::Center),
                Rect { y, height: 1, ..body },
            );
            return None;
        }
        DetailsKind::File(fi) => render_file(f, body, fi, theme),
        DetailsKind::Tally(t) => render_tally(f, body, t, theme),
    };
    // Draw the preview beneath the metadata, separated by a blank row.
    let top = body.y + 1 + rows as u16 + 1;
    if top + 1 >= body.y + body.height {
        return None;
    }
    let preview_area = Rect { y: top, height: body.y + body.height - top, ..body };
    render_preview(f, preview_area, &data.preview, theme, graphics)
}

/// Draw `label: value` metadata rows, returning how many interior rows were used
/// so the caller can place the preview below them.
fn render_file(f: &mut Frame, area: Rect, fi: &FileInfo, theme: &Theme) -> usize {
    let label = Style::default().fg(theme.header_fg).bg(theme.panel_bg);
    let value = Style::default().fg(theme.panel_fg).bg(theme.panel_bg);
    let kind = match fi.kind {
        VfsKind::Dir => "Directory",
        VfsKind::Symlink => "Symbolic link",
        VfsKind::Other => "Special",
        VfsKind::File => "File",
    };
    let mut rows: Vec<(&str, String)> = vec![
        ("Name", fi.name.clone()),
        ("In", fi.dir.clone()),
        ("Type", crate::l10n::trd(kind)),
    ];
    if let Some(t) = &fi.symlink_target {
        rows.push(("Links to", t.clone()));
    }
    rows.push((
        "Size",
        format!("{}   ({} {})", human_size(fi.size), group_digits(fi.size), crate::l10n::trd("bytes")),
    ));
    if let Some(m) = fi.mode {
        rows.push(("Access", format!("{}  ({:04o})", rwx(m), m & 0o7777)));
    }
    rows.push(("Owner", fi.owner.clone()));
    rows.push(("Group", fi.group.clone()));
    if let Some(t) = fi.mtime {
        rows.push(("Modified", format_time(t)));
    }
    if let Some(t) = fi.atime {
        rows.push(("Accessed", format_time(t)));
    }
    if let Some(t) = fi.ctime {
        rows.push(("Changed", format_time(t)));
    }
    if let Some(i) = fi.inode {
        rows.push(("Inode", i.to_string()));
    }
    draw_rows(f, area, &rows, label, value);
    rows.len()
}

fn render_tally(f: &mut Frame, area: Rect, t: &Tally, theme: &Theme) -> usize {
    let label = Style::default().fg(theme.header_fg).bg(theme.panel_bg);
    let value = Style::default().fg(theme.panel_fg).bg(theme.panel_bg);
    let rows: Vec<(&str, String)> = vec![
        ("", t.label.clone()),
        ("", String::new()),
        ("Total size", format!("{}   ({} {})", human_size(t.total), group_digits(t.total), crate::l10n::trd("bytes"))),
        ("Files", group_digits(t.files)),
        ("Directories", group_digits(t.dirs)),
    ];
    draw_rows(f, area, &rows, label, value);
    let mut used = rows.len();
    if t.scanning {
        let dim = Style::default()
            .fg(theme.header_fg)
            .bg(theme.panel_bg)
            .add_modifier(Modifier::ITALIC);
        let y = area.y + rows.len() as u16 + 1;
        if y < area.y + area.height {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(crate::l10n::trd("calculating…"), dim))),
                Rect { y, height: 1, ..area },
            );
        }
        used += 1;
    }
    used
}

/// Draw the preview beneath the metadata. Returns the pixel-image target rect
/// when the preview is an image and `graphics` is available (drawn by the
/// caller); otherwise draws the preview inline and returns `None`.
fn render_preview(
    f: &mut Frame,
    area: Rect,
    preview: &Preview,
    theme: &Theme,
    graphics: bool,
) -> Option<Rect> {
    if matches!(preview, Preview::None) {
        return None;
    }
    // A dim `─ Preview ─────` rule separates it from the metadata above.
    let rule = Style::default().fg(theme.panel_border).bg(theme.panel_bg);
    let mut header = format!("─ {} ", crate::l10n::trd("Preview"));
    while header.chars().count() < area.width as usize {
        header.push('─');
    }
    let header: String = header.chars().take(area.width as usize).collect();
    f.render_widget(Paragraph::new(Line::from(Span::styled(header, rule))), Rect { height: 1, ..area });

    let content = Rect { y: area.y + 1, height: area.height.saturating_sub(1), ..area };
    if content.height == 0 {
        return None;
    }
    let bg = Style::default().bg(theme.panel_bg);
    match preview {
        Preview::None => None,
        Preview::Loading => {
            let dim = rule.add_modifier(Modifier::ITALIC);
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(crate::l10n::trd("loading preview…"), dim))),
                Rect { height: 1, ..content },
            );
            None
        }
        Preview::Text(lines) => {
            let out: Vec<Line> = lines
                .iter()
                .take(content.height as usize)
                .map(|pl| highlight_line(&pl.text, &pl.runs, theme.panel_fg, theme.panel_bg))
                .collect();
            f.render_widget(Paragraph::new(out).style(bg), content);
            None
        }
        Preview::Archive(names) => {
            render_preview_names(f, content, names, theme.archive_fg, theme);
            None
        }
        Preview::Tree(rows) => {
            render_preview_tree(f, content, rows, theme);
            None
        }
        Preview::Image(pi) => {
            // An EXIF summary (when present) sits above the thumbnail; the image
            // gets the remaining rows.
            let mut img_area = content;
            if !pi.exif.is_empty() && content.height >= 8 {
                let n = (pi.exif.len() as u16).min(5);
                render_exif(f, Rect { height: n, ..content }, &pi.exif, theme);
                img_area = Rect {
                    y: content.y + n + 1,
                    height: content.height.saturating_sub(n + 1),
                    ..content
                };
            }
            if img_area.height == 0 {
                None
            } else if graphics {
                Some(img_area) // the caller draws the pixel image here
            } else {
                crate::util::img::render_halfblocks(f, img_area, &pi.img, theme.panel_bg);
                None
            }
        }
    }
}

/// Draw an EXIF `label : value` summary block. The labels are translated here
/// (values are data — camera names, exposures — and left as-is).
fn render_exif(f: &mut Frame, area: Rect, exif: &[(String, String)], theme: &Theme) {
    let label = Style::default().fg(theme.header_fg).bg(theme.panel_bg);
    let value = Style::default().fg(theme.panel_fg).bg(theme.panel_bg);
    let tr: Vec<(String, &String)> = exif.iter().map(|(l, v)| (crate::l10n::trd(l), v)).collect();
    let lw = tr.iter().map(|(l, _)| l.chars().count()).max().unwrap_or(0);
    let lines: Vec<Line> = tr
        .iter()
        .take(area.height as usize)
        .map(|(l, v)| {
            Line::from(vec![
                Span::styled(format!(" {l:>lw$} : "), label),
                Span::styled((*v).clone(), value),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines).style(Style::default().bg(theme.panel_bg)), area);
}

/// Build a styled line from syntax-highlight color runs over `text` (runs are
/// character-aligned to `text`; any tail beyond the runs uses `base`).
fn highlight_line(text: &str, runs: &[ColorRun], base: Color, bg: Color) -> Line<'static> {
    if runs.is_empty() {
        return Line::from(Span::styled(text.to_string(), Style::default().fg(base).bg(bg)));
    }
    let chars: Vec<char> = text.chars().collect();
    let mut spans: Vec<Span> = Vec::with_capacity(runs.len() + 1);
    let mut i = 0usize;
    for (count, color) in runs {
        let end = (i + *count as usize).min(chars.len());
        if i >= end {
            continue;
        }
        spans.push(Span::styled(chars[i..end].iter().collect::<String>(), Style::default().fg(*color).bg(bg)));
        i = end;
    }
    if i < chars.len() {
        spans.push(Span::styled(chars[i..].iter().collect::<String>(), Style::default().fg(base).bg(bg)));
    }
    Line::from(spans)
}

/// Render a capped list of names (archive entries), with a `… N more` tail line
/// when there are more than fit.
fn render_preview_names(f: &mut Frame, area: Rect, names: &[String], fg: Color, theme: &Theme) {
    let rows = area.height as usize;
    let style = Style::default().fg(fg).bg(theme.panel_bg);
    let mut lines: Vec<Line> = Vec::with_capacity(rows);
    let shown = if names.len() > rows { rows.saturating_sub(1) } else { names.len() };
    for n in names.iter().take(shown) {
        lines.push(Line::from(Span::styled(format!(" {n}"), style)));
    }
    if names.len() > shown {
        let dim = Style::default().fg(theme.panel_border).bg(theme.panel_bg).add_modifier(Modifier::ITALIC);
        lines.push(Line::from(Span::styled(format!(" … {} more", names.len() - shown), dim)));
    }
    f.render_widget(Paragraph::new(lines).style(Style::default().bg(theme.panel_bg)), area);
}

/// Render an indented directory-tree preview.
fn render_preview_tree(f: &mut Frame, area: Rect, rows: &[PreviewTreeLine], theme: &Theme) {
    let n = area.height as usize;
    let mut lines: Vec<Line> = Vec::with_capacity(n);
    let shown = if rows.len() > n { n.saturating_sub(1) } else { rows.len() };
    for r in rows.iter().take(shown) {
        let indent = "  ".repeat(r.depth as usize);
        let (marker, fg) =
            if r.is_dir { ('/', theme.dir_fg) } else { (' ', theme.panel_fg) };
        let text = format!(" {indent}{marker}{}", r.name);
        lines.push(Line::from(Span::styled(text, Style::default().fg(fg).bg(theme.panel_bg))));
    }
    if rows.len() > shown {
        let dim = Style::default().fg(theme.panel_border).bg(theme.panel_bg).add_modifier(Modifier::ITALIC);
        lines.push(Line::from(Span::styled(format!(" … {} more", rows.len() - shown), dim)));
    }
    f.render_widget(Paragraph::new(lines).style(Style::default().bg(theme.panel_bg)), area);
}

/// Draw `label: value` rows, right-aligning the labels into a column. Labels are
/// translated here (and the column width is measured on the translated text).
fn draw_rows(f: &mut Frame, area: Rect, rows: &[(&str, String)], label: Style, value: Style) {
    let tr_rows: Vec<(String, &String)> = rows
        .iter()
        .map(|(l, v)| (if l.is_empty() { String::new() } else { crate::l10n::trd(l) }, v))
        .collect();
    let lw = tr_rows.iter().map(|(l, _)| l.chars().count()).max().unwrap_or(0);
    for (i, (l, v)) in tr_rows.iter().enumerate() {
        let y = area.y + 1 + i as u16;
        if y >= area.y + area.height {
            break;
        }
        let spans = if l.is_empty() {
            vec![Span::styled((*v).clone(), value.add_modifier(Modifier::BOLD))]
        } else {
            vec![
                Span::styled(format!("{:>lw$} : ", l), label),
                Span::styled((*v).clone(), value),
            ]
        };
        f.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect { x: area.x, y, width: area.width, height: 1 },
        );
    }
}

/// `rwxr-xr-x`-style permission string from the low 9 mode bits.
fn rwx(mode: u32) -> String {
    let bit = |m: u32, c: char| if mode & m != 0 { c } else { '-' };
    let mut s = String::with_capacity(9);
    for (r, w, x) in [(0o400, 0o200, 0o100), (0o040, 0o020, 0o010), (0o004, 0o002, 0o001)] {
        s.push(bit(r, 'r'));
        s.push(bit(w, 'w'));
        s.push(bit(x, 'x'));
    }
    s
}

/// Group a number's digits in threes: `1234567` → `"1,234,567"`.
fn group_digits(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    let bytes = s.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn file_info(name: &str) -> FileInfo {
        FileInfo {
            name: name.to_string(),
            dir: "/x".to_string(),
            kind: VfsKind::File,
            size: 10,
            mode: Some(0o644),
            owner: "u".to_string(),
            group: "g".to_string(),
            mtime: None,
            atime: None,
            ctime: None,
            inode: None,
            symlink_target: None,
        }
    }

    fn screen(data: &DetailsData, graphics: bool) -> (String, Option<Rect>) {
        let theme = Theme::mc();
        let mut t = Terminal::new(TestBackend::new(44, 24)).unwrap();
        let mut rect = None;
        t.draw(|f| rect = render(f, f.area(), data, &theme, graphics)).unwrap();
        let b = t.backend().buffer();
        let text: String = (0..b.area.height)
            .flat_map(|y| (0..b.area.width).map(move |x| (x, y)))
            .map(|(x, y)| b[(x, y)].symbol().to_string())
            .collect();
        (text, rect)
    }

    #[test]
    fn renders_text_preview_below_metadata() {
        let mut data = DetailsData { kind: DetailsKind::File(file_info("code.rs")), ..Default::default() };
        data.preview = Preview::Text(vec![
            PreviewLine { text: "fn main() {}".into(), runs: vec![(2, Color::Red), (10, Color::Blue)] },
            PreviewLine { text: "// a comment".into(), runs: vec![] },
        ]);
        let (text, rect) = screen(&data, false);
        assert!(text.contains("code.rs"), "metadata still shown");
        assert!(text.contains("Preview"), "preview header shown");
        assert!(text.contains("fn main() {}"), "highlighted head shown");
        assert!(text.contains("// a comment"));
        assert!(rect.is_none(), "text preview draws inline");
    }

    #[test]
    fn image_preview_reserves_rect_with_graphics_and_draws_blocks_without() {
        let img = image::RgbaImage::from_pixel(6, 6, image::Rgba([200, 100, 40, 255]));
        let mut data = DetailsData { kind: DetailsKind::File(file_info("pic.png")), ..Default::default() };
        data.preview = Preview::Image(PreviewImage {
            img: img.clone(),
            sig: 7,
            exif: vec![("Camera".into(), "Canon EOS 5D".into())],
        });

        // With graphics, render reserves a rect (the caller draws the pixels) and
        // paints no half-blocks itself. The EXIF summary is shown.
        let (text, rect) = screen(&data, true);
        assert!(rect.is_some(), "pixel preview reserves a target rect");
        assert!(!text.contains('▀'), "no half-blocks when pixel graphics handle it");
        assert!(text.contains("Camera") && text.contains("Canon EOS 5D"), "exif summary shown: {text:?}");

        // Without graphics, it draws half-block cell art.
        data.preview = Preview::Image(PreviewImage { img, sig: 7, exif: vec![] });
        let (text, rect) = screen(&data, false);
        assert!(rect.is_none());
        assert!(text.contains('▀'), "ascii fallback draws half-blocks");
    }

    #[test]
    fn renders_tree_and_archive_previews() {
        let mut data = DetailsData { kind: DetailsKind::File(file_info("dir")), ..Default::default() };
        data.preview = Preview::Tree(vec![
            PreviewTreeLine { depth: 0, name: "src".into(), is_dir: true },
            PreviewTreeLine { depth: 1, name: "main.rs".into(), is_dir: false },
        ]);
        let (text, _) = screen(&data, false);
        assert!(text.contains("/src") && text.contains("main.rs"), "tree preview: {text:?}");

        data.preview = Preview::Archive(vec!["readme.txt".into(), "sub/".into()]);
        let (text, _) = screen(&data, false);
        assert!(text.contains("readme.txt") && text.contains("sub/"), "archive preview: {text:?}");
    }

    #[test]
    fn rwx_and_grouping() {
        assert_eq!(rwx(0o644), "rw-r--r--");
        assert_eq!(rwx(0o755), "rwxr-xr-x");
        assert_eq!(rwx(0o000), "---------");
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(12), "12");
        assert_eq!(group_digits(1234), "1,234");
        assert_eq!(group_digits(12_856_320), "12,856,320");
    }
}
