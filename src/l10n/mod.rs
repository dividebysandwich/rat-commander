//! Localization: a small key→string catalog per language.
//!
//! Each language is a TOML file with a `name` (shown in Settings) and a
//! `[strings]` table mapping English source strings to their translation.
//! [`tr`] looks a key up in the active language and falls back to the key
//! itself (the English source) when it's missing, so a partial translation
//! still works and new keys degrade gracefully.
//!
//! On startup the built-in English + German catalogs are written into a `lang/`
//! subdirectory of the config directory (if not already there), and every
//! `*.toml` in that directory is discovered — so a user can edit a translation
//! or drop in a whole new language file and it appears in the Settings chooser.

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

/// Built-in catalogs, embedded so the files can be (re)generated and used as a
/// fallback even without a writable config directory. Each entry is the file
/// name written into `lang/` and its embedded contents. English comes first so
/// it is the default / listed first.
const BUILTIN_FILES: &[(&str, &str)] = &[
    ("en.toml", include_str!("en.toml")),
    ("de.toml", include_str!("de.toml")),
    ("fr.toml", include_str!("fr.toml")),
    ("es.toml", include_str!("es.toml")),
    ("pt.toml", include_str!("pt.toml")),
    ("nl.toml", include_str!("nl.toml")),
    ("cs.toml", include_str!("cs.toml")),
    ("sk.toml", include_str!("sk.toml")),
    ("hu.toml", include_str!("hu.toml")),
    ("sr.toml", include_str!("sr.toml")),
    ("uk.toml", include_str!("uk.toml")),
    ("ru.toml", include_str!("ru.toml")),
    ("ja.toml", include_str!("ja.toml")),
    ("zh-Hant.toml", include_str!("zh-Hant.toml")),
    ("zh-Hans.toml", include_str!("zh-Hans.toml")),
    ("hi.toml", include_str!("hi.toml")),
    ("fa.toml", include_str!("fa.toml")),
    ("ar.toml", include_str!("ar.toml")),
];

/// One language's translation table.
#[derive(Debug, Clone, Deserialize)]
pub struct Catalog {
    /// Display name (e.g. "English", "Deutsch"), shown in the Settings chooser.
    pub name: String,
    /// Short language code (informational).
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    strings: HashMap<String, String>,
}

impl Catalog {
    fn get(&self, key: &str) -> Option<&str> {
        self.strings.get(key).map(|s| s.as_str())
    }
}

fn builtin_catalogs() -> Vec<Catalog> {
    BUILTIN_FILES
        .iter()
        .filter_map(|(_, s)| toml::from_str::<Catalog>(s).ok())
        .collect()
}

/// All available languages (built-ins until [`load_languages`] discovers files).
static LANGS: LazyLock<RwLock<Vec<Catalog>>> = LazyLock::new(|| RwLock::new(builtin_catalogs()));
/// The active catalog (English by default).
static ACTIVE: LazyLock<RwLock<Catalog>> = LazyLock::new(|| {
    RwLock::new(builtin_catalogs().into_iter().next().unwrap_or_else(|| Catalog {
        name: "English".to_string(),
        code: "en".to_string(),
        strings: HashMap::new(),
    }))
});

/// Translate `key` into the active language. Unknown keys fall back to the key
/// itself (the English source string).
pub fn tr(key: &str) -> String {
    ACTIVE
        .read()
        .ok()
        .and_then(|c| c.get(key).map(|s| s.to_string()))
        .unwrap_or_else(|| key.to_string())
}

/// Names of all available languages, for the Settings chooser.
pub fn available() -> Vec<String> {
    LANGS
        .read()
        .map(|l| l.iter().map(|c| c.name.clone()).collect())
        .unwrap_or_default()
}

/// The active language's display name.
pub fn active_name() -> String {
    ACTIVE.read().map(|c| c.name.clone()).unwrap_or_default()
}

/// Switch the active language by display name (used by the live Settings
/// preview and on load). Returns whether a matching language was found.
pub fn set_active_by_name(name: &str) -> bool {
    let langs = LANGS.read().unwrap();
    if let Some(c) = langs.iter().find(|c| c.name == name) {
        *ACTIVE.write().unwrap() = c.clone();
        true
    } else {
        false
    }
}

/// Load languages at startup: generate the `lang/` directory (English + German)
/// if absent, discover every `*.toml` there, and set the active language to
/// `preferred` (by name), falling back to English.
pub fn load_languages(preferred: Option<&str>) {
    let discovered = ensure_and_discover();
    if !discovered.is_empty() {
        *LANGS.write().unwrap() = discovered;
    }
    let chosen = preferred.and_then(|name| set_active_by_name(name).then_some(()));
    if chosen.is_none() {
        // Default to the first available language (English).
        if let Some(c) = LANGS.read().unwrap().first().cloned() {
            *ACTIVE.write().unwrap() = c;
        }
    }
}

/// Ensure the `lang/` directory exists with the built-in files, then parse every
/// `*.toml` there. Falls back to the embedded built-ins if there's no config
/// directory or nothing parseable was found.
fn ensure_and_discover() -> Vec<Catalog> {
    let Some(dir) = crate::config::paths::lang_dir() else {
        return builtin_catalogs();
    };
    let _ = std::fs::create_dir_all(&dir);
    // Write the built-in files if absent (never clobber a user's edits).
    for (fname, body) in BUILTIN_FILES {
        let p = dir.join(fname);
        if !p.exists() {
            let _ = std::fs::write(&p, body);
        }
    }
    // Discover all *.toml files (en.toml first for a stable, English-first order).
    let mut cats: Vec<(String, Catalog)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let path = e.path();
            if path.extension().and_then(|s| s.to_str()) != Some("toml") {
                continue;
            }
            if let Some(cat) = std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| toml::from_str::<Catalog>(&s).ok())
            {
                let fname = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                cats.push((fname, cat));
            }
        }
    }
    if cats.is_empty() {
        return builtin_catalogs();
    }
    cats.sort_by(|a, b| {
        let rank = |n: &str| if n == "en.toml" { (0, String::new()) } else { (1, n.to_string()) };
        rank(&a.0).cmp(&rank(&b.0))
    });
    cats.into_iter().map(|(_, c)| c).collect()
}

#[cfg(test)]
mod tests;
