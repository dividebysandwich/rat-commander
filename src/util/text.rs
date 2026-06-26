//! Display-width-aware string helpers (handles wide/CJK characters).

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Truncate `s` so its display width is at most `max`, appending nothing.
/// Returns the truncated string and its actual display width.
pub fn truncate_width(s: &str, max: usize) -> (String, usize) {
    if s.width() <= max {
        let w = s.width();
        return (s.to_string(), w);
    }
    let mut out = String::new();
    let mut w = 0;
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if w + cw > max {
            break;
        }
        out.push(ch);
        w += cw;
    }
    (out, w)
}

/// Truncate to `max`, putting an ellipsis at the end if it was shortened.
pub fn ellipsize(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let (mut out, mut w) = truncate_width(s, max.saturating_sub(1));
    out.push('~');
    w += 1;
    let _ = w;
    out
}

/// Left-align `s` in a field of display width `width`, padding with spaces.
pub fn pad_right(s: &str, width: usize) -> String {
    let (mut out, w) = truncate_width(s, width);
    for _ in w..width {
        out.push(' ');
    }
    out
}

/// Right-align `s` in a field of display width `width`.
pub fn pad_left(s: &str, width: usize) -> String {
    let (t, w) = truncate_width(s, width);
    let mut out = String::with_capacity(width);
    for _ in w..width {
        out.push(' ');
    }
    out.push_str(&t);
    out
}
