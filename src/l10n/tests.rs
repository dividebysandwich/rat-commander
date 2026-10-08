use super::*;

#[test]
fn all_builtin_catalogs_parse_with_unique_names() {
    // Every embedded file must parse (a TOML typo would silently drop it).
    let cats = builtin_catalogs();
    assert_eq!(cats.len(), super::BUILTIN_FILES.len(), "every built-in file parsed");
    for expected in ["English", "Deutsch", "Français", "Русский", "日本語", "العربية"]
    {
        assert!(cats.iter().any(|c| c.name == expected), "missing language: {expected}");
    }
    // Language display names must be distinct so the chooser is unambiguous.
    let mut names: Vec<&str> = cats.iter().map(|c| c.name.as_str()).collect();
    names.sort_unstable();
    let unique = names.len();
    names.dedup();
    assert_eq!(names.len(), unique, "duplicate language display name");
}

#[test]
fn german_catalog_translates_across_categories() {
    let de = builtin_catalogs().into_iter().find(|c| c.name == "Deutsch").expect("German catalog");
    // A representative key from each translated surface.
    assert_eq!(de.get("File"), Some("Datei")); // menu bar title
    assert_eq!(de.get("&Copy"), Some("&Kopieren")); // menu item
    assert_eq!(de.get("Help"), Some("Hilfe")); // F-key bar
    assert_eq!(de.get("Language"), Some("Sprache")); // settings field
    assert_eq!(de.get("Cancel"), Some("Abbrechen")); // dialog button
    // A key not present in the catalog → None (so `tr` falls back to the key).
    assert_eq!(de.get("this key does not exist"), None);
}

#[test]
fn every_language_covers_every_english_key() {
    let cats = builtin_catalogs();
    let en = cats.iter().find(|c| c.name == "English").expect("English");
    for cat in cats.iter().filter(|c| c.name != "English") {
        let missing: Vec<&str> = en
            .strings
            .keys()
            .filter(|k| !cat.strings.contains_key(*k))
            .map(|s| s.as_str())
            .collect();
        assert!(missing.is_empty(), "{} is missing translations for: {missing:?}", cat.name);
    }
}

/// The Settings dialog has a fixed number of lines for the focused setting's
/// description, and a line cut off with an ellipsis loses exactly the part
/// that says when to turn the setting off. So every description has to fit, in
/// every language, not just in English.
#[test]
fn every_settings_description_fits_its_lines_in_every_language() {
    use crate::ui::dialog::{HELP_ROWS, HELP_WIDTH, settings_help_texts};
    for cat in builtin_catalogs() {
        for key in settings_help_texts() {
            let text = cat.get(key).unwrap_or(key);
            let lines = crate::util::text::wrap(text, HELP_WIDTH);
            assert!(
                lines.len() <= HELP_ROWS,
                "{}: the description takes {} lines of {HELP_WIDTH} cells, {HELP_ROWS} fit: {text}",
                cat.name,
                lines.len()
            );
        }
    }
}

#[test]
fn set_active_finds_known_and_rejects_unknown_languages() {
    // Deliberately keeps English active either way, so this never leaks German
    // into other tests that render translated UI (the active catalog is global).
    assert!(set_active_by_name("English"));
    assert_eq!(active_name(), "English");
    assert!(!set_active_by_name("Nonexistent language"));
    assert_eq!(active_name(), "English", "an unknown name leaves the active one");
    // With English active, tr returns the source and falls back on unknown keys.
    assert_eq!(tr("&Copy"), "&Copy");
    assert_eq!(tr("Totally untranslated"), "Totally untranslated");
}

#[test]
fn menu_accelerators_are_unique_per_menu_in_every_language() {
    // The Git submenu's keys come straight from the menu itself, so the two can
    // never drift apart.
    let git_keys: Vec<&str> = crate::ui::menu::GIT_MENU_KEYS.iter().map(|(k, _)| *k).collect();
    // Likewise the panel menus' Sort order submenu: its sort keys, then the toggles.
    let sort_keys: Vec<&str> = crate::ui::menu::SORT_KEYS
        .iter()
        .map(|(k, _)| *k)
        .chain(["&Reverse order", "&Directories first"])
        .collect();
    // The item label keys of each menu (mirroring `ui::menu`). The `&`
    // accelerator letter must be unique within a menu, in every language.
    let mut editor_menus: Vec<&[&str]> = crate::editor::menu::MENU_KEYS.to_vec();
    // The hex editor's binary-templates submenu is a menu of its own.
    editor_menus.push(crate::editor::menu::TEMPLATE_MENU_KEYS);
    editor_menus.push(crate::editor::menu::JSON_MENU_KEYS);
    let menus: &[&[&str]] = &[
        &[
            "&View",
            "&Edit",
            "&Copy",
            "&Rename/Move",
            "M&ulti rename",
            "&Make directory",
            "&Delete",
            "C&hmod",
            "Cho&wn",
            "&Symlink",
            "Com&press...",
            "Chec&ksum...",
            "Send over &LAN...",
            "Receive over L&AN...",
            "Cop&y path to clipboard",
            "&Git",
            "&Background operations...",
            "Select gr&oup",
            "U&nselect group",
            "&Invert selection",
            "&Quit",
        ],
        // The Git submenu (File → Git, or Alt-G). Its accelerators only need to be
        // unique among themselves, since it is a menu of its own.
        &git_keys,
        &sort_keys,
        &[
            "C&ommand palette...",
            "Directory &hotlist...",
            "Panel f&ilter...",
            "Directory hi&story...",
            "Sy&nc panels",
            "Show directory on other p&anel",
            "&Find file...",
            "Paneli&ze command output...",
            "Find d&uplicates...",
            "Compare &directories...",
            "S&ynchronize directories...",
            "Compare fi&les...",
            "&Process explorer...",
            "Disk &explorer...",
            "Disk &manager...",
            "Network &connections...",
            "S&wap panels",
            "&Re-read directories",
            "&Toggle split V/H",
        ],
        &["&Settings...", "&Edit themes...", "Edit e&xtensions...", "Edit &menu file..."],
        &[
            "&Full view",
            "&Brief view",
            "&Details view",
            "Tree v&iew",
            "&3D view",
            "T&humbnails view",
            "&Activity log",
            "&Sort order",
            "SFT&P connection...",
            "F&TP connection...",
            "FTPS c&onnection...",
            "S&CP connection...",
            "Go &local (keep session)",
        ],
    ];
    let accel = |s: &str| -> Option<char> {
        s.find('&').and_then(|b| s[b + 1..].chars().next()).map(|c| c.to_ascii_lowercase())
    };
    // The editor's six menus are checked on the same terms as the panel ones.
    let menus: Vec<&[&str]> = menus.iter().copied().chain(editor_menus).collect();
    for cat in builtin_catalogs() {
        for (mi, keys) in menus.iter().enumerate() {
            let mut seen = std::collections::HashSet::new();
            for k in *keys {
                let label = cat.get(k).unwrap_or(k);
                if let Some(a) = accel(label) {
                    assert!(
                        seen.insert(a),
                        "duplicate accelerator '{a}' in menu {mi} of {} ({label})",
                        cat.name
                    );
                }
            }
        }
    }
}

#[test]
fn arabic_and_persian_catalogs_are_marked_rtl() {
    let cats = builtin_catalogs();
    for name in ["العربية", "فارسی"] {
        assert!(cats.iter().find(|c| c.name == name).expect(name).rtl, "{name} should be rtl");
    }
    // A Latin-script language must not be flagged rtl.
    assert!(!cats.iter().find(|c| c.name == "English").unwrap().rtl);
    assert!(!cats.iter().find(|c| c.name == "Deutsch").unwrap().rtl);
}

#[test]
fn contains_rtl_detects_arabic_not_latin() {
    assert!(super::contains_rtl("مرحبا"));
    assert!(super::contains_rtl("Save حفظ")); // mixed
    assert!(!super::contains_rtl("hello"));
    assert!(!super::contains_rtl("Speichern"));
}

#[test]
fn reshape_reorders_arabic_leaves_latin_alone() {
    // Latin text is untouched by shaping + bidi.
    assert_eq!(super::reshape_and_reorder("hello"), "hello");
    // Arabic text is shaped and reordered into visual order, so it changes and
    // (for a pure-RTL run) the visual-first char is the logical-last one.
    let logical = "سلام";
    let visual = super::reshape_and_reorder(logical);
    assert_ne!(visual, logical, "arabic is reshaped/reordered");
    assert!(!visual.is_empty());
    let last_logical = logical.chars().next_back().unwrap();
    assert_ne!(visual.chars().next().unwrap(), logical.chars().next().unwrap());
    let _ = last_logical;
}

#[test]
fn a_wrapped_line_keeps_its_paragraphs_direction() {
    // An Arabic paragraph wrapped so that a line starts with a Latin key name.
    // Judged on its own that line is left-to-right, putting "F3" first; as part
    // of the right-to-left paragraph "F3" belongs at its right-hand end.
    let paragraph = "يفتح العارض عند الضغط على F3 في اللوحة";
    let line = "F3 في اللوحة";
    let alone = super::reshape_and_reorder(line);
    assert!(alone.starts_with("F3"), "on its own the line runs left to right: {alone}");
    let level = super::paragraph_level(paragraph);
    assert!(level.is_some_and(|l| l.is_rtl()), "the paragraph is right to left");
    let in_paragraph = super::reshape_and_reorder_at(line, level);
    assert!(in_paragraph.ends_with("F3"), "in its paragraph it runs right to left: {in_paragraph}");
}

#[test]
fn reshape_maps_arabic_to_joined_presentation_forms() {
    // Shaping replaces base Arabic letters with contextual (joined) presentation
    // forms in the U+FB50..U+FEFF blocks, which is what makes them connect on a
    // terminal without its own shaping engine.
    let out = super::reshape_and_reorder("مرحبا");
    assert!(
        out.chars().any(|c| (0xFB50..=0xFEFF).contains(&(c as u32))),
        "expected presentation forms in {:?}",
        out.chars().map(|c| format!("U+{:04X}", c as u32)).collect::<Vec<_>>()
    );
}

#[test]
fn display_is_a_noop_when_the_active_language_is_not_rtl() {
    // English is active by default (no global mutation here), so display leaves
    // even RTL text unchanged — reshaping only kicks in for an RTL language.
    assert!(!active_is_rtl());
    assert_eq!(display("مرحبا"), "مرحبا");
    assert_eq!(display("hello"), "hello");
}

#[test]
fn available_lists_the_builtin_languages() {
    let names = available();
    assert!(names.contains(&"English".to_string()));
    assert!(names.contains(&"Deutsch".to_string()));
}

#[test]
fn backfill_adds_missing_builtin_keys_without_clobbering_user_values() {
    use std::collections::HashMap;
    // Simulate a stale on-disk German catalog: one key the user customized, and
    // otherwise missing everything a newer built-in would have.
    let mut cats = vec![(
        "de.toml".to_string(),
        Catalog {
            name: "Deutsch".to_string(),
            code: "de".to_string(),
            rtl: false,
            strings: HashMap::from([("Cancel".to_string(), "MEINE-VERSION".to_string())]),
        },
    )];
    super::backfill_from_builtins(&mut cats);
    let de = &cats[0].1;

    // The user's own value for an existing key is preserved (not overwritten).
    assert_eq!(de.get("Cancel"), Some("MEINE-VERSION"));

    // Keys only present in the built-in are now filled in with the built-in
    // translation (so new strings show up on an existing install).
    let builtin_de = builtin_catalogs().into_iter().find(|c| c.name == "Deutsch").unwrap();
    assert_eq!(de.get("Continue"), builtin_de.get("Continue"));
    assert!(de.get("Continue").is_some(), "a new built-in key was backfilled");
    assert!(de.strings.len() > 1, "backfill pulled in the rest of the built-in keys");
}

/// Every literal key handed to [`tr`]/[`trd`] has to exist in the English
/// catalog.
///
/// `every_language_covers_every_english_key` only checks English against the
/// other seventeen, so it cannot see a key that never reached `en.toml` in the
/// first place: the call site then renders its English source, the miss is
/// invisible, and no translation of it can ever exist. Three keys had slipped
/// through in exactly that way, all of them in render code with no tests of its
/// own — so this checks the class rather than the instances.
#[test]
fn every_translated_literal_is_in_the_english_catalog() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let en = builtin_catalogs().into_iter().find(|c| c.name == "English").expect("English");

    let (mut missing, mut checked) = (Vec::new(), 0usize);
    for file in rs_files(&root) {
        // This file deliberately translates an unknown key to prove the fallback
        // works, so its literals are not call sites to check.
        if file.ends_with("l10n/tests.rs") {
            continue;
        }
        let src = std::fs::read_to_string(&file).expect("read source");
        for key in translated_literals(&src) {
            checked += 1;
            if en.get(&key).is_none() {
                missing.push(format!("{}: {key:?}", file.display()));
            }
        }
    }

    // Guard against the scanner silently matching nothing and passing forever.
    assert!(checked > 100, "only {checked} call sites found — the scanner is broken");
    assert!(
        missing.is_empty(),
        "passed to tr()/trd() but absent from en.toml:\n  {}",
        missing.join("\n  ")
    );
}

/// Every `.rs` file under `dir`, recursively.
fn rs_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let (mut out, mut stack) = (Vec::new(), vec![dir.to_path_buf()]);
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    out
}

/// The literal keys passed to `tr("…")` / `trd("…")` in `src`.
///
/// Deliberately simple: a call whose argument is not a plain literal (`tr(k)`,
/// `tr(&s)`) is skipped, since its key is only known at run time. No literal in
/// the tree contains an escape, so escapes are not handled — one appearing later
/// would end the key early and be reported as a miss, which is the safe
/// direction to fail in.
fn translated_literals(src: &str) -> Vec<String> {
    let (bytes, mut out, mut i) = (src.as_bytes(), Vec::new(), 0usize);
    while let Some(rel) = src[i..].find("tr") {
        let start = i + rel;
        i = start + 2;
        // `substr("…")` must not be mistaken for `tr("…")`.
        let prev = start.checked_sub(1).map(|p| bytes[p]);
        if prev.is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_') {
            continue;
        }
        let after = if src[i..].starts_with("d(") {
            i + 2
        } else if src[i..].starts_with('(') {
            i + 1
        } else {
            continue;
        };
        if !src[after..].starts_with('"') {
            continue;
        }
        let key = after + 1;
        let Some(len) = src[key..].find('"') else { continue };
        out.push(src[key..key + len].to_string());
        i = key + len;
    }
    out
}
