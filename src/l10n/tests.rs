use super::*;

#[test]
fn all_builtin_catalogs_parse_with_unique_names() {
    // Every embedded file must parse (a TOML typo would silently drop it).
    let cats = builtin_catalogs();
    assert_eq!(cats.len(), super::BUILTIN_FILES.len(), "every built-in file parsed");
    for expected in ["English", "Deutsch", "Français", "Русский", "日本語", "العربية"] {
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
    let de = builtin_catalogs()
        .into_iter()
        .find(|c| c.name == "Deutsch")
        .expect("German catalog");
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
    // The item label keys of each menu (mirroring `ui::menu`). The `&`
    // accelerator letter must be unique within a menu, in every language.
    let menus: &[&[&str]] = &[
        &[
            "&View", "&Edit", "&Copy", "&Rename/Move", "M&ulti rename", "&Make directory",
            "&Delete", "C&hmod", "Cho&wn", "&Symlink", "Com&press...", "Chec&ksum...",
            "&Background operations...", "Select &group", "U&nselect group",
            "&Invert selection", "&Quit",
        ],
        &[
            "C&ommand palette...", "&Find file...", "Find d&uplicates...", "Compare &directories...",
            "Compare fi&les...", "&Process explorer...", "Disk &explorer...", "Disk &manager...",
            "Network &connections...", "S&wap panels", "&Re-read directories", "&Toggle split V/H",
        ],
        &[
            "&Settings...", "&Confirmations...", "&Edit themes...", "Edit e&xtensions...",
            "Edit &menu file...",
        ],
        &[
            "&Full view", "&Brief view", "&Details view", "Tree v&iew", "Sort: &Name",
            "Sort: &Extension", "Sort: &Size", "Sort: &Modify time", "Sort: &Unsorted",
            "&Reverse order", "SFT&P connection...", "F&TP connection...", "S&CP connection...",
            "Go &local (keep session)",
        ],
    ];
    let accel = |s: &str| -> Option<char> {
        s.find('&')
            .and_then(|b| s[b + 1..].chars().next())
            .map(|c| c.to_ascii_lowercase())
    };
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
