//! The F2 user menu — a Midnight-Commander-compatible `menu` file.
//!
//! Format (a practical subset of mc's): a line whose first column is a
//! non-blank, non-`#` character is a menu entry; that first character is the
//! hotkey and the rest of the line is the title. The following lines that start
//! with whitespace are the shell command(s) to run for that entry. `#` lines
//! are comments. mc condition lines (`+ …`) and default lines (`= …`) are
//! recognised and skipped (entries are always shown). Macros expanded before
//! running: `%f`/`%p` current file, `%d` current dir, `%t` tagged files,
//! `%s` tagged-or-current, `%%` a literal percent.

use crate::config::paths;

#[derive(Debug, Clone)]
pub struct UserMenuEntry {
    pub hotkey: char,
    pub title: String,
    pub command: String,
}

/// Load the user menu, creating the default file if none exists.
pub fn load_or_create() -> Vec<UserMenuEntry> {
    if let Some(path) = paths::menu_file() {
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                // Repair the historical compress-command bug in any menu file
                // shipped before the fix (see [`fix_compress_commands`]). Best
                // effort on the write; parse the corrected text either way so
                // F2 works this session even if the file can't be rewritten.
                if let Some(fixed) = fix_compress_commands(&text) {
                    let _ = std::fs::write(&path, &fixed);
                    return parse(&fixed);
                }
                return parse(&text);
            }
            Err(_) => {
                // Create the default menu (best effort).
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&path, DEFAULT_MENU);
                return parse(DEFAULT_MENU);
            }
        }
    }
    parse(DEFAULT_MENU)
}

/// The `tar` lines shipped (before the fix) by the four "Compress the current
/// subdirectory" entries. Each ran from *inside* the panel's own directory, so
/// `basename %d` named a sibling that didn't exist there
/// (`tar: <dir>: Cannot stat`).
const BUGGY_COMPRESS_LINES: &[&str] = &[
    "tar cf - \"$Pwd\" | gzip -f9 > \"$Pwd.tar.gz\"",
    "tar cf - \"$Pwd\" | bzip2 -f9 > \"$Pwd.tar.bz2\"",
    "tar cf - \"$Pwd\" | xz -f9 > \"$Pwd.tar.xz\"",
    "tar cf - \"$Pwd\" | zstd -19 -o \"$Pwd.tar.zst\"",
];

/// Repair the compress-command bug in an existing menu file: prepend `cd ..` so
/// `tar` runs from the parent directory. Only the exact shipped-buggy lines are
/// touched, and only when a preceding command hasn't already `cd`'d away, so a
/// customised or already-fixed menu is left untouched. Idempotent — the fixed
/// line no longer matches. Returns the corrected text, or `None` if unchanged.
fn fix_compress_commands(text: &str) -> Option<String> {
    let mut changed = false;
    let mut prev_cmd = "";
    let fixed: Vec<String> = text
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            let fix = BUGGY_COMPRESS_LINES.contains(&trimmed) && !prev_cmd.starts_with("cd ");
            if !trimmed.is_empty() {
                prev_cmd = trimmed;
            }
            if fix {
                changed = true;
                let indent = &line[..line.len() - line.trim_start().len()];
                format!("{indent}cd .. && {trimmed}")
            } else {
                line.to_string()
            }
        })
        .collect();
    if !changed {
        return None;
    }
    let mut out = fixed.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    Some(out)
}

/// Ensure the F2 user `menu` file exists on disk (writing the default if it
/// doesn't yet) and return its path, so Options → Edit menu file can open it in
/// the internal editor. Mirrors [`crate::ext::ensure_ext_file`]. Returns `None`
/// when no config directory is available.
pub fn ensure_menu_file() -> Option<std::path::PathBuf> {
    let path = paths::menu_file()?;
    if !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, DEFAULT_MENU);
    }
    Some(path)
}

/// Parse menu text into entries.
pub fn parse(text: &str) -> Vec<UserMenuEntry> {
    let mut entries: Vec<UserMenuEntry> = Vec::new();
    let mut cur: Option<UserMenuEntry> = None;

    for line in text.lines() {
        let first = line.chars().next();
        match first {
            // Comment, condition (`+`), or default (`=`) lines: ignored.
            Some('#') | Some('+') | Some('=') => continue,
            None => {
                // Blank line: ends the command block if any pending? Keep the
                // entry open so blank lines inside a script are tolerated.
                continue;
            }
            Some(c) if c.is_whitespace() => {
                // Command line for the current entry.
                if let Some(e) = cur.as_mut() {
                    if !e.command.is_empty() {
                        e.command.push('\n');
                    }
                    e.command.push_str(line.trim_start());
                }
            }
            Some(c) => {
                // New entry: hotkey + title.
                if let Some(e) = cur.take() {
                    entries.push(e);
                }
                let title = line[c.len_utf8()..].trim().to_string();
                cur = Some(UserMenuEntry {
                    hotkey: c,
                    title,
                    command: String::new(),
                });
            }
        }
    }
    if let Some(e) = cur.take() {
        entries.push(e);
    }
    // Drop entries with no command (e.g. stray headers).
    entries.retain(|e| !e.command.trim().is_empty());
    entries
}

/// The default menu written when none exists (Midnight Commander style).
pub const DEFAULT_MENU: &str = r#"# rat-commander user menu (Midnight Commander compatible)
#
# A line starting in column 0 with a letter/digit is a menu entry; that
# character is the hotkey. Indented lines below it are the shell commands.
# Macros: %f current file, %d current directory, %t tagged files, %% percent.

@      Do something on the current file
        %f

3      Compress the current subdirectory (tar.gz)
        Pwd=`basename "%d"`
        cd .. && tar cf - "$Pwd" | gzip -f9 > "$Pwd.tar.gz"

4      Compress the current subdirectory (tar.bz2)
        Pwd=`basename "%d"`
        cd .. && tar cf - "$Pwd" | bzip2 -f9 > "$Pwd.tar.bz2"

5      Compress the current subdirectory (tar.xz)
        Pwd=`basename "%d"`
        cd .. && tar cf - "$Pwd" | xz -f9 > "$Pwd.tar.xz"

6      Compress the current subdirectory (tar.zst)
        Pwd=`basename "%d"`
        cd .. && tar cf - "$Pwd" | zstd -19 -o "$Pwd.tar.zst"

m      View manual page
        man -P cat "%f" | ${PAGER:-less}

y      Gzip or gunzip current file
        case "%f" in
            *.gz) gunzip "%f" ;;
            *) gzip "%f" ;;
        esac

b      Bzip2 or bunzip2 current file
        case "%f" in
            *.bz2) bunzip2 "%f" ;;
            *) bzip2 "%f" ;;
        esac

c      Count lines/words/bytes of tagged (or current) files
        wc %s

h      Compute SHA-256 of tagged (or current) files
        sha256sum %s
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_entries_and_commands() {
        let entries = parse(DEFAULT_MENU);
        assert!(!entries.is_empty());
        let tgz = entries.iter().find(|e| e.hotkey == '3').unwrap();
        assert!(tgz.title.contains("tar.gz"));
        assert!(tgz.command.contains("gzip"));
        // condition/comment lines must not become entries
        assert!(entries.iter().all(|e| e.hotkey != '#' && e.hotkey != '+'));
    }

    #[test]
    fn ignores_conditions_and_comments() {
        let text = "# a comment\n+ f \\.txt$\nx Title\n\techo hi\n";
        let e = parse(text);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].hotkey, 'x');
        assert_eq!(e[0].command, "echo hi");
    }

    /// The default compress commands must `cd ..` so `tar` runs from the parent
    /// (the panel's own directory is the cwd, so `basename %d` names a sibling
    /// that isn't there). Regression guard for the original bug report.
    #[test]
    fn default_compress_commands_cd_up_first() {
        for hk in ['3', '4', '5', '6'] {
            let e = parse(DEFAULT_MENU).into_iter().find(|e| e.hotkey == hk).unwrap();
            assert!(
                e.command.contains("cd .. && tar cf - \"$Pwd\""),
                "entry {hk} must cd up before tar: {:?}",
                e.command
            );
        }
        // And the fixed default is stable under a re-run of the migration.
        assert!(fix_compress_commands(DEFAULT_MENU).is_none(), "default is already fixed");
    }

    #[test]
    fn migration_fixes_the_buggy_tar_lines() {
        let buggy = "3      Compress the current subdirectory (tar.gz)\n        Pwd=`basename \"%d\"`\n        tar cf - \"$Pwd\" | gzip -f9 > \"$Pwd.tar.gz\"\n";
        let fixed = fix_compress_commands(buggy).expect("buggy text is rewritten");
        assert!(fixed.contains("        cd .. && tar cf - \"$Pwd\" | gzip -f9 > \"$Pwd.tar.gz\""));
        assert!(fixed.ends_with('\n'), "trailing newline preserved");
        // Idempotent: the corrected text is not rewritten again.
        assert!(fix_compress_commands(&fixed).is_none());
    }

    #[test]
    fn migration_preserves_indentation() {
        // A tab-indented custom copy is fixed with its own indentation kept.
        let buggy = "x Zip it\n\ttar cf - \"$Pwd\" | xz -f9 > \"$Pwd.tar.xz\"\n";
        let fixed = fix_compress_commands(buggy).unwrap();
        assert!(fixed.contains("\tcd .. && tar cf - \"$Pwd\" | xz -f9 > \"$Pwd.tar.xz\""));
    }

    #[test]
    fn migration_leaves_already_fixed_menus_alone() {
        // MC-style fix on a separate line — must not add a second `cd ..`.
        let mc_style = "        Pwd=`basename \"%d\"`\n        cd ..\n        tar cf - \"$Pwd\" | gzip -f9 > \"$Pwd.tar.gz\"\n";
        assert!(fix_compress_commands(mc_style).is_none());
        // An unrelated menu is untouched.
        assert!(fix_compress_commands("x Title\n\techo hi\n").is_none());
    }
}
