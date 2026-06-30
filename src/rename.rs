//! Multi-file rename engine.
//!
//! Expands a filename *mask* with placeholders into a new name for each
//! selected file, then optionally applies a search-and-replace and a case
//! transform. This is pure logic (no I/O, no clock) so it is fully unit-tested;
//! the date/time strings are captured once by the caller via [`date_time_now`].
//!
//! Supported mask placeholders (case-insensitive keyword, brackets literal when
//! unrecognised):
//! - `[N]` — file name without its extension; `[N3-5]`, `[N3-]`, `[N3]` slice it
//! - `[E]` — file extension without the dot; `[E1-2]` etc. slice it
//! - `[C]` — the running counter
//! - `[YMD]` — the captured date, `YYYYMMDD`
//! - `[hms]` — the captured time, `HHMMSS`

/// How the generated name's letter case is transformed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseMode {
    /// Leave the case as produced by the mask.
    Unchanged,
    /// Lower-case the whole result.
    Lower,
    /// Upper-case the whole result.
    Upper,
}

impl CaseMode {
    /// All variants, in cycle order (matches the dialog's ◂ ▸ chooser).
    pub const ALL: [CaseMode; 3] = [CaseMode::Unchanged, CaseMode::Lower, CaseMode::Upper];

    /// Human label shown in the dialog.
    pub fn label(self) -> &'static str {
        match self {
            CaseMode::Unchanged => "unchanged",
            CaseMode::Lower => "lowercase",
            CaseMode::Upper => "UPPERCASE",
        }
    }
}

/// A complete multi-rename specification, applied per file by [`RenameRule::apply`].
#[derive(Debug, Clone)]
pub struct RenameRule {
    /// The rename mask with `[...]` placeholders.
    pub mask: String,
    pub case: CaseMode,
    /// Counter value for the first file.
    pub counter_start: i64,
    /// Added to the counter for each successive file.
    pub counter_step: i64,
    /// Minimum counter width (zero-padded).
    pub counter_digits: usize,
    /// Substring searched for in the generated name (empty = no replacement).
    pub search: String,
    /// What `search` is replaced with.
    pub replace: String,
    /// Whether `search` matches case-sensitively.
    pub search_case_sensitive: bool,
    /// Substituted for `[YMD]` (e.g. `"20260630"`).
    pub date: String,
    /// Substituted for `[hms]` (e.g. `"143007"`).
    pub time: String,
}

impl RenameRule {
    /// The new name for `original` at zero-based position `index`.
    pub fn apply(&self, original: &str, index: usize) -> String {
        let (stem, ext) = split_name(original);
        let counter = self.counter_start + (index as i64) * self.counter_step;
        let counter_str = format!("{counter:0width$}", width = self.counter_digits);

        let mut name = expand_mask(&self.mask, stem, ext, &counter_str, &self.date, &self.time);
        // The default mask "[N].[E]" leaves a trailing dot for extension-less
        // files; drop the dot an empty [E] produced so it round-trips cleanly.
        if ext.is_empty() && name.ends_with('.') {
            name.pop();
        }
        let name = replace_all(&name, &self.search, &self.replace, self.search_case_sensitive);
        match self.case {
            CaseMode::Unchanged => name,
            CaseMode::Lower => name.to_lowercase(),
            CaseMode::Upper => name.to_uppercase(),
        }
    }
}

/// Split a file name into (stem, extension-without-dot). A leading dot is part of
/// the name (a dotfile like `.bashrc` has no extension); the split is on the last
/// interior dot (so `a.tar.gz` → `("a.tar", "gz")`).
fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i + 1..]),
        _ => (name, ""),
    }
}

/// Expand every `[...]` placeholder in `mask`. Unrecognised tokens are emitted
/// verbatim (brackets included) so literal `[`/`]` survive.
fn expand_mask(mask: &str, stem: &str, ext: &str, counter: &str, date: &str, time: &str) -> String {
    let chars: Vec<char> = mask.chars().collect();
    let mut out = String::with_capacity(mask.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '['
            && let Some(close) = (i + 1..chars.len()).find(|&j| chars[j] == ']')
        {
            let token: String = chars[i + 1..close].iter().collect();
            if let Some(sub) = substitute(&token, stem, ext, counter, date, time) {
                out.push_str(&sub);
                i = close + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Resolve a single placeholder token (the text between the brackets), or `None`
/// if it is not a recognised placeholder.
fn substitute(token: &str, stem: &str, ext: &str, counter: &str, date: &str, time: &str) -> Option<String> {
    match token.to_ascii_lowercase().as_str() {
        "c" => return Some(counter.to_string()),
        "ymd" => return Some(date.to_string()),
        "hms" => return Some(time.to_string()),
        _ => {}
    }
    let first = token.chars().next()?;
    let source = match first.to_ascii_uppercase() {
        'N' => stem,
        'E' => ext,
        _ => return None,
    };
    let rest = &token[first.len_utf8()..];
    if rest.is_empty() {
        return Some(source.to_string());
    }
    let (start, end) = parse_range(rest, source.chars().count())?;
    Some(substr(source, start, end))
}

/// Parse a 1-based inclusive slice spec like `"3-5"`, `"3-"`, `"-5"` or `"3"`.
fn parse_range(spec: &str, len: usize) -> Option<(usize, usize)> {
    if let Some(dash) = spec.find('-') {
        let (l, r) = (&spec[..dash], &spec[dash + 1..]);
        let start = if l.is_empty() { 1 } else { l.parse().ok()? };
        let end = if r.is_empty() { len } else { r.parse().ok()? };
        Some((start, end))
    } else {
        let n = spec.parse().ok()?;
        Some((n, n))
    }
}

/// Characters `start..=end` (1-based, inclusive) of `s`, clamped to its length.
fn substr(s: &str, start: usize, end: usize) -> String {
    if start == 0 || end < start {
        return String::new();
    }
    s.chars().skip(start - 1).take(end - start + 1).collect()
}

/// Replace every occurrence of `needle` in `haystack` with `repl`. When
/// `case_sensitive` is false, matching is ASCII-case-insensitive (non-ASCII
/// bytes still match exactly, which keeps UTF-8 boundaries intact).
fn replace_all(haystack: &str, needle: &str, repl: &str, case_sensitive: bool) -> String {
    if needle.is_empty() {
        return haystack.to_string();
    }
    if case_sensitive {
        return haystack.replace(needle, repl);
    }
    let (hb, nb) = (haystack.as_bytes(), needle.as_bytes());
    let mut out = String::with_capacity(haystack.len());
    let mut i = 0;
    while i < hb.len() {
        if i + nb.len() <= hb.len() && hb[i..i + nb.len()].eq_ignore_ascii_case(nb) {
            out.push_str(repl);
            i += nb.len();
        } else {
            let c = haystack[i..].chars().next().unwrap();
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

/// Capture the current date (`YYYYMMDD`) and time (`HHMMSS`) for `[YMD]`/`[hms]`.
/// Uses the same calendar-agnostic (UTC) basis as the listing's `format_time`.
pub fn date_time_now() -> (String, String) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, m, d, h, mi, s) = civil_from_unix(secs);
    (format!("{y:04}{m:02}{d:02}"), format!("{h:02}{mi:02}{s:02}"))
}

/// Convert a Unix timestamp (UTC) into civil (year, month, day, hour, min, sec).
/// Howard Hinnant's `civil_from_days` algorithm, with seconds.
fn civil_from_unix(secs: i64) -> (i64, i64, i64, i64, i64, i64) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (hour, min, sec) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d, hour, min, sec)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(mask: &str) -> RenameRule {
        RenameRule {
            mask: mask.to_string(),
            case: CaseMode::Unchanged,
            counter_start: 1,
            counter_step: 1,
            counter_digits: 0,
            search: String::new(),
            replace: String::new(),
            search_case_sensitive: false,
            date: "20260630".to_string(),
            time: "143007".to_string(),
        }
    }

    #[test]
    fn default_mask_round_trips_names() {
        let r = rule("[N].[E]");
        assert_eq!(r.apply("photo.jpg", 0), "photo.jpg");
        assert_eq!(r.apply("archive.tar.gz", 0), "archive.tar.gz");
        // Extension-less files don't gain a trailing dot.
        assert_eq!(r.apply("README", 0), "README");
        // Dotfiles have no extension.
        assert_eq!(r.apply(".bashrc", 0), ".bashrc");
    }

    #[test]
    fn counter_increments_with_padding() {
        let mut r = rule("img[C].[E]");
        r.counter_digits = 3;
        assert_eq!(r.apply("a.png", 0), "img001.png");
        assert_eq!(r.apply("b.png", 1), "img002.png");
        assert_eq!(r.apply("c.png", 2), "img003.png");
    }

    #[test]
    fn counter_honours_start_and_step() {
        let mut r = rule("[C]");
        r.counter_start = 10;
        r.counter_step = 5;
        assert_eq!(r.apply("x", 0), "10");
        assert_eq!(r.apply("x", 1), "15");
        assert_eq!(r.apply("x", 2), "20");
    }

    #[test]
    fn substring_slices() {
        assert_eq!(rule("[N1-3]").apply("hello.txt", 0), "hel");
        assert_eq!(rule("[N3-]").apply("hello.txt", 0), "llo");
        assert_eq!(rule("[N2]").apply("hello.txt", 0), "e");
        assert_eq!(rule("[E1-2]").apply("a.jpeg", 0), "jp");
        // Out-of-range slices clamp to empty / available chars.
        assert_eq!(rule("[N9-12]").apply("hi.txt", 0), "");
    }

    #[test]
    fn date_and_time_tokens() {
        assert_eq!(rule("[YMD]_[hms].[E]").apply("a.log", 0), "20260630_143007.log");
    }

    #[test]
    fn unknown_tokens_stay_literal() {
        assert_eq!(rule("[X][N].[E]").apply("a.txt", 0), "[X]a.txt");
        assert_eq!(rule("[N]([C]).[E]").apply("a.txt", 0), "a(1).txt");
    }

    #[test]
    fn case_transforms() {
        let mut r = rule("[N].[E]");
        r.case = CaseMode::Upper;
        assert_eq!(r.apply("Photo.Jpg", 0), "PHOTO.JPG");
        r.case = CaseMode::Lower;
        assert_eq!(r.apply("Photo.Jpg", 0), "photo.jpg");
    }

    #[test]
    fn search_replace_respects_case_flag() {
        let mut r = rule("[N].[E]");
        r.search = "img".to_string();
        r.replace = "pic".to_string();
        // Case-insensitive by default: matches IMG / Img / img.
        assert_eq!(r.apply("IMG_01.jpg", 0), "pic_01.jpg");
        r.search_case_sensitive = true;
        assert_eq!(r.apply("IMG_01.jpg", 0), "IMG_01.jpg");
        assert_eq!(r.apply("img_01.jpg", 0), "pic_01.jpg");
    }

    #[test]
    fn case_labels_cover_all_variants() {
        for c in CaseMode::ALL {
            assert!(!c.label().is_empty());
        }
    }
}
