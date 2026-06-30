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
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
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
}

impl DetailsData {
    /// Whether a background size scan is still running.
    pub fn scanning(&self) -> bool {
        matches!(&self.kind, DetailsKind::Tally(t) if t.scanning)
    }
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

/// Draw the details body into `area`.
pub fn render(f: &mut Frame, area: Rect, data: &DetailsData, theme: &Theme) {
    if area.width < 4 || area.height < 2 {
        return;
    }
    let body = Rect { x: area.x + 1, width: area.width.saturating_sub(2), ..area };
    match &data.kind {
        DetailsKind::Empty => {
            let dim = Style::default().fg(theme.panel_border).bg(theme.panel_bg);
            let msg = "No file under the other panel's cursor";
            let y = body.y + body.height / 2;
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(msg, dim)))
                    .alignment(ratatui::layout::Alignment::Center),
                Rect { y, height: 1, ..body },
            );
        }
        DetailsKind::File(fi) => render_file(f, body, fi, theme),
        DetailsKind::Tally(t) => render_tally(f, body, t, theme),
    }
}

fn render_file(f: &mut Frame, area: Rect, fi: &FileInfo, theme: &Theme) {
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
        ("Type", kind.to_string()),
    ];
    if let Some(t) = &fi.symlink_target {
        rows.push(("Links to", t.clone()));
    }
    rows.push(("Size", format!("{}   ({} bytes)", human_size(fi.size), group_digits(fi.size))));
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
}

fn render_tally(f: &mut Frame, area: Rect, t: &Tally, theme: &Theme) {
    let label = Style::default().fg(theme.header_fg).bg(theme.panel_bg);
    let value = Style::default().fg(theme.panel_fg).bg(theme.panel_bg);
    let rows: Vec<(&str, String)> = vec![
        ("", t.label.clone()),
        ("", String::new()),
        ("Total size", format!("{}   ({} bytes)", human_size(t.total), group_digits(t.total))),
        ("Files", group_digits(t.files)),
        ("Directories", group_digits(t.dirs)),
    ];
    draw_rows(f, area, &rows, label, value);
    if t.scanning {
        let dim = Style::default()
            .fg(theme.header_fg)
            .bg(theme.panel_bg)
            .add_modifier(Modifier::ITALIC);
        let y = area.y + rows.len() as u16 + 1;
        if y < area.y + area.height {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled("calculating…", dim))),
                Rect { y, height: 1, ..area },
            );
        }
    }
}

/// Draw `label: value` rows, right-aligning the labels into a column.
fn draw_rows(f: &mut Frame, area: Rect, rows: &[(&str, String)], label: Style, value: Style) {
    let lw = rows.iter().map(|(l, _)| l.chars().count()).max().unwrap_or(0);
    for (i, (l, v)) in rows.iter().enumerate() {
        let y = area.y + 1 + i as u16;
        if y >= area.y + area.height {
            break;
        }
        let spans = if l.is_empty() {
            vec![Span::styled(v.clone(), value.add_modifier(Modifier::BOLD))]
        } else {
            vec![
                Span::styled(format!("{:>lw$} : ", l), label),
                Span::styled(v.clone(), value),
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
