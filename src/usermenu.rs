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
            Ok(text) => return parse(&text),
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
        tar cf - "$Pwd" | gzip -f9 > "$Pwd.tar.gz"

4      Compress the current subdirectory (tar.bz2)
        Pwd=`basename "%d"`
        tar cf - "$Pwd" | bzip2 -f9 > "$Pwd.tar.bz2"

5      Compress the current subdirectory (tar.xz)
        Pwd=`basename "%d"`
        tar cf - "$Pwd" | xz -f9 > "$Pwd.tar.xz"

6      Compress the current subdirectory (tar.zst)
        Pwd=`basename "%d"`
        tar cf - "$Pwd" | zstd -19 -o "$Pwd.tar.zst"

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
}
