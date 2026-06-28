//! Rendering of the [`DiskView`] treemap.

use super::{human_gb, DiskEntry, DiskView};
use crate::ui::theme::Theme;
use crate::util::text::{ellipsize, pad_right};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

pub fn render(f: &mut Frame, area: Rect, dv: &mut DiskView, theme: &Theme) {
    let title = format!(
        " Disk Explorer — {}  ({}) ",
        dv.cwd.display(),
        human_gb(dv.total())
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(theme.panel_border_active).bg(theme.panel_bg))
        .title(Span::styled(
            crate::util::text::ellipsize(&title, area.width.saturating_sub(2) as usize),
            Style::default()
                .fg(theme.panel_border_active)
                .bg(theme.panel_bg)
                .add_modifier(Modifier::BOLD),
        ))
        .style(theme.panel_base());
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 2 || inner.width < 4 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // selected-box readout
            Constraint::Min(1),    // treemap
            Constraint::Length(1), // shortcut bar
        ])
        .split(inner);
    let header = rows[0];
    let body = rows[1];
    render_footer(f, rows[2], theme);

    dv.rects.clear();
    if dv.scanning {
        render_header(f, header, None, 0, theme);
        render_scanning(f, body, dv.scan_done, dv.scan_total, theme);
        return;
    }
    if dv.entries.is_empty() {
        render_header(f, header, None, 0, theme);
        center_text(f, body, "(no subdirectories)", theme);
        return;
    }
    let selected = dv.entries.get(dv.selected.min(dv.entries.len() - 1));
    render_header(f, header, selected, dv.total(), theme);

    let rects = treemap(&dv.entries, body);
    dv.rects = rects.clone();
    if dv.selected >= dv.entries.len() {
        dv.selected = dv.entries.len() - 1;
    }
    let n = dv.entries.len();
    for (i, (entry, rect)) in dv.entries.iter().zip(rects.iter()).enumerate() {
        draw_box(f, *rect, entry, i == dv.selected, i, n, theme);
    }
}

/// Show the selected box's name and size at the top, so the selection is always
/// legible even when its box is too small to render a label.
fn render_header(f: &mut Frame, area: Rect, selected: Option<&DiskEntry>, total: u64, theme: &Theme) {
    let spans = match selected {
        Some(e) => {
            let pct = if total > 0 {
                100.0 * e.size as f32 / total as f32
            } else {
                0.0
            };
            vec![
                Span::styled(
                    " ▶ ",
                    Style::default().fg(theme.panel_border_active).bg(theme.panel_bg),
                ),
                Span::styled(
                    ellipsize(&e.name, area.width.saturating_sub(28) as usize),
                    Style::default()
                        .fg(theme.cursor_fg)
                        .bg(theme.panel_bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("   {}   {:.0}% of total", human_gb(e.size), pct),
                    Style::default().fg(theme.panel_fg).bg(theme.panel_bg),
                ),
            ]
        }
        None => vec![Span::styled(
            " (nothing selected)",
            Style::default().fg(theme.panel_fg).bg(theme.panel_bg),
        )],
    };
    f.render_widget(Paragraph::new(Line::from(spans)).style(theme.panel_base()), area);
}

fn render_footer(f: &mut Frame, area: Rect, theme: &Theme) {
    let hint = "←↑↓→ move   Enter: open   g / Ctrl-Enter: go to dir   Backspace: up   Esc: close";
    // Draw as a highlighted bar (like the F-key row) so it's clearly visible.
    let line = pad_right(&format!(" {hint}"), area.width as usize);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(line, theme.fkey_label)))
            .style(theme.fkey_label),
        area,
    );
}

/// Show scanning progress: a centered label and a horizontal progress bar. The
/// bar is determinate once the subdirectory count is known, indeterminate (just
/// a label) before that.
fn render_scanning(f: &mut Frame, area: Rect, done: usize, total: usize, theme: &Theme) {
    let label = if total > 0 {
        format!("Scanning… {done} / {total} directories")
    } else {
        "Scanning… (enumerating directories)".to_string()
    };
    let mid = area.y + area.height / 2;
    f.render_widget(
        Paragraph::new(Line::from(label))
            .alignment(Alignment::Center)
            .style(theme.panel_base()),
        Rect { y: mid.saturating_sub(1), height: 1, ..area },
    );

    if total == 0 {
        return;
    }
    // A centered bar ~60% of the body width.
    let bar_w = (area.width as usize * 3 / 5).clamp(10, area.width as usize);
    let bar_x = area.x + (area.width as usize - bar_w) as u16 / 2;
    let ratio = (done as f32 / total as f32).clamp(0.0, 1.0);
    let filled = (ratio * bar_w as f32).round() as usize;
    let mut spans = Vec::with_capacity(bar_w + 1);
    for i in 0..bar_w {
        // Use the same animated pulse fill as the file-copy progress bars.
        let (ch, color) = if i < filled {
            ('█', crate::ui::dialog::pulse_fill(theme, theme.panel_border_active, i, bar_w))
        } else {
            ('░', theme.panel_border)
        };
        spans.push(Span::styled(ch.to_string(), Style::default().fg(color).bg(theme.panel_bg)));
    }
    spans.push(Span::styled(
        format!(" {:.0}%", ratio * 100.0),
        Style::default().fg(theme.panel_fg).bg(theme.panel_bg),
    ));
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(theme.panel_base()),
        Rect { x: bar_x, y: mid + 1, height: 1, width: area.width - (bar_x - area.x) },
    );
}

fn center_text(f: &mut Frame, area: Rect, text: &str, theme: &Theme) {
    let row = Rect {
        y: area.y + area.height / 2,
        height: 1,
        ..area
    };
    f.render_widget(
        Paragraph::new(Line::from(text.to_string()))
            .alignment(Alignment::Center)
            .style(theme.panel_base()),
        row,
    );
}

fn draw_box(f: &mut Frame, rect: Rect, entry: &DiskEntry, selected: bool, idx: usize, n: usize, theme: &Theme) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let color = if selected {
        theme.panel_border_active
    } else if theme.truecolor {
        theme.gradient_at(idx, n.max(1))
    } else {
        theme.panel_border
    };

    // Tiny boxes: just a colored block (no room for a border or labels).
    if rect.width < 4 || rect.height < 3 {
        let style = Style::default().fg(color).bg(theme.panel_bg);
        let buf = f.buffer_mut();
        for yy in rect.y..rect.y + rect.height {
            buf.set_string(rect.x, yy, "█".repeat(rect.width as usize), style);
        }
        return;
    }

    let mut border = Style::default().fg(color).bg(theme.panel_bg);
    if selected {
        border = border.add_modifier(Modifier::BOLD);
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border)
        .style(theme.panel_base());
    let bi = block.inner(rect);
    f.render_widget(block, rect);
    if bi.width == 0 || bi.height == 0 {
        return;
    }

    let name_style = if selected {
        Style::default().fg(theme.cursor_fg).bg(theme.panel_bg).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.panel_fg).bg(theme.panel_bg).add_modifier(Modifier::BOLD)
    };
    let size_style = Style::default().fg(color).bg(theme.panel_bg);

    let name = ellipsize(&entry.name, bi.width as usize);
    let size = human_gb(entry.size);
    if bi.height >= 2 {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(name, name_style)))
                .alignment(Alignment::Center),
            Rect { height: 1, ..bi },
        );
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(ellipsize(&size, bi.width as usize), size_style)))
                .alignment(Alignment::Center),
            Rect { y: bi.y + 1, height: 1, ..bi },
        );
        // Big enough: list the largest files (path relative to this box + size).
        if bi.height >= 5 && bi.width >= 16 && !entry.files.is_empty() {
            let list = Rect {
                y: bi.y + 3,
                height: bi.height - 3,
                ..bi
            };
            draw_file_list(f, list, &entry.files, color, theme);
        }
    } else {
        // One interior row: show the name only.
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(name, name_style)))
                .alignment(Alignment::Center),
            bi,
        );
    }
}

/// List the biggest files inside a box: each row is `relative/path … SIZE`,
/// the path left-aligned (dim) and the size right-aligned in the box color.
fn draw_file_list(
    f: &mut Frame,
    area: Rect,
    files: &[super::FileEntry],
    color: ratatui::style::Color,
    theme: &Theme,
) {
    let w = area.width as usize;
    let rows = area.height as usize;
    for (k, file) in files.iter().take(rows).enumerate() {
        // In truecolor, fade each successive row's background a little darker so
        // the list reads as a gradient down the box.
        let bg = if theme.truecolor {
            darken(theme.panel_bg, (k as f32 * 0.08).min(0.6))
        } else {
            theme.panel_bg
        };
        let path_style = Style::default().fg(theme.panel_fg).bg(bg);
        let size_style = Style::default().fg(color).bg(bg);

        let size = human_gb(file.size);
        // Reserve "<space>SIZE" on the right; the path fills the rest.
        let path_w = w.saturating_sub(size.chars().count() + 1).max(1);
        let path = ellipsize(&file.rel, path_w);
        let line = Line::from(vec![
            Span::styled(pad_right(&path, path_w), path_style),
            Span::styled(format!(" {size}"), size_style),
        ]);
        f.render_widget(
            Paragraph::new(line).style(Style::default().bg(bg)),
            Rect { y: area.y + k as u16, height: 1, ..area },
        );
    }
}

/// Scale an RGB color toward black by `t` (0 = unchanged, 1 = black).
fn darken(c: ratatui::style::Color, t: f32) -> ratatui::style::Color {
    use ratatui::style::Color;
    if let Color::Rgb(r, g, b) = c {
        let f = |x: u8| (x as f32 * (1.0 - t)).round().clamp(0.0, 255.0) as u8;
        Color::Rgb(f(r), f(g), f(b))
    } else {
        c
    }
}

// ---------------------------------------------------------------------------
// Squarified treemap (Bruls, Huizing & van Wijk)
// ---------------------------------------------------------------------------

struct FRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// Lay out `entries` (already sorted largest-first) as a treemap filling `area`,
/// returning one integer rect per entry in the same order.
fn treemap(entries: &[DiskEntry], area: Rect) -> Vec<Rect> {
    let n = entries.len();
    if n == 0 || area.width == 0 || area.height == 0 {
        return vec![Rect { width: 0, height: 0, ..area }; n];
    }
    let total: u64 = entries.iter().map(|e| e.size).sum();
    // A floor so even small/empty directories stay visible and navigable.
    let base = (total / n as u64 / 12).max(1);
    let weights: Vec<f64> = entries.iter().map(|e| (e.size + base) as f64).collect();
    let wsum: f64 = weights.iter().sum();
    let total_area = area.width as f64 * area.height as f64;
    let areas: Vec<f64> = weights.iter().map(|w| w / wsum * total_area).collect();

    let frects = squarify(&areas, area.x as f64, area.y as f64, area.width as f64, area.height as f64);
    let x_max = (area.x + area.width) as f64;
    let y_max = (area.y + area.height) as f64;
    frects
        .iter()
        .map(|r| {
            let x0 = r.x.round().clamp(area.x as f64, x_max);
            let y0 = r.y.round().clamp(area.y as f64, y_max);
            let x1 = (r.x + r.w).round().clamp(x0, x_max);
            let y1 = (r.y + r.h).round().clamp(y0, y_max);
            Rect {
                x: x0 as u16,
                y: y0 as u16,
                width: (x1 - x0) as u16,
                height: (y1 - y0) as u16,
            }
        })
        .collect()
}

fn squarify(areas: &[f64], x: f64, y: f64, w: f64, h: f64) -> Vec<FRect> {
    let mut out: Vec<FRect> = Vec::with_capacity(areas.len());
    let mut rect = FRect { x, y, w, h };
    let mut row: Vec<f64> = Vec::new();
    let mut i = 0;
    while i < areas.len() {
        let length = rect.w.min(rect.h);
        if length <= 0.0 {
            // No space left; emit zero rects for the remainder.
            for _ in i..areas.len() {
                out.push(FRect { x: rect.x, y: rect.y, w: 0.0, h: 0.0 });
            }
            return out;
        }
        let a = areas[i];
        row.push(a);
        let with = worst(&row, length);
        row.pop();
        let without = if row.is_empty() { f64::MAX } else { worst(&row, length) };
        if row.is_empty() || without >= with {
            row.push(a);
            i += 1;
        } else {
            layout_row(&row, &mut rect, &mut out);
            row.clear();
        }
    }
    if !row.is_empty() {
        layout_row(&row, &mut rect, &mut out);
    }
    out
}

/// Worst (largest) aspect ratio in a row laid along side `length`.
fn worst(row: &[f64], length: f64) -> f64 {
    let sum: f64 = row.iter().sum();
    if sum <= 0.0 || length <= 0.0 {
        return f64::MAX;
    }
    let max = row.iter().cloned().fold(f64::MIN, f64::max);
    let min = row.iter().cloned().fold(f64::MAX, f64::min);
    let l2 = length * length;
    let s2 = sum * sum;
    f64::max(l2 * max / s2, s2 / (l2 * min))
}

fn layout_row(row: &[f64], rect: &mut FRect, out: &mut Vec<FRect>) {
    let sum: f64 = row.iter().sum();
    if sum <= 0.0 {
        for _ in row {
            out.push(FRect { x: rect.x, y: rect.y, w: 0.0, h: 0.0 });
        }
        return;
    }
    if rect.w >= rect.h {
        // Lay the row as a column down the left edge.
        let col_w = sum / rect.h;
        let mut yy = rect.y;
        for &a in row {
            let cell_h = a / col_w;
            out.push(FRect { x: rect.x, y: yy, w: col_w, h: cell_h });
            yy += cell_h;
        }
        rect.x += col_w;
        rect.w -= col_w;
    } else {
        // Lay the row across the top edge.
        let row_h = sum / rect.w;
        let mut xx = rect.x;
        for &a in row {
            let cell_w = a / row_h;
            out.push(FRect { x: xx, y: rect.y, w: cell_w, h: row_h });
            xx += cell_w;
        }
        rect.y += row_h;
        rect.h -= row_h;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(name: &str, size: u64) -> DiskEntry {
        DiskEntry { name: name.into(), size, files: vec![] }
    }

    #[test]
    fn scanning_shows_a_progress_bar() {
        use crate::disk::DiskView;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut dv = DiskView::new(std::path::PathBuf::from("/tmp"));
        dv.scanning = true;
        dv.scan_done = 3;
        dv.scan_total = 12;
        let theme = crate::ui::theme::Theme::mc();
        let mut t = Terminal::new(TestBackend::new(80, 20)).unwrap();
        t.draw(|f| render(f, f.area(), &mut dv, &theme)).unwrap();
        let b = t.backend().buffer();
        let mut s = String::new();
        for y in 0..b.area.height {
            for x in 0..b.area.width {
                s.push_str(b[(x, y)].symbol());
            }
        }
        assert!(s.contains("3 / 12 directories"), "progress label");
        assert!(s.contains('█') && s.contains('░'), "determinate bar drawn");
        assert!(s.contains("25%"), "percentage shown");
    }

    #[test]
    fn big_box_lists_its_largest_files() {
        use crate::disk::{DiskView, FileEntry};
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut dv = DiskView::new(std::path::PathBuf::from("/tmp"));
        dv.scanning = false;
        // A single entry fills the whole treemap → its box is large.
        dv.entries = vec![DiskEntry {
            name: "project".into(),
            size: 9_000_000,
            files: vec![
                FileEntry { rel: "target/huge.bin".into(), size: 5_000_000 },
                FileEntry { rel: "assets/movie.mp4".into(), size: 3_000_000 },
            ],
        }];
        let theme = crate::ui::theme::Theme::mc();
        let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
        t.draw(|f| render(f, f.area(), &mut dv, &theme)).unwrap();
        let b = t.backend().buffer();
        let mut s = String::new();
        for y in 0..b.area.height {
            for x in 0..b.area.width {
                s.push_str(b[(x, y)].symbol());
            }
        }
        assert!(s.contains("target/huge.bin"), "largest file path shown");
        assert!(s.contains("assets/movie.mp4"), "second file path shown");
        assert!(s.contains("4.8 MB"), "file size shown");
    }

    #[test]
    fn renders_treemap_with_boxes_and_footer() {
        use crate::disk::DiskView;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut dv = DiskView::new(std::path::PathBuf::from("/tmp"));
        dv.scanning = false;
        dv.entries = vec![e("big", 9_000_000), e("small", 100_000)];
        let theme = crate::ui::theme::Theme::mc();
        let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
        t.draw(|f| render(f, f.area(), &mut dv, &theme)).unwrap();
        let b = t.backend().buffer();
        let mut s = String::new();
        for y in 0..b.area.height {
            for x in 0..b.area.width {
                s.push_str(b[(x, y)].symbol());
            }
        }
        assert!(s.contains("Disk Explorer"), "title");
        assert!(s.contains("big"), "largest box labeled");
        assert!(s.contains("Backspace"), "footer guidance");
        assert!(s.contains("▶") && s.contains("of total"), "selected-box header");
        assert_eq!(dv.rects.len(), 2, "geometry recorded for navigation");
    }

    #[test]
    fn treemap_covers_area_and_orders_by_input() {
        let entries = vec![e("a", 800), e("b", 150), e("c", 50)];
        let area = Rect { x: 0, y: 0, width: 40, height: 20 };
        let rects = treemap(&entries, area);
        assert_eq!(rects.len(), 3);
        // Largest entry gets the largest box.
        let areas: Vec<u32> = rects.iter().map(|r| r.width as u32 * r.height as u32).collect();
        assert!(areas[0] >= areas[1] && areas[1] >= areas[2], "areas: {areas:?}");
        // Every box lies within the area.
        for r in &rects {
            assert!(r.x + r.width <= area.x + area.width);
            assert!(r.y + r.height <= area.y + area.height);
        }
    }
}
