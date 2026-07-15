//! The viewer's search matcher.
//!
//! The viewer pages huge files straight from disk, so its search can never hold
//! the whole source in memory: it scans **overlapping windows** of bytes. This
//! module holds the part that decides what a match *is* — compiled once from the
//! search dialog's options into a [`Needle`] — leaving `ViewerState` to do the
//! windowing. Keeping it separate makes the matching rules testable against
//! plain byte slices, with no file or terminal involved.

/// What to look for, compiled from the search dialog's options.
pub enum Needle {
    /// A literal byte string: normal text (optionally ASCII case-insensitive, or
    /// bounded to whole words) and hex mode, which is the same search with the
    /// bytes spelled out.
    Bytes { pat: Vec<u8>, case_insensitive: bool, whole_words: bool },
    /// A regular expression (also how wildcard mode arrives, pre-converted).
    /// Applied to each window's bytes read as lossy UTF-8.
    Re(regex::Regex),
}

/// How much a regex window overlaps its predecessor. A literal needle knows
/// exactly how far a match can straddle a boundary (`pat.len() - 1`), but a
/// regex match has no bounded length, so this is a pragmatic limit: a single
/// match longer than this *in a file-backed source* can be missed at a window
/// seam. In-memory sources are scanned in one window, so they are exact.
pub const RE_OVERLAP: usize = 64 * 1024;

/// Whether `b` is a word byte, for the whole-words option.
fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

impl Needle {
    /// Build from the search dialog's parameters. `None` when the pattern is
    /// unusable — an invalid regex, or hex mode with something that isn't a pair
    /// of hex digits per byte.
    pub fn build(
        pattern: &str,
        regex: bool,
        case_sensitive: bool,
        whole_words: bool,
        hex: bool,
    ) -> Option<Needle> {
        if pattern.trim().is_empty() {
            return None;
        }
        if hex {
            // Hex bytes are exact: case and word boundaries are meaningless.
            return parse_hex_bytes(pattern).map(|pat| Needle::Bytes {
                pat,
                case_insensitive: false,
                whole_words: false,
            });
        }
        if regex {
            let mut pat = pattern.to_string();
            if whole_words {
                pat = format!(r"\b(?:{pat})\b");
            }
            return regex::RegexBuilder::new(&pat)
                .case_insensitive(!case_sensitive)
                .build()
                .ok()
                .map(Needle::Re);
        }
        Some(Needle::Bytes {
            pat: pattern.as_bytes().to_vec(),
            case_insensitive: !case_sensitive,
            whole_words,
        })
    }

    /// How far back a window must overlap its predecessor for this needle so no
    /// match is lost at the seam.
    pub fn overlap(&self) -> usize {
        match self {
            Needle::Bytes { pat, .. } => pat.len().saturating_sub(1),
            Needle::Re(_) => RE_OVERLAP,
        }
    }

    /// The shortest possible match, used to skip windows too small to hold one.
    pub fn min_len(&self) -> usize {
        match self {
            Needle::Bytes { pat, .. } => pat.len(),
            Needle::Re(_) => 1,
        }
    }

    /// Byte offset of the first match in `buf` at or after `from`, if any.
    pub fn find(&self, buf: &[u8], from: usize) -> Option<usize> {
        if from >= buf.len() {
            return None;
        }
        match self {
            Needle::Bytes { pat, case_insensitive, whole_words } => {
                if pat.is_empty() || pat.len() > buf.len() {
                    return None;
                }
                let last = buf.len() - pat.len();
                (from..=last).find(|&i| {
                    let hit = buf[i..i + pat.len()].iter().zip(pat).all(|(a, b)| {
                        if *case_insensitive { a.eq_ignore_ascii_case(b) } else { a == b }
                    });
                    hit && (!*whole_words || word_bounded(buf, i, pat.len()))
                })
            }
            Needle::Re(re) => {
                // The window is read as lossy UTF-8; `from` is a byte index into
                // the original bytes, which the lossy copy may not share when the
                // source holds invalid sequences. Searching the tail and adding
                // `from` back keeps the returned offset in the caller's terms.
                let text = String::from_utf8_lossy(&buf[from..]);
                re.find(&text).map(|m| from + m.start())
            }
        }
    }

}

/// Whether the `len`-byte match at `at` is bounded by non-word bytes.
fn word_bounded(buf: &[u8], at: usize, len: usize) -> bool {
    let before_ok = at == 0 || !is_word(buf[at - 1]);
    let after = at + len;
    let after_ok = after >= buf.len() || !is_word(buf[after]);
    before_ok && after_ok
}

/// Parse a hex byte string like `"48 65 6c"` (whitespace optional) into bytes.
pub fn parse_hex_bytes(s: &str) -> Option<Vec<u8>> {
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.is_empty() || !cleaned.len().is_multiple_of(2) {
        return None;
    }
    (0..cleaned.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&cleaned[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(pat: &str, ci: bool, ww: bool) -> Needle {
        Needle::build(pat, false, !ci, ww, false).expect("builds")
    }

    #[test]
    fn literal_search_respects_case_sensitivity() {
        let hay = b"one TWO three two";
        // Case-insensitive (the dialog's default) finds the first spelling.
        assert_eq!(bytes("two", true, false).find(hay, 0), Some(4));
        // Case-sensitive skips it and lands on the exact one.
        assert_eq!(bytes("two", false, false).find(hay, 0), Some(14));
        assert_eq!(bytes("TWO", false, false).find(hay, 0), Some(4));
    }

    #[test]
    fn find_starts_at_the_offset_so_repeats_advance() {
        let hay = b"aa bb aa";
        let n = bytes("aa", true, false);
        assert_eq!(n.find(hay, 0), Some(0));
        // Resuming past the first hit is what makes a repeated search advance.
        assert_eq!(n.find(hay, 1), Some(6));
        assert_eq!(n.find(hay, 7), None);
        assert_eq!(n.find(hay, 999), None, "an out-of-range start is not a panic");
    }

    #[test]
    fn whole_words_needs_non_word_boundaries() {
        let n = bytes("hit", true, true);
        assert_eq!(n.find(b"a hit here", 0), Some(2));
        assert_eq!(n.find(b"hitting", 0), None, "a prefix of a longer word");
        assert_eq!(n.find(b"xhit", 0), None, "a suffix of a longer word");
        assert_eq!(n.find(b"hit", 0), Some(0), "the whole buffer is a word");
        assert_eq!(n.find(b"_hit", 0), None, "underscore counts as a word byte");
        // Without the option the same haystacks all match.
        assert_eq!(bytes("hit", true, false).find(b"hitting", 0), Some(0));
    }


    #[test]
    fn regex_and_wildcard_modes() {
        let re = Needle::build(r"h.t", true, true, false, false).expect("builds");
        assert_eq!(re.find(b"a hat b", 0), Some(2));
        // Case-insensitive by default (case_sensitive = false).
        let ci = Needle::build(r"h.t", true, false, false, false).expect("builds");
        assert_eq!(ci.find(b"a HAT b", 0), Some(2));
        // Whole words wraps the pattern in boundaries.
        let ww = Needle::build(r"h.t", true, true, true, false).expect("builds");
        assert_eq!(ww.find(b"hatter", 0), None);
        assert_eq!(ww.find(b"the hat", 0), Some(4));
        // An invalid regex is refused rather than panicking.
        assert!(Needle::build("(unclosed", true, true, false, false).is_none());
    }

    #[test]
    fn hex_mode_matches_raw_bytes() {
        let n = Needle::build("48 65", false, false, false, true).expect("builds");
        assert_eq!(n.find(b"xxHello", 0), Some(2));
        // Whitespace is optional, and hex is case-insensitive as *notation*.
        assert_eq!(Needle::build("4865", false, false, false, true).unwrap().find(b"xxHe", 0), Some(2));
        assert_eq!(Needle::build("4865", false, false, false, true).unwrap().find(b"xxhe", 0), None,
                   "the bytes themselves are matched exactly");
        // Odd digits / non-hex are refused.
        assert!(Needle::build("486", false, false, false, true).is_none());
        assert!(Needle::build("zz", false, false, false, true).is_none());
    }

    #[test]
    fn an_empty_pattern_builds_nothing() {
        for hex in [false, true] {
            assert!(Needle::build("", false, false, false, hex).is_none());
            assert!(Needle::build("   ", false, false, false, hex).is_none());
        }
    }

    #[test]
    fn overlap_covers_a_literal_straddling_a_window_seam() {
        // A literal can only straddle by len-1 bytes, which is what the caller
        // rewinds — so no match is ever lost at a seam.
        let n = bytes("abcd", true, false);
        assert_eq!(n.overlap(), 3);
        assert_eq!(n.min_len(), 4);
        // A regex has no bounded length, hence the fixed rewind.
        let re = Needle::build("a+", true, true, false, false).unwrap();
        assert_eq!(re.overlap(), RE_OVERLAP);
    }
}
