//! Inline hex-color highlighting for the viewer and editor: tint the `#` of a
//! `#rgb` / `#rrggbb` / `#rrggbbaa` token with the color it denotes, regardless
//! of (or in the absence of) syntax coloring.

use ratatui::style::Color;

/// For every hex-color token in `chars`, returns `(index of its '#', color)`.
/// The run of hex digits after `#` must be exactly 3, 6, or 8 long and bounded
/// by a non-hex character, so partial or over-long runs (e.g. `#12345`) don't
/// match. For an 8-digit `#rrggbbaa` token the alpha is ignored.
pub fn hex_color_hashes(chars: &[char]) -> Vec<(usize, Color)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '#' {
            i += 1;
            continue;
        }
        // Measure the run of hex digits immediately after the '#'.
        let start = i + 1;
        let mut j = start;
        while j < chars.len() && chars[j].is_ascii_hexdigit() {
            j += 1;
        }
        if let Some(color) = run_color(&chars[start..j]) {
            out.push((i, color));
        }
        // Resume scanning past the run (so the digits can't start a new match).
        i = j.max(i + 1);
    }
    out
}

/// Parse a hex digit run (no leading `#`) into a color when it is 3, 6, or 8
/// digits long; otherwise `None`.
fn run_color(digits: &[char]) -> Option<Color> {
    let nib = |c: char| c.to_digit(16).map(|d| d as u8);
    match digits.len() {
        // #rgb → expand each nibble to a byte (0xA → 0xAA).
        3 => Some(Color::Rgb(
            nib(digits[0])? * 17,
            nib(digits[1])? * 17,
            nib(digits[2])? * 17,
        )),
        // #rrggbb / #rrggbbaa (alpha ignored).
        6 | 8 => {
            let byte = |hi: char, lo: char| Some(nib(hi)? * 16 + nib(lo)?);
            Some(Color::Rgb(
                byte(digits[0], digits[1])?,
                byte(digits[2], digits[3])?,
                byte(digits[4], digits[5])?,
            ))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hashes(s: &str) -> Vec<(usize, Color)> {
        hex_color_hashes(&s.chars().collect::<Vec<_>>())
    }

    #[test]
    fn matches_six_digit_hex() {
        assert_eq!(hashes("color: #ff501a;"), vec![(7, Color::Rgb(0xff, 0x50, 0x1a))]);
    }

    #[test]
    fn matches_three_and_eight_digit_and_uppercase() {
        assert_eq!(hashes("#FFF"), vec![(0, Color::Rgb(0xff, 0xff, 0xff))]);
        assert_eq!(hashes("#0a0b0cff"), vec![(0, Color::Rgb(0x0a, 0x0b, 0x0c))]);
        assert_eq!(hashes("#AbCdEf"), vec![(0, Color::Rgb(0xab, 0xcd, 0xef))]);
    }

    #[test]
    fn rejects_invalid_lengths() {
        assert!(hashes("#12").is_empty()); // 2
        assert!(hashes("#12345").is_empty()); // 5
        assert!(hashes("#1234567").is_empty()); // 7
        assert!(hashes("# fff").is_empty()); // gap
        assert!(hashes("nohash").is_empty());
    }

    #[test]
    fn finds_multiple_per_line() {
        let v = hashes("a #abc b #001122 c");
        assert_eq!(v, vec![(2, Color::Rgb(0xaa, 0xbb, 0xcc)), (9, Color::Rgb(0x00, 0x11, 0x22))]);
    }
}
