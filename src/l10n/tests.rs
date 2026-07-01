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
            "&Delete", "C&hmod", "Cho&wn", "&Symlink", "Com&press...", "Select &group",
            "U&nselect group", "&Invert selection", "&Quit",
        ],
        &[
            "&Find file...", "Find d&uplicates...", "Compare &directories...", "Compare fi&les...",
            "&Process explorer...", "Disk &explorer...", "Disk &manager...", "S&wap panels",
            "&Re-read directories", "&Toggle split V/H",
        ],
        &["&Settings...", "&Confirmations...", "&Edit themes..."],
        &[
            "&Full view", "&Brief view", "&Details view", "Sort: &Name", "Sort: &Extension",
            "Sort: &Size", "Sort: &Modify time", "Sort: &Unsorted", "&Reverse order",
            "SFT&P connection...", "F&TP connection...", "S&CP connection...", "Disconnect (&local)",
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
fn available_lists_the_builtin_languages() {
    let names = available();
    assert!(names.contains(&"English".to_string()));
    assert!(names.contains(&"Deutsch".to_string()));
}
