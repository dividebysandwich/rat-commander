//! The F2 user menu — a Midnight-Commander-compatible `menu` file.
//!
//! Format: a line whose first column is a non-blank, non-`#` character is a menu
//! entry; that first character is the hotkey and the rest of the line is the
//! title. The following whitespace-indented lines are the shell command(s) for
//! that entry. `#` lines are comments.
//!
//! **Condition lines** may precede an entry: `+ <cond>` includes the entry only
//! when the condition is true, `= <cond>` marks the default entry (`=+`/`+=` do
//! both; a trailing `?` debug marker is accepted and ignored). Sub-conditions —
//! evaluated left-to-right, no precedence — are `f`/`F <pat>` (current/other
//! file), `d`/`D <pat>` (current/other dir), `t`/`T <types>` (file type), `x
//! <path>` (executable exists), `!` (negate), `&`/`|`. Type chars: `n` not-dir,
//! `r` regular, `d` dir, `l` link, `c`/`b` char/block device, `f` fifo, `s`
//! socket, `x` executable, `t` tagged. A first line `shell_patterns=0` switches
//! `f`/`d` patterns from shell globs to regular expressions.
//!
//! Macros ([`crate::app::state`] expands them before running): `%f`/`%p` current
//! file, `%d` current dir, `%s` tagged-or-current, `%t` tagged files;
//! `%F`/`%P`/`%D`/`%S`/`%T` the same on the other panel; `%u` tagged (untagged
//! afterwards). Value macros are shell-quoted; `%0f` disables quoting, `%1f`
//! forces it. `%{prompt}` asks the user (verbatim answer), `%%` a literal
//! percent.

use crate::config::paths;
use crate::panel::selection::NameMatcher;

#[derive(Debug, Clone, Default)]
pub struct UserMenuEntry {
    pub hotkey: char,
    pub title: String,
    pub command: String,
    /// mc `+` inclusion condition (raw text), if any — the entry is shown only
    /// when this evaluates true. `None` ⇒ always shown.
    pub include: Option<String>,
    /// mc `=` default condition (raw text), if any — selects the default entry.
    pub default: Option<String>,
}

/// A parsed user menu: the entries plus the `shell_patterns` mode (`true` = shell
/// globs, `false` = regular expressions) that governs `f`/`d` condition patterns.
#[derive(Debug, Clone)]
pub struct UserMenu {
    pub entries: Vec<UserMenuEntry>,
    pub shell_patterns: bool,
}

/// Load the user menu, creating the default file if none exists.
pub fn load_or_create() -> UserMenu {
    if let Some(path) = paths::menu_file() {
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                // Migrate a legacy on-disk menu in place (the historical compress
                // `cd ..` bug and the pre-quoting `"%f"` wrapping). Back it up
                // once, best-effort on the write; parse the migrated text either
                // way so F2 works this session even if the file can't be written.
                if let Some(fixed) = migrate_menu_text(&text) {
                    backup_once(&path, &text);
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

/// Save a one-time `menu.bak` copy of the pre-migration menu, so a user can
/// recover if an automatic migration misjudged their customisations.
fn backup_once(path: &std::path::Path, original: &str) {
    let bak = path.with_extension("bak");
    if !bak.exists() {
        let _ = std::fs::write(&bak, original);
    }
}

/// Migrate a legacy on-disk menu to current conventions: the historical
/// compress-command `cd ..` repair, then unquoting the macros that now
/// shell-quote themselves. Idempotent. Returns the corrected text, or `None` if
/// nothing changed.
fn migrate_menu_text(text: &str) -> Option<String> {
    let mut cur = std::borrow::Cow::Borrowed(text);
    if let Some(fixed) = fix_compress_commands(&cur) {
        cur = std::borrow::Cow::Owned(fixed);
    }
    if let Some(unquoted) = unquote_self_quoting_macros(&cur) {
        cur = std::borrow::Cow::Owned(unquoted);
    }
    match cur {
        std::borrow::Cow::Borrowed(_) => None,
        std::borrow::Cow::Owned(s) => Some(s),
    }
}

/// Strip the double quotes around a *standalone* self-quoting macro (`"%f"` →
/// `%f`, likewise `%p`/`%d`/`%s`/`%t`). The macro must be a whole shell token —
/// a boundary (start/end, whitespace, or a `;|&<>()` `` ` `` `$` metacharacter)
/// on each side — so shell vars (`"$Pwd"`), `%%`, `%{…}`, `%x` (raw extension),
/// and a macro that only abuts adjacent string literals (`echo "x "%t"."`, from
/// mc's own menu) are all left untouched. Idempotent. Returns `None` if
/// unchanged.
fn unquote_self_quoting_macros(text: &str) -> Option<String> {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r#"(^|[\s;|&<>()`$])"%([fpdst])"($|[\s;|&<>()`$])"#).unwrap()
    });
    match re.replace_all(text, "${1}%${2}${3}") {
        std::borrow::Cow::Borrowed(_) => None,
        std::borrow::Cow::Owned(s) => Some(s),
    }
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

/// Parse menu text into entries plus the `shell_patterns` mode. `+`/`=` condition
/// lines are attached to the entry they precede.
pub fn parse(text: &str) -> UserMenu {
    let mut entries: Vec<UserMenuEntry> = Vec::new();
    let mut cur: Option<UserMenuEntry> = None;
    let mut pending_include: Option<String> = None;
    let mut pending_default: Option<String> = None;
    let mut shell_patterns = true;
    let mut seen_content = false;

    for line in text.lines() {
        // The first non-blank line may set the pattern mode (mc convention).
        if !seen_content && !line.trim().is_empty() {
            seen_content = true;
            if let Some(rest) = line.trim().strip_prefix("shell_patterns=") {
                shell_patterns = rest.trim() != "0";
                continue;
            }
        }
        let first = line.chars().next();
        match first {
            Some('#') => continue, // comment
            Some('+') | Some('=') => {
                // Condition line — attach to the next entry.
                if let Some((inc, def, cond)) = parse_condition_prefix(line) {
                    if inc {
                        pending_include = Some(cond.clone());
                    }
                    if def {
                        pending_default = Some(cond);
                    }
                }
            }
            None => continue, // blank line: keep the command block open
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
                // New entry: hotkey + title, taking any pending conditions.
                if let Some(e) = cur.take() {
                    entries.push(e);
                }
                let title = line[c.len_utf8()..].trim().to_string();
                cur = Some(UserMenuEntry {
                    hotkey: c,
                    title,
                    command: String::new(),
                    include: pending_include.take(),
                    default: pending_default.take(),
                });
            }
        }
    }
    if let Some(e) = cur.take() {
        entries.push(e);
    }
    // Drop entries with no command (e.g. stray headers).
    entries.retain(|e| !e.command.trim().is_empty());
    UserMenu { entries, shell_patterns }
}

/// Parse a condition line's leading prefix. Returns `(include, default, cond)`
/// for `+`/`=`/`+=`/`=+` (each with an optional trailing `?` debug marker, which
/// is accepted and ignored), or `None` if the line isn't a condition.
fn parse_condition_prefix(line: &str) -> Option<(bool, bool, String)> {
    let (mut include, mut default, mut end) = (false, false, 0);
    for (i, c) in line.char_indices() {
        match c {
            '+' => {
                include = true;
                end = i + c.len_utf8();
            }
            '=' => {
                default = true;
                end = i + c.len_utf8();
            }
            '?' => end = i + c.len_utf8(), // debug marker — ignored
            _ => break,
        }
    }
    if !include && !default {
        return None;
    }
    Some((include, default, line[end..].trim().to_string()))
}

/// One directory entry's type facts, for evaluating `t <types>` conditions.
#[derive(Debug, Clone, Default)]
pub struct FileCond {
    pub name: String,
    pub is_regular: bool,
    pub is_dir: bool,
    pub is_link: bool,
    pub is_exec: bool,
    pub is_char: bool,
    pub is_block: bool,
    pub is_fifo: bool,
    pub is_socket: bool,
}

/// Everything a menu condition can inspect: the current and other-panel files,
/// their directory paths, and whether either panel has tagged files.
#[derive(Debug, Clone, Default)]
pub struct MenuContext {
    pub file: Option<FileCond>,
    pub other_file: Option<FileCond>,
    pub dir: String,
    pub other_dir: String,
    pub tagged: bool,
    pub other_tagged: bool,
}

/// Evaluate an mc menu condition against `ctx`. Sub-conditions combine strictly
/// **left-to-right** (no operator precedence): `f a | f b & t r` is
/// `((f a | f b) & t r)`. `shell_patterns` selects glob vs regex for `f`/`d`.
pub fn eval_condition(cond: &str, ctx: &MenuContext, shell_patterns: bool) -> bool {
    #[derive(Clone, Copy)]
    enum Op {
        And,
        Or,
    }

    let mut result = true;
    let mut op = Op::And;
    let mut negate = false;
    let mut tokens = cond.split_whitespace();

    while let Some(tok) = tokens.next() {
        match tok {
            "&" => op = Op::And,
            "|" => op = Op::Or,
            "!" => negate = !negate,
            "f" | "F" | "d" | "D" | "t" | "T" | "x" | "y" => {
                let arg = tokens.next().unwrap_or("");
                let mut v = eval_sub(tok, arg, ctx, shell_patterns);
                if negate {
                    v = !v;
                    negate = false;
                }
                result = match op {
                    Op::And => result && v,
                    Op::Or => result || v,
                };
            }
            _ => {} // stray token — ignore
        }
    }
    result
}

/// Evaluate a single sub-condition (`f`/`F`/`d`/`D`/`t`/`T`/`x`/`y`).
fn eval_sub(kind: &str, arg: &str, ctx: &MenuContext, shell_patterns: bool) -> bool {
    let name_matches = |name: &str| {
        NameMatcher::build(arg, /* case_sensitive */ true, shell_patterns)
            .map(|m| m.is_match(name))
            .unwrap_or(false)
    };
    match kind {
        "f" => ctx.file.as_ref().is_some_and(|f| name_matches(&f.name)),
        "F" => ctx.other_file.as_ref().is_some_and(|f| name_matches(&f.name)),
        "d" => name_matches(&ctx.dir),
        "D" => name_matches(&ctx.other_dir),
        "t" => type_matches(arg, ctx.file.as_ref(), ctx.tagged),
        "T" => type_matches(arg, ctx.other_file.as_ref(), ctx.other_tagged),
        "x" => is_executable_path(arg),
        "y" => false, // mcedit syntax condition — not applicable to the panel menu
        _ => false,
    }
}

/// True if `file` matches ANY of the mc type chars in `types`; the `t` char is
/// panel-level (are there tagged files?) rather than file-level.
fn type_matches(types: &str, file: Option<&FileCond>, tagged: bool) -> bool {
    types.chars().any(|c| match c {
        't' => tagged,
        _ => file.is_some_and(|f| match c {
            'n' => !f.is_dir,
            'r' => f.is_regular,
            'd' => f.is_dir,
            'l' => f.is_link,
            'x' => f.is_exec,
            'c' => f.is_char,
            'b' => f.is_block,
            'f' => f.is_fifo,
            's' => f.is_socket,
            _ => false,
        }),
    })
}

/// Whether `path` names an existing executable file (mc `x <path>`). On Unix
/// checks an execute bit; elsewhere just that the path exists.
fn is_executable_path(path: &str) -> bool {
    match std::fs::metadata(path) {
        Ok(m) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                m.is_file() && m.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                let _ = m;
                true
            }
        }
        Err(_) => false,
    }
}

/// The default menu written when none exists (Midnight Commander style).
pub const DEFAULT_MENU: &str = r#"# rat-commander user menu (Midnight Commander compatible)
#
# A line starting in column 0 with a letter/digit is a menu entry; that
# character is the hotkey. Indented lines below it are the shell commands.
#
# Conditions before an entry: "+ <cond>" shows it only when true, "= <cond>"
# makes it the default. Sub-conditions (left to right, ! & |): f/F <pat> file,
# d/D <pat> dir, t/T <types> type (r regular, d dir, l link, x exec, t tagged),
# x <path> executable exists. "shell_patterns=0" on line 1 = regex, else globs.
#
# Macros: %f/%p current file, %d current dir, %s tagged-or-current, %t tagged
# files; %F/%D/%S/%T the same on the other panel; %u tagged (untag after). Value
# macros are shell-quoted (%0f off, %1f on). %% a literal percent.
# %{Prompt text} pops up a dialog and inserts what you type.

+ ! t t
@      Do something on the current file
        CMD=%{Enter command}
        $CMD %f

+ t t
@      Do something on the tagged files
        CMD=%{Enter command}
        for i in %t ; do $CMD "$i" ; done

= t d
3      Compress the current subdirectory (tar.gz)
        Pwd=`basename %d`
        cd .. && tar cf - "$Pwd" | gzip -f9 > "$Pwd.tar.gz"

= t d
4      Compress the current subdirectory (tar.bz2)
        Pwd=`basename %d`
        cd .. && tar cf - "$Pwd" | bzip2 -f9 > "$Pwd.tar.bz2"

= t d
5      Compress the current subdirectory (tar.xz)
        Pwd=`basename %d`
        cd .. && tar cf - "$Pwd" | xz -f9 > "$Pwd.tar.xz"

= t d
6      Compress the current subdirectory (tar.zst)
        Pwd=`basename %d`
        cd .. && tar cf - "$Pwd" | zstd -19 -o "$Pwd.tar.zst"

+ t r
m      View manual page
        man -P cat %f | ${PAGER:-less}

+ ! t t
y      Gzip or gunzip current file
        case %f in
            *.gz) gunzip %f ;;
            *) gzip %f ;;
        esac

+ ! t t
b      Bzip2 or bunzip2 current file
        case %f in
            *.bz2) bunzip2 %f ;;
            *) bzip2 %f ;;
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
        let menu = parse(DEFAULT_MENU);
        assert!(!menu.entries.is_empty());
        let tgz = menu.entries.iter().find(|e| e.hotkey == '3').unwrap();
        assert!(tgz.title.contains("tar.gz"));
        assert!(tgz.command.contains("gzip"));
        // condition/comment lines must not become entries
        assert!(menu.entries.iter().all(|e| e.hotkey != '#' && e.hotkey != '+'));
        // the default menu ships with bare (self-quoting) macros and is stable
        // under the upgrade migration.
        assert!(!menu.entries.iter().any(|e| e.command.contains("\"%f\"") || e.command.contains("\"%d\"")));
        assert!(migrate_menu_text(DEFAULT_MENU).is_none(), "default needs no migration");
    }

    #[test]
    fn captures_conditions_and_skips_comments() {
        let text = "# a comment\n+ f \\.txt$\nx Title\n\techo hi\n";
        let menu = parse(text);
        assert_eq!(menu.entries.len(), 1);
        let e = &menu.entries[0];
        assert_eq!(e.hotkey, 'x');
        assert_eq!(e.command, "echo hi");
        assert_eq!(e.include.as_deref(), Some("f \\.txt$"));
        assert!(e.default.is_none());
    }

    #[test]
    fn parses_prefixes_and_shell_patterns() {
        let text = "shell_patterns=0\n=+ f \\.c$ & t r\nc Compile\n\tcc %f\n= t d\nz Zip\n\tzip x\n";
        let menu = parse(text);
        assert!(!menu.shell_patterns, "shell_patterns=0 ⇒ regex mode");
        let c = menu.entries.iter().find(|e| e.hotkey == 'c').unwrap();
        assert_eq!(c.include.as_deref(), Some("f \\.c$ & t r"));
        assert_eq!(c.default.as_deref(), Some("f \\.c$ & t r"), "=+ sets both");
        let z = menu.entries.iter().find(|e| e.hotkey == 'z').unwrap();
        assert_eq!(z.default.as_deref(), Some("t d"));
        assert!(z.include.is_none(), "= sets only the default");
    }

    /// The default compress commands must `cd ..` so `tar` runs from the parent
    /// (the panel's own directory is the cwd, so `basename %d` names a sibling
    /// that isn't there). Regression guard for the original bug report.
    #[test]
    fn default_compress_commands_cd_up_first() {
        for hk in ['3', '4', '5', '6'] {
            let e = parse(DEFAULT_MENU).entries.into_iter().find(|e| e.hotkey == hk).unwrap();
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

    fn regular(name: &str) -> FileCond {
        FileCond { name: name.into(), is_regular: true, ..Default::default() }
    }

    #[test]
    fn eval_tagged_and_negation() {
        let mut ctx = MenuContext::default();
        assert!(!eval_condition("t t", &ctx, true));
        assert!(eval_condition("! t t", &ctx, true));
        ctx.tagged = true;
        assert!(eval_condition("t t", &ctx, true));
        assert!(!eval_condition("! t t", &ctx, true));
        assert!(eval_condition("", &ctx, true), "empty condition ⇒ include");
    }

    #[test]
    fn eval_file_pattern_and_type_regex() {
        // Regex mode: \.c$ is unanchored ends-with; & t r requires a regular file.
        let file = MenuContext { file: Some(regular("main.c")), ..Default::default() };
        assert!(eval_condition("f \\.c$ & t r", &file, false));
        let dir = MenuContext {
            file: Some(FileCond { name: "main.c".into(), is_dir: true, ..Default::default() }),
            ..Default::default()
        };
        assert!(!eval_condition("f \\.c$ & t r", &dir, false), "a dir fails t r");
    }

    #[test]
    fn eval_left_to_right_glob() {
        // ((f *.tar.gz | f *.tgz) & t n) — glob mode, evaluated left to right.
        let tgz = MenuContext { file: Some(regular("archive.tgz")), ..Default::default() };
        assert!(eval_condition("f *.tar.gz | f *.tgz & t n", &tgz, true));
        let txt = MenuContext { file: Some(regular("notes.txt")), ..Default::default() };
        assert!(!eval_condition("f *.tar.gz | f *.tgz & t n", &txt, true));
    }

    #[test]
    fn eval_type_set_and_other_panel() {
        let link = MenuContext {
            file: Some(FileCond { name: "l".into(), is_link: true, ..Default::default() }),
            ..Default::default()
        };
        assert!(eval_condition("t rl", &link, true), "t rl = regular OR link");
        let other = MenuContext { other_dir: "/tmp/work".into(), ..Default::default() };
        assert!(eval_condition("D work", &other, false), "D matches other dir (regex)");
        assert!(!eval_condition("F anything", &other, false), "no other file ⇒ false");
    }

    #[cfg(unix)]
    #[test]
    fn eval_executable_path() {
        assert!(eval_condition("x /bin/sh", &MenuContext::default(), true));
        assert!(!eval_condition("x /definitely/not/here", &MenuContext::default(), true));
    }

    #[test]
    fn migration_unquotes_self_quoting_macros() {
        let old = "y Gzip\n\tcase \"%f\" in *.gz) gunzip \"%f\" ;; esac\nz Zip\n\tPwd=`basename \"%d\"`\n\techo \"$Pwd\" %{keep}\n";
        let new = migrate_menu_text(old).expect("migrated");
        assert!(new.contains("case %f in") && new.contains("gunzip %f"), "%f unquoted: {new}");
        assert!(new.contains("basename %d"), "%d unquoted");
        assert!(new.contains("\"$Pwd\""), "shell-var quotes kept");
        assert!(new.contains("%{keep}"), "prompt macro untouched");
        assert!(migrate_menu_text(&new).is_none(), "idempotent");
        assert!(unquote_self_quoting_macros("gzip %f\ncat %d\n").is_none(), "bare macros untouched");
        // mc's own menu abuts %t against adjacent string literals — not a quoted
        // macro token, so it must be left alone.
        assert!(
            unquote_self_quoting_macros("echo \"Cannot decode \"%t\".\"\n").is_none(),
            "adjacent-literal %t is not a quoted macro"
        );
        // A quoted macro at end-of-string (no trailing char) is still unquoted.
        assert_eq!(unquote_self_quoting_macros("cat \"%f\"").as_deref(), Some("cat %f"));
    }
}
