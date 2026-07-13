//! `rc.ext` — user-editable file associations in Midnight Commander's classic
//! `mc.ext` format.
//!
//! A line starting in column 0 is a *matcher*; the indented `Key=Value` lines
//! below it are *actions*. Supported matchers:
//!
//! * `regex/PATTERN`   — regex on the file name (`regex/i/…` = case-insensitive)
//! * `shell/PATTERN`   — `.ext` matches a name suffix; anything else is an exact
//!   name match (`shell/i/…` = case-insensitive)
//!
//! `type/…` / `directory/…` and other matcher kinds are recognised but skipped
//! (their action block is ignored). Supported actions are `Open` (Enter),
//! `View` (F3) and `Edit` (F4); `Icon=` and unknown keys are simply stored and
//! ignored by the dispatcher.
//!
//! This mirrors [`crate::usermenu`] (the F2 `menu` file) in structure and load
//! behaviour: the default file is written on first run.

use crate::config::paths;
use std::collections::HashMap;

/// How an [`ExtEntry`] decides whether it applies to a file name.
pub enum Matcher {
    Regex(regex::Regex),
    /// `pat` is matched as a suffix when it starts with `.`, else as an exact
    /// name. `ci` selects case-insensitive comparison.
    Shell { pat: String, ci: bool },
}

impl Matcher {
    fn matches(&self, filename: &str) -> bool {
        match self {
            Matcher::Regex(re) => re.is_match(filename),
            Matcher::Shell { pat, ci } => {
                if *ci {
                    let name = filename.to_lowercase();
                    let pat = pat.to_lowercase();
                    Self::shell_match(&name, &pat)
                } else {
                    Self::shell_match(filename, pat)
                }
            }
        }
    }

    fn shell_match(name: &str, pat: &str) -> bool {
        if pat.starts_with('.') {
            name.ends_with(pat)
        } else {
            name == pat
        }
    }
}

/// A single matcher plus its actions (`Open`/`View`/`Edit`/…).
pub struct ExtEntry {
    matcher: Matcher,
    actions: HashMap<String, String>,
}

impl ExtEntry {
    /// The command template for an action key, if defined.
    pub fn action(&self, key: &str) -> Option<&str> {
        self.actions.get(key).map(|s| s.as_str())
    }
}

/// All parsed `rc.ext` rules, in file order (first match wins).
#[derive(Default)]
pub struct ExtRules {
    entries: Vec<ExtEntry>,
}

impl ExtRules {
    /// Load `rc.ext`, creating the default file if none exists.
    pub fn load_or_create() -> ExtRules {
        if let Some(path) = paths::ext_file() {
            match std::fs::read_to_string(&path) {
                Ok(text) => return parse(&text),
                Err(_) => {
                    // Create the default file (best effort).
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(&path, DEFAULT_EXT);
                    return parse(DEFAULT_EXT);
                }
            }
        }
        parse(DEFAULT_EXT)
    }

    /// The first rule whose matcher applies to `filename`.
    pub fn lookup(&self, filename: &str) -> Option<&ExtEntry> {
        self.entries.iter().find(|e| e.matcher.matches(filename))
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Parse a matcher line (already trimmed). Returns `None` for unsupported
/// matcher kinds (`type/`, `directory/`, …) or an invalid regex — the caller
/// then skips that entry's action block.
fn parse_matcher(line: &str) -> Option<Matcher> {
    if let Some(rest) = line.strip_prefix("regex/") {
        let (ci, pat) = strip_ci(rest);
        return regex::RegexBuilder::new(pat)
            .case_insensitive(ci)
            .build()
            .ok()
            .map(Matcher::Regex);
    }
    if let Some(rest) = line.strip_prefix("shell/") {
        let (ci, pat) = strip_ci(rest);
        return Some(Matcher::Shell {
            pat: pat.to_string(),
            ci,
        });
    }
    None
}

/// Strip a leading `i/` (case-insensitive marker) from a matcher pattern.
fn strip_ci(rest: &str) -> (bool, &str) {
    match rest.strip_prefix("i/") {
        Some(p) => (true, p),
        None => (false, rest),
    }
}

/// Parse `rc.ext` text into rules.
pub fn parse(text: &str) -> ExtRules {
    let mut entries: Vec<ExtEntry> = Vec::new();
    let mut cur: Option<ExtEntry> = None;

    for line in text.lines() {
        match line.chars().next() {
            None => continue,          // blank line
            Some('#') => continue,     // comment
            Some(c) if c.is_whitespace() => {
                // Action line for the current entry: `Key=Value`.
                if let Some(e) = cur.as_mut()
                    && let Some((k, v)) = line.trim().split_once('=')
                {
                    e.actions.insert(k.trim().to_string(), v.trim().to_string());
                }
            }
            Some(_) => {
                // Matcher line (column 0). Flush the previous entry first.
                if let Some(e) = cur.take() {
                    entries.push(e);
                }
                cur = parse_matcher(line.trim()).map(|matcher| ExtEntry {
                    matcher,
                    actions: HashMap::new(),
                });
            }
        }
    }
    if let Some(e) = cur.take() {
        entries.push(e);
    }
    // Drop entries with no actions (e.g. a matcher with an empty block).
    entries.retain(|e| !e.actions.is_empty());
    ExtRules { entries }
}

/// The default file written when none exists (Midnight Commander style).
pub const DEFAULT_EXT: &str = r#"# rat-commander file associations (Midnight Commander mc.ext compatible)
#
# A line starting in column 0 is a matcher; indented "Key=Value" lines below it
# are actions. Matchers:
#     regex/PATTERN    regex on the file name (regex/i/... = case-insensitive)
#     shell/.ext       file-name suffix       (shell/name = exact name)
# Actions:
#     Open=...   run on Enter. "%cd <path>/<prefix>://" mounts the file with the
#                Midnight Commander extfs.d script <prefix> (e.g. uzip, iso9660,
#                rpm) and browses it as a directory.
#     View=...   run on F3. "%view{ascii}" or "%view{hex}" pipes the command's
#                output into the built-in viewer.
#     Edit=...   run on F4.
# Macros: %f / %p file name, %d directory, %x extension, %% a literal percent.
#
# extfs scripts are found in ~/.local/share/mc/extfs.d and /usr/lib/mc/extfs.d.

# zip (browse via the uzip extfs script; F3 shows the archive listing)
regex/\.(zip|ZIP)$
    Open=%cd %p/uzip://
    View=%view{ascii} unzip -v %f

# ISO9660 CD/DVD image
shell/i/.iso
    Open=%cd %p/iso9660://

# RPM package
shell/i/.rpm
    Open=%cd %p/rpm://
    View=%view{ascii} rpm -qivlp --nomanifest %f

# Debian package
shell/i/.deb
    Open=%cd %p/deb://
    View=%view{ascii} dpkg-deb -I %f && dpkg-deb -c %f

# LHA archive
shell/i/.lha
    Open=%cd %p/ulha://
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regex_and_shell_matchers() {
        let rules = parse(
            "regex/\\.(zip|ZIP)$\n    Open=%cd %p/uzip://\nshell/.iso\n    Open=%cd %p/iso9660://\n",
        );
        assert!(rules.lookup("archive.zip").is_some());
        assert!(rules.lookup("archive.ZIP").is_some());
        assert!(rules.lookup("archive.tar").is_none());
        assert_eq!(
            rules.lookup("disc.iso").unwrap().action("Open"),
            Some("%cd %p/iso9660://")
        );
        // exact name (not a suffix) does not spuriously match
        assert!(rules.lookup("iso").is_none());
    }

    #[test]
    fn case_insensitive_shell() {
        let rules = parse("shell/i/.rpm\n    Open=%cd %p/rpm://\n");
        assert!(rules.lookup("pkg.rpm").is_some());
        assert!(rules.lookup("PKG.RPM").is_some());
    }

    #[test]
    fn captures_actions_and_skips_unsupported() {
        let rules = parse(
            "# a comment\nregex/\\.png$\n    Open=xdg-open %f\n    View=%view{ascii} file %f\n    Icon=image.xpm\ntype/PNG image\n    Open=should-be-skipped\n",
        );
        let e = rules.lookup("photo.png").unwrap();
        assert_eq!(e.action("Open"), Some("xdg-open %f"));
        assert_eq!(e.action("View"), Some("%view{ascii} file %f"));
        assert_eq!(e.action("Icon"), Some("image.xpm"));
        // The type/ entry is skipped entirely, so nothing else matches.
        assert_eq!(rules.entries.len(), 1);
    }

    #[test]
    fn first_match_wins() {
        let rules = parse("shell/.tgz\n    Open=first\nregex/\\.tgz$\n    Open=second\n");
        assert_eq!(rules.lookup("x.tgz").unwrap().action("Open"), Some("first"));
    }

    #[test]
    fn value_may_contain_equals() {
        let rules = parse("shell/.mk\n    Open=make -f %f VAR=value\n");
        assert_eq!(
            rules.lookup("build.mk").unwrap().action("Open"),
            Some("make -f %f VAR=value")
        );
    }

    #[test]
    fn default_file_parses_with_zip_entry() {
        let rules = parse(DEFAULT_EXT);
        let zip = rules.lookup("thing.zip").expect("zip entry present");
        assert_eq!(zip.action("Open"), Some("%cd %p/uzip://"));
        assert!(zip.action("View").unwrap().starts_with("%view{ascii}"));
    }
}
