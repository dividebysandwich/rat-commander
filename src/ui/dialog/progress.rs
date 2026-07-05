//! Progress dialog and the indeterminate "busy" spinner.

use super::widgets::*;
use super::DialogResult;
use crate::ops::progress::{ProgressUpdate, TaskId};
use crate::ui::graphics::{raster, Gfx, Slot};
use ratatui::style::Color;

// ---------------------------------------------------------------------------
// Progress dialog
// ---------------------------------------------------------------------------

pub struct ProgressDialog {
    pub id: TaskId,
    pub verb: &'static str,
    pub current_name: String,
    pub file_done: u64,
    pub file_total: u64,
    pub total_done: u64,
    pub total_total: u64,
    pub files_done: u64,
    pub files_total: u64,
    /// When true, render an indeterminate sweep (e.g. find-file scanning).
    pub indeterminate: bool,
    /// Noun in the indeterminate "{n} {noun} found" line (e.g. "files").
    pub(crate) noun: &'static str,
    /// Transfer-speed samples: (bytes-done, bytes/sec) for the chart.
    pub(crate) samples: Vec<(f64, f64)>,
    peak_speed: f64,
    last_bytes: u64,
    last_instant: Option<std::time::Instant>,
    /// The Abort button's screen rect, recorded so the button can also be clicked
    /// (the indeterminate renderer, and the backgroundable transfer dialog).
    abort_rect: Rect,
    /// The "To background" button rect (backgroundable transfer dialogs only).
    bg_rect: Rect,
    /// Whether this transfer can be sent to the background (copy/move/delete).
    /// Find/checksum/archive progress dialogs stay modal.
    pub backgroundable: bool,
    /// Focused button on a backgroundable dialog: 0 = To background, 1 = Abort.
    focus: u8,
}

impl ProgressDialog {
    pub fn new(id: TaskId, verb: &'static str) -> Self {
        ProgressDialog {
            id,
            verb,
            current_name: String::new(),
            file_done: 0,
            file_total: 0,
            total_done: 0,
            total_total: 0,
            files_done: 0,
            files_total: 0,
            indeterminate: false,
            noun: "files",
            samples: Vec::new(),
            peak_speed: 0.0,
            last_bytes: 0,
            last_instant: None,
            abort_rect: Rect::default(),
            bg_rect: Rect::default(),
            backgroundable: false,
            focus: 0,
        }
    }

    /// An indeterminate progress dialog for find-file scanning.
    pub fn find(id: TaskId) -> Self {
        let mut d = Self::new(id, "Searching");
        d.indeterminate = true;
        d
    }

    /// An indeterminate "scanning" dialog with a custom title `verb` and the
    /// noun used in its "{n} {noun} found" line.
    pub fn scan(id: TaskId, verb: &'static str, noun: &'static str) -> Self {
        let mut d = Self::new(id, verb);
        d.indeterminate = true;
        d.noun = noun;
        d
    }

    /// Hit-test a click against the dialog's buttons: the Abort button on an
    /// indeterminate scan, and the "To background" / "Abort" pair on a
    /// backgroundable transfer. A plain determinate dialog ignores clicks.
    pub(crate) fn handle_click(&self, col: u16, row: u16) -> DialogResult {
        let hit = |r: Rect| {
            r.width > 0 && col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height
        };
        if self.backgroundable {
            if hit(self.bg_rect) {
                return DialogResult::Background(self.id);
            }
            if hit(self.abort_rect) {
                return DialogResult::Abort(self.id);
            }
        } else if self.indeterminate && hit(self.abort_rect) {
            return DialogResult::Abort(self.id);
        }
        DialogResult::None
    }

    pub fn update(&mut self, u: &ProgressUpdate) {
        self.verb = u.verb;
        self.current_name = u.current_name.clone();
        self.file_done = u.file_done;
        self.file_total = u.file_total;
        self.total_done = u.total_done;
        self.total_total = u.total_total;
        self.files_done = u.files_done;
        self.files_total = u.files_total;

        // Sample transfer speed (~every 100 ms) for the chart.
        let now = std::time::Instant::now();
        match self.last_instant {
            None => {
                self.last_instant = Some(now);
                self.last_bytes = u.total_done;
            }
            Some(prev) => {
                let dt = now.duration_since(prev).as_secs_f64();
                if dt >= 0.1 {
                    let speed = u.total_done.saturating_sub(self.last_bytes) as f64 / dt;
                    self.peak_speed = self.peak_speed.max(speed);
                    self.samples.push((u.total_done as f64, speed));
                    if self.samples.len() > 1024 {
                        self.samples.remove(0);
                    }
                    self.last_instant = Some(now);
                    self.last_bytes = u.total_done;
                }
            }
        }
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        // Modal (find/checksum/archive): any of Esc/Enter/q aborts — unchanged.
        if !self.backgroundable {
            return match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => DialogResult::Abort(self.id),
                _ => DialogResult::None,
            };
        }
        // Backgroundable transfer: Esc/q/a abort; b backgrounds; ←/→/Tab move
        // focus between [To background] and [Abort]; Enter activates the focused.
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('a') => DialogResult::Abort(self.id),
            KeyCode::Char('b') => DialogResult::Background(self.id),
            KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
                self.focus ^= 1;
                DialogResult::None
            }
            KeyCode::Enter => {
                if self.focus == 0 {
                    DialogResult::Background(self.id)
                } else {
                    DialogResult::Abort(self.id)
                }
            }
            _ => DialogResult::None,
        }
    }

    /// Estimated time remaining, from the recent transfer speed. `"--:--"` until
    /// there's enough signal (or when the total is unknown).
    pub(crate) fn eta_text(&self) -> String {
        let speed = self.samples.last().map(|s| s.1).unwrap_or(0.0);
        if self.total_total == 0 || speed < 1.0 {
            return "--:--".to_string();
        }
        let remaining = self.total_total.saturating_sub(self.total_done) as f64;
        let secs = (remaining / speed).round() as u64;
        let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
        if h > 0 {
            format!("{h}:{m:02}:{s:02}")
        } else {
            format!("{m:02}:{s:02}")
        }
    }

    fn ratio(done: u64, total: u64) -> f64 {
        if total == 0 {
            if done > 0 { 1.0 } else { 0.0 }
        } else {
            (done as f64 / total as f64).clamp(0.0, 1.0)
        }
    }

    pub(crate) fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme, mut gfx: Option<&mut Gfx>) {
        if self.indeterminate {
            return self.render_indeterminate(f, area, theme, gfx);
        }
        let w = 64u16.min(area.width.saturating_sub(4));
        let rect = centered(area, w, 16);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block(self.verb, theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // file name
                Constraint::Length(1), // file gauge
                Constraint::Length(1), // total label
                Constraint::Length(1), // total gauge
                Constraint::Length(1), // chart title
                Constraint::Min(3),    // speed chart
                Constraint::Length(1), // abort
            ])
            .split(inner);

        let name = crate::util::text::ellipsize(&self.current_name, inner.width as usize);
        f.render_widget(Paragraph::new(Line::from(name)).style(base), rows[0]);

        gauge(
            f,
            gfx.as_deref_mut(),
            rows[1],
            Slot::TransferFileBar,
            Self::ratio(self.file_done, self.file_total),
            &format!("{} / {}", human_size(self.file_done), human_size(self.file_total)),
            theme.exec_fg,
            theme,
        );

        f.render_widget(
            Paragraph::new(Line::from(format!(
                "Total: {} / {}  ({}/{} files)",
                human_size(self.total_done),
                human_size(self.total_total),
                self.files_done,
                self.files_total
            )))
            .style(base),
            rows[2],
        );

        let total_ratio = Self::ratio(self.total_done, self.total_total);
        gauge(
            f,
            gfx.as_deref_mut(),
            rows[3],
            Slot::TransferTotalBar,
            total_ratio,
            &format!("{:.0}%", total_ratio * 100.0),
            theme.panel_border_active,
            theme,
        );

        f.render_widget(
            Paragraph::new(Line::from(format!(
                "Speed: {}/s   peak {}/s   ETA {}",
                human_size(self.samples.last().map(|s| s.1).unwrap_or(0.0) as u64),
                human_size(self.peak_speed as u64),
                self.eta_text(),
            )))
            .style(base),
            rows[4],
        );
        self.render_speed_chart(f, rows[5], theme, gfx.as_deref_mut());

        if self.backgroundable {
            // Two buttons: [To background] on the left, [Abort] on the right, the
            // focused one highlighted.
            let row = rows[6];
            let half = row.width / 2;
            let left = Rect { x: row.x, y: row.y, width: half, height: 1 };
            let right = Rect { x: row.x + half, y: row.y, width: row.width - half, height: 1 };
            let bg = center_button_rect(left, 17);
            let ab = center_button_rect(right, 11);
            self.bg_rect = bg;
            self.abort_rect = ab;
            let (bg_focus, ab_focus) = (self.focus == 0, self.focus == 1);
            if !gfx_button(f, gfx.as_deref_mut(), Slot::Button(0), bg, "To background", bg_focus, theme) {
                f.render_widget(
                    Paragraph::new(Line::from(button("[ To background ]", bg_focus, theme)))
                        .alignment(ratatui::layout::Alignment::Center)
                        .style(base),
                    left,
                );
            }
            if !gfx_button(f, gfx, Slot::Button(1), ab, "Abort", ab_focus, theme) {
                f.render_widget(
                    Paragraph::new(Line::from(button("[ Abort ]", ab_focus, theme)))
                        .alignment(ratatui::layout::Alignment::Center)
                        .style(base),
                    right,
                );
            }
        } else {
            let abort = center_button_rect(rows[6], 11);
            if !gfx_button(f, gfx, Slot::Button(0), abort, "Abort", true, theme) {
                f.render_widget(
                    Paragraph::new(Line::from(button("[ Abort ]", true, theme)))
                        .alignment(ratatui::layout::Alignment::Center)
                        .style(base),
                    rows[6],
                );
            }
        }
    }

    /// A sparkline of transfer speed over bytes transferred. Each column is a
    /// vertical bar of partial-block glyphs, colored with a gradient that
    /// brightens towards the top of the graph.
    fn render_speed_chart(&self, f: &mut Frame, area: Rect, theme: &Theme, gfx: Option<&mut Gfx>) {
        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        if self.samples.len() < 2 {
            f.render_widget(Paragraph::new(Line::from("  measuring…")).style(base), area);
            return;
        }
        let (w, h) = (area.width as usize, area.height as usize);
        if w == 0 || h == 0 {
            return;
        }
        const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let y_max = (self.peak_speed * 1.15).max(1.0);
        let levels = h * 8;

        // Pick a bar color that contrasts with the dialog background: the theme
        // accent when it stands out, otherwise a green that does (e.g. MC's light
        // dialog, where the cyan accent washes out). The gradient runs from that
        // intense color at the top down toward the background near the baseline.
        let bg = theme.dialog_bg;
        let accent = theme.panel_border_active;
        let intense = if (luma(accent) - luma(bg)).abs() >= 80.0 {
            accent
        } else if luma(bg) > 140.0 {
            ratatui::style::Color::Rgb(0x1e, 0x7a, 0x1e) // dark green on light bg
        } else {
            ratatui::style::Color::Rgb(0x4c, 0xff, 0x4c) // light green on dark bg
        };
        let top = intense;
        let bottom = mix_rgb(intense, bg, 0.7);

        // Bin the peak speed by *transferred-bytes position*, so each column owns
        // a fixed byte range. Past columns therefore never change — only the
        // current (rightmost) bars move as the transfer advances — and the graph
        // grows left→right like a progress bar instead of scrolling.
        let x_max = if self.total_total > 0 {
            self.total_total as f64
        } else {
            (self.last_bytes.max(1)) as f64
        };
        let mut bars = vec![0f64; w];
        let mut seen = vec![false; w];
        for &(bytes, speed) in &self.samples {
            let col = ((bytes / x_max) * w as f64).floor().clamp(0.0, (w - 1) as f64) as usize;
            bars[col] = bars[col].max(speed);
            seen[col] = true;
        }
        // Carry the last value across empty bins inside the transferred region so
        // the area stays contiguous up to the current progress; columns beyond it
        // remain empty.
        let done_col = ((self.total_done as f64 / x_max) * w as f64).round() as usize;
        let mut last = 0.0;
        for c in 0..w {
            if seen[c] {
                last = bars[c];
            } else if c < done_col {
                bars[c] = last;
            }
        }

        // Graphics path: a smooth filled line graph in the same accent color.
        if let Some(g) = gfx
            && g.available() {
                let (pw, ph) = g.px_size(area);
                let line = raster::rgb(intense);
                let img = raster::line_graph(pw, ph, &bars, y_max, |_| line, raster::rgb(theme.dialog_bg));
                g.draw(f, area, Slot::TransferSpeed, img);
                return;
            }

        let buf = f.buffer_mut();
        for (col, &bar) in bars.iter().enumerate() {
            let filled = (((bar.max(0.0) / y_max) * levels as f64).round() as usize).min(levels);
            for row in 0..h {
                let from_bottom = h - 1 - row;
                let cell = filled.saturating_sub(from_bottom * 8).min(8);
                let t = if h <= 1 { 1.0 } else { 1.0 - row as f32 / (h - 1) as f32 };
                let style = Style::default().fg(mix_rgb(bottom, top, t)).bg(theme.dialog_bg);
                buf.set_string(
                    area.x + col as u16,
                    area.y + row as u16,
                    BLOCKS[cell].to_string(),
                    style,
                );
            }
        }
    }

    /// Render an indeterminate scanning dialog (current path + sweep + count).
    fn render_indeterminate(&mut self, f: &mut Frame, area: Rect, theme: &Theme, mut gfx: Option<&mut Gfx>) {
        let w = 64u16.min(area.width.saturating_sub(4));
        let rect = centered(area, w, 8);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block(self.verb, theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let line_at = |yy: u16| Rect { x: inner.x, y: yy, width: inner.width, height: 1 };

        f.render_widget(
            Paragraph::new(Line::from(format!("{} {} found", self.files_done, self.noun)))
                .style(base),
            line_at(inner.y),
        );
        let name = crate::util::text::ellipsize(&self.current_name, inner.width as usize);
        f.render_widget(Paragraph::new(Line::from(name)).style(base), line_at(inner.y + 1));

        // A bouncing block sweeps based on the update counter (files_done).
        let bar_area = line_at(inner.y + 3);
        let bar_w = inner.width as usize;
        let block_w = (bar_w / 5).max(1);
        let span = bar_w.saturating_sub(block_w).max(1);
        let phase = (self.files_done as usize) % (2 * span);
        let pos = if phase < span { phase } else { 2 * span - phase };
        let drawn = if let Some(g) = gfx.as_deref_mut() {
            if g.available() && bar_w > 0 {
                let (pw, ph) = g.px_size(bar_area);
                let pos_frac = (pos + block_w / 2) as f64 / bar_w as f64;
                let img = raster::sweep_bar(
                    pw,
                    ph,
                    pos_frac,
                    0.2,
                    raster::rgb(theme.panel_border_active),
                    raster::rgb(theme.panel_border),
                    raster::rgb(theme.dialog_bg),
                );
                g.draw(f, bar_area, Slot::Indeterminate, img);
                true
            } else {
                false
            }
        } else {
            false
        };
        if !drawn {
            let mut bar = String::with_capacity(bar_w);
            for i in 0..bar_w {
                bar.push(if i >= pos && i < pos + block_w { '█' } else { '░' });
            }
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    bar,
                    Style::default().fg(theme.input_bg).bg(theme.dialog_bg),
                ))),
                bar_area,
            );
        }

        // Centered, clickable Abort button (rect recorded for hit-testing).
        let label = "[ Abort ]";
        let bw = label.chars().count() as u16;
        let arect = Rect {
            x: inner.x + inner.width.saturating_sub(bw) / 2,
            y: inner.y + inner.height - 1,
            width: bw,
            height: 1,
        };
        if !gfx_button(f, gfx, Slot::Button(0), arect, "Abort", true, theme) {
            f.render_widget(Paragraph::new(Line::from(button(label, true, theme))).style(base), arect);
        }
        self.abort_rect = arect;
    }
}

/// Draw a progress bar as a gradient pixel "pill" via `gfx`, falling back to the
/// cell [`pulse_gauge`]. The filled portion runs a gradient tinted from `base`,
/// with an animated highlight sweep when the theme is animated; `label` is
/// centered over the bar either way.
#[allow(clippy::too_many_arguments)]
fn gauge(
    f: &mut Frame,
    gfx: Option<&mut Gfx>,
    area: Rect,
    slot: Slot,
    ratio: f64,
    label: &str,
    base: Color,
    theme: &Theme,
) {
    if let Some(g) = gfx
        && g.available() && area.width > 0 && area.height > 0 {
            let (w, h) = g.px_size(area);
            let base_rgb = raster::rgb(base);
            let dark = raster::over((0, 0, 0), base_rgb, 0.55);
            let bright = raster::over(base_rgb, (255, 255, 255), 0.30);
            let animated = theme.animated;
            let anim = theme.anim as f64;
            let fill = move |t: f64| {
                let mut c = raster::over(dark, bright, t);
                if animated {
                    // A soft highlight band sweeps left→right as anim advances.
                    let pos = (anim * 0.02).rem_euclid(1.0);
                    let d = (t - pos)
                        .abs()
                        .min((t - pos + 1.0).abs())
                        .min((t - pos - 1.0).abs());
                    let hi = (1.0 - d / 0.22).clamp(0.0, 1.0);
                    c = raster::over(c, (255, 255, 255), 0.4 * hi);
                }
                c
            };
            let img = raster::gradient_bar(
                w,
                h,
                ratio,
                fill,
                raster::rgb(theme.panel_border),
                raster::rgb(theme.dialog_bg),
            );
            g.draw(f, area, slot, img);
            overlay_label(f, area, label, theme);
            return;
        }
    pulse_gauge(f, area, ratio, label, base, theme);
}

/// Center `label` on a graphics bar (a small readable plate over the pixels).
fn overlay_label(f: &mut Frame, area: Rect, label: &str, theme: &Theme) {
    let w = area.width as usize;
    let chars: Vec<char> = label.chars().take(w).collect();
    if chars.is_empty() {
        return;
    }
    let lstart = area.x + ((w - chars.len()) / 2) as u16;
    let midy = area.y + area.height / 2;
    let s: String = chars.into_iter().collect();
    f.buffer_mut()
        .set_string(lstart, midy, s, Style::default().fg(theme.bar_fg).bg(theme.dialog_bg));
}

// ---------------------------------------------------------------------------
// Busy dialog (indeterminate "working…" spinner)
// ---------------------------------------------------------------------------

/// A small, non-dismissible modal shown while a blocking background operation
/// runs (e.g. `mkfs`). It carries no buttons and swallows all input; the app
/// replaces it when the operation reports back.
pub struct BusyDialog {
    pub title: String,
    pub message: String,
    /// Spinner frame, advanced once per UI tick.
    frame: usize,
}

impl BusyDialog {
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        BusyDialog { title: title.into(), message: message.into(), frame: 0 }
    }

    /// Advance the spinner animation (called from the app's tick handler).
    pub fn tick(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    pub(crate) fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let w = 56u16.min(area.width.saturating_sub(4));
        let rect = centered(area, w, 6);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block(&self.title, theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let spin = SPINNER[self.frame % SPINNER.len()];
        let text = format!("{spin}  {}", self.message);
        // Vertically center the (possibly wrapped) message within the inner box.
        let iw = inner.width.max(1);
        let lines = (text.chars().count() as u16).div_ceil(iw).clamp(1, inner.height);
        let y = inner.y + inner.height.saturating_sub(lines) / 2;
        let text_area = Rect { y, height: lines, ..inner };
        f.render_widget(
            Paragraph::new(text)
                .wrap(Wrap { trim: true })
                .style(base.add_modifier(Modifier::BOLD))
                .alignment(ratatui::layout::Alignment::Center),
            text_area,
        );
    }
}

