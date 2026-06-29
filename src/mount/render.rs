//! Rendering of the [`MountView`] disk-mounter tool.

use super::{ListHit, MountView, Pane};
use crate::ui::theme::Theme;
use crate::util::bytes::human_size;
use crate::util::text::{ellipsize, pad_right};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

pub fn render(f: &mut Frame, area: Rect, mv: &mut MountView, theme: &Theme) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(theme.panel_border_active).bg(theme.panel_bg))
        .title(Span::styled(
            " Disk Manager ",
            Style::default()
                .fg(theme.panel_border_active)
                .bg(theme.panel_bg)
                .add_modifier(Modifier::BOLD),
        ))
        .style(theme.panel_base());
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width < 12 || inner.height < 4 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),    // device + mount lists
            Constraint::Length(2), // selected-device details
            Constraint::Length(1), // status
            Constraint::Length(1), // footer
        ])
        .split(inner);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[0]);

    mv.view_rows = (cols[0].height as usize).saturating_sub(2).max(1);
    render_devices(f, cols[0], mv, theme);
    render_mounts(f, cols[1], mv, theme);
    render_details(f, rows[1], mv, theme);
    render_status(f, rows[2], mv, theme);
    render_footer(f, rows[3], theme);
}

/// A bordered sub-panel; the focused one gets the active accent border.
fn panel(f: &mut Frame, area: Rect, title: &str, focused: bool, theme: &Theme) -> Rect {
    let border = if focused {
        theme.panel_border_active
    } else {
        theme.panel_border
    };
    let b = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border).bg(theme.panel_bg))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(theme.header_fg).bg(theme.panel_bg).add_modifier(Modifier::BOLD),
        ))
        .style(theme.panel_base());
    let inner = b.inner(area);
    f.render_widget(b, area);
    inner
}

fn render_devices(f: &mut Frame, area: Rect, mv: &mut MountView, theme: &Theme) {
    let focused = mv.focus == Pane::Devices;
    let inner = panel(f, area, "Block devices", focused, theme);
    if inner.height == 0 {
        mv.dev_hit = ListHit::default();
        return;
    }
    let w = inner.width as usize;
    let normal = Style::default().fg(theme.panel_fg).bg(theme.panel_bg);
    let dim = Style::default().fg(theme.panel_border).bg(theme.panel_bg);
    let rows = inner.height as usize;
    let top = mv.dev_cursor.saturating_sub(rows.saturating_sub(1));
    mv.dev_hit = ListHit { area: inner, top };

    let mut lines: Vec<Line> = Vec::with_capacity(rows);
    if mv.devices.is_empty() {
        lines.push(Line::from(Span::styled("  (no block devices)", dim)));
    }
    for (i, d) in mv.devices.iter().enumerate().skip(top).take(rows) {
        // Tree: partitions are drawn indented under their parent disk.
        let name_disp = if d.parent.is_some() {
            let last = mv.devices.get(i + 1).is_none_or(|n| n.parent != d.parent);
            format!("{} {}", if last { "└" } else { "├" }, d.name)
        } else {
            d.name.clone()
        };
        // "name  fstype label / model…  SIZE  [→ mountpoint]" — the middle shows
        // the partition's filesystem + volume name when present, else the model.
        let size = human_size(d.size);
        let mnt = match &d.mountpoint {
            Some(p) => format!("  → {p}"),
            None => String::new(),
        };
        let info = if !d.fstype.is_empty() || !d.label.is_empty() {
            format!("{} {}", d.fstype, d.label).trim().to_string()
        } else {
            d.model.clone()
        };
        let name_col = pad_right(&name_disp, 14);
        let right = format!("{size:>7}{mnt}");
        let mid = w.saturating_sub(name_col.chars().count() + right.chars().count() + 2);
        let info_col = pad_right(&ellipsize(&info, mid), mid);
        let text = format!("{name_col} {info_col} {right}");
        let style = if i == mv.dev_cursor && focused {
            theme.cursor
        } else if d.mountpoint.is_some() {
            // Mounted devices are dimmed (they're shown in the mounts pane too).
            dim
        } else {
            normal
        };
        lines.push(Line::from(Span::styled(pad_right(&ellipsize(&text, w), w), style)));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_mounts(f: &mut Frame, area: Rect, mv: &mut MountView, theme: &Theme) {
    let focused = mv.focus == Pane::Mounts;
    let inner = panel(f, area, "Mounts", focused, theme);
    if inner.height == 0 {
        mv.mnt_hit = ListHit::default();
        return;
    }
    let w = inner.width as usize;
    let normal = Style::default().fg(theme.panel_fg).bg(theme.panel_bg);
    let dim = Style::default().fg(theme.panel_border).bg(theme.panel_bg);
    let rows = inner.height as usize;
    let top = mv.mnt_cursor.saturating_sub(rows.saturating_sub(1));
    mv.mnt_hit = ListHit { area: inner, top };

    let mut lines: Vec<Line> = Vec::with_capacity(rows);
    if mv.mounts.is_empty() {
        lines.push(Line::from(Span::styled("  (no device mounts)", dim)));
    }
    for (i, m) in mv.mounts.iter().enumerate().skip(top).take(rows) {
        let dev = m.dev.strip_prefix("/dev/").unwrap_or(&m.dev);
        let text = format!("{}  {}", pad_right(dev, 12), m.mountpoint);
        let style = if i == mv.mnt_cursor && focused {
            theme.cursor
        } else {
            normal
        };
        lines.push(Line::from(Span::styled(pad_right(&ellipsize(&text, w), w), style)));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

/// Two-line details panel for the currently relevant device.
fn render_details(f: &mut Frame, area: Rect, mv: &MountView, theme: &Theme) {
    let Some(d) = mv.detail_device() else {
        return;
    };
    let label = Style::default().fg(theme.header_fg).bg(theme.panel_bg);
    let value = Style::default().fg(theme.panel_fg).bg(theme.panel_bg);
    let dash = |s: &str| if s.is_empty() { "—".to_string() } else { s.to_string() };
    let w = area.width as usize;

    let mut line1 = vec![
        Span::styled(" Model: ", label),
        Span::styled(dash(&d.model), value),
        Span::styled("   Serial: ", label),
        Span::styled(dash(&d.serial), value),
        Span::styled("   Vendor: ", label),
        Span::styled(dash(&d.vendor), value),
    ];
    let where_ = match &d.mountpoint {
        Some(p) => format!("{}  → {p}", d.dev),
        None => format!("{}  (not mounted)", d.dev),
    };
    let mut line2 = vec![
        Span::styled(" Type: ", label),
        Span::styled(dash(&d.fstype), value),
        Span::styled("   Label: ", label),
        Span::styled(dash(&d.label), value),
        Span::styled("   Size: ", label),
        Span::styled(human_size(d.size), value),
        Span::styled("   ", value),
        Span::styled(where_, value),
    ];
    truncate_spans(&mut line1, w);
    truncate_spans(&mut line2, w);

    f.render_widget(
        Paragraph::new(Line::from(line1)).style(theme.panel_base()),
        Rect { height: 1, ..area },
    );
    f.render_widget(
        Paragraph::new(Line::from(line2)).style(theme.panel_base()),
        Rect { y: area.y + 1, height: 1, ..area },
    );
}

/// Trim a span list so its total displayed width fits in `w`.
fn truncate_spans(spans: &mut Vec<Span>, w: usize) {
    let mut used = 0usize;
    let mut keep = 0usize;
    for s in spans.iter_mut() {
        let len = s.content.chars().count();
        if used + len <= w {
            used += len;
            keep += 1;
        } else {
            let room = w.saturating_sub(used);
            *s = Span::styled(ellipsize(s.content.as_ref(), room), s.style);
            keep += 1;
            break;
        }
    }
    spans.truncate(keep);
}

fn render_status(f: &mut Frame, area: Rect, mv: &MountView, theme: &Theme) {
    if mv.status.is_empty() {
        return;
    }
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {}", ellipsize(&mv.status, area.width.saturating_sub(1) as usize)),
            Style::default().fg(theme.header_fg).bg(theme.panel_bg),
        ))),
        area,
    );
}

fn render_footer(f: &mut Frame, area: Rect, theme: &Theme) {
    let hint = "Tab switch   ↑↓ move   Enter actions   u unmount   r refresh   Esc close";
    let line = pad_right(&format!(" {hint}"), area.width as usize);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(line, theme.fkey_label))).style(theme.fkey_label),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::super::{BlockDevice, MountEntry, MountView};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn renders_both_panels() {
        let mut mv = MountView::new();
        mv.devices = vec![
            BlockDevice { name: "sda".into(), dev: "/dev/sda".into(), size: 500_000_000_000, model: "Acme SSD".into(), vendor: "Acme".into(), serial: "SN12345".into(), ..Default::default() },
            BlockDevice { name: "sda1".into(), dev: "/dev/sda1".into(), size: 512_000_000, fstype: "vfat".into(), label: "ESP".into(), mountpoint: Some("/boot".into()), parent: Some("sda".into()) , ..Default::default() },
        ];
        mv.mounts = vec![MountEntry { dev: "/dev/sda1".into(), mountpoint: "/boot".into(), fstype: "vfat".into() }];
        let theme = crate::ui::theme::Theme::mc();
        let mut t = Terminal::new(TestBackend::new(90, 14)).unwrap();
        t.draw(|f| super::render(f, f.area(), &mut mv, &theme)).unwrap();
        let b = t.backend().buffer();
        let mut s = String::new();
        for y in 0..b.area.height {
            for x in 0..b.area.width {
                s.push_str(b[(x, y)].symbol());
            }
        }
        assert!(s.contains("Disk Manager"), "title");
        assert!(s.contains("Block devices"), "devices panel");
        assert!(s.contains("Mounts"), "mounts panel");
        assert!(s.contains("sda"), "device listed");
        assert!(s.contains("/boot"), "mount point shown");
        assert!(s.contains("Esc close"), "footer guidance");
        // The details panel shows the selected device's model/vendor/serial.
        assert!(s.contains("Acme SSD"), "model in the device row + details");
        assert!(s.contains("Serial:") && s.contains("SN12345"), "serial in details");
        assert!(s.contains("Vendor:"), "vendor label in details");
        // The partition is drawn nested under its parent disk.
        assert!(s.contains("└ sda1"), "partition shown as a tree child");
        // The partition row carries its filesystem type and volume label.
        assert!(s.contains("vfat") && s.contains("ESP"), "partition type + label listed");
    }
}
