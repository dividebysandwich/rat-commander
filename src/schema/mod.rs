//! JSON Schema validation for the JSON, YAML and TOML files the editor checks.
//!
//! The schema a file is validated against is, in this order: the one it names
//! itself (`"$schema"` in JSON, a `# yaml-language-server: $schema=…` comment
//! in YAML, `#:schema …` in TOML); the one the user's
//! `schemas/schemas.toml` maps its path to; or the bundled one for a
//! well-known configuration file (see `assets/schemas/catalog.toml`). A schema
//! is found by URL among the bundled ones, or read from a file; nothing is
//! ever fetched from the network, and a `$ref` to anything else is taken as
//! allowing anything.

mod bundle;
mod value;

use crate::lint::{Diagnostic, Lang};
use jsonschema::error::ValidationErrorKind;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

/// Files larger than this aren't validated.
const MAX_BYTES: usize = 4 << 20;
/// Most schema errors reported for a file.
const MAX_ERRORS: usize = 200;
/// Longest message shown: some quote a whole object back.
const MAX_MESSAGE: usize = 200;

/// Where a schema comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Schema {
    /// A bundled one, by file name.
    Bundled(&'static str),
    File(PathBuf),
}

/// The user's own mapping of files to schemas.
#[derive(serde::Deserialize, Default)]
struct UserMap {
    #[serde(default)]
    map: Vec<Mapping>,
}

#[derive(serde::Deserialize)]
struct Mapping {
    files: Vec<String>,
    /// A schema file (relative to the schemas directory), a bundled schema's
    /// URL, or `none`.
    schema: String,
}

/// `~/.config/rat-commander/schemas/` — none in tests, which must not depend
/// on a developer's own mapping.
pub fn user_dir() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    crate::config::paths::config_dir().map(|d| d.join("schemas"))
}

/// The schema a document names in itself.
fn named_in(lang: Lang, text: &str, doc: &value::Doc) -> Option<String> {
    let comment = |prefix: &str| {
        text.lines().take(20).find_map(|l| {
            let rest = l.trim().strip_prefix('#')?;
            rest.trim_start().strip_prefix(prefix).map(|s| s.trim().to_string())
        })
    };
    match lang {
        Lang::Json(_) => doc.value.get("$schema")?.as_str().map(str::to_string),
        Lang::Yaml => comment("yaml-language-server: $schema="),
        Lang::Toml => text
            .lines()
            .take(20)
            .find_map(|l| l.trim().strip_prefix("#:schema").map(|s| s.trim().to_string())),
        Lang::Xml => None,
    }
}

/// Where the reference `r` — a URL or a path, relative to `dir` — leads.
fn resolve(r: &str, dir: Option<&Path>) -> Option<Schema> {
    if let Some(k) = bundle::by_url(r) {
        return Some(Schema::Bundled(k.file.as_str()));
    }
    if let Some(p) = r.strip_prefix("file://") {
        return Some(Schema::File(PathBuf::from(p)));
    }
    if r.contains("://") {
        return None;
    }
    let p = crate::vfs::remote::sshconfig::expand_tilde(r);
    let p = if p.is_absolute() { p } else { dir?.join(p) };
    p.is_file().then_some(Schema::File(p))
}

fn glob_matches(pattern: &str, path: &Path) -> bool {
    globset::GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .is_ok_and(|g| g.compile_matcher().is_match(path))
}

/// The schema for the document `doc` in `text`, at `path` (a local file).
fn choose(lang: Lang, text: &str, doc: &value::Doc, path: Option<&Path>) -> Option<Schema> {
    let dir = path.and_then(Path::parent);
    if let Some(r) = named_in(lang, text, doc) {
        return resolve(&r, dir);
    }
    let path = path?;
    if let Some(udir) = user_dir()
        && let Ok(t) = std::fs::read_to_string(udir.join("schemas.toml"))
        && let Ok(m) = toml::from_str::<UserMap>(&t)
        && let Some(mapping) = m.map.iter().find(|m| m.files.iter().any(|g| glob_matches(g, path)))
    {
        return match mapping.schema.as_str() {
            "none" => None,
            s => resolve(s, Some(&udir)),
        };
    }
    bundle::catalog()
        .iter()
        .find(|k| k.files.iter().any(|g| glob_matches(g, path)))
        .map(|k| Schema::Bundled(k.file.as_str()))
}

/// Finds what a schema refers to: bundled schemas by URL and local files; for
/// anything else, a schema that allows everything.
struct Retriever;

impl jsonschema::Retrieve for Retriever {
    fn retrieve(
        &self,
        uri: &jsonschema::Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let s = uri.as_str();
        if let Some(bytes) = bundle::by_url(s).and_then(|k| bundle::get(&k.file)) {
            return Ok(serde_json::from_slice(bytes)?);
        }
        if let Some(p) = s.strip_prefix("file://") {
            return Ok(serde_json::from_str(&std::fs::read_to_string(p)?)?);
        }
        Ok(Value::Bool(true))
    }
}

type Compiled = Result<Arc<jsonschema::Validator>, String>;

/// Validators built so far, by schema (and a file's modification time).
static VALIDATORS: LazyLock<Mutex<HashMap<String, Compiled>>> = LazyLock::new(Default::default);

fn validator(schema: &Schema) -> Compiled {
    let key = match schema {
        Schema::Bundled(f) => format!("bundled:{f}"),
        Schema::File(p) => {
            let mtime = std::fs::metadata(p).and_then(|m| m.modified()).ok();
            format!("{}@{mtime:?}", p.display())
        }
    };
    if let Some(v) = VALIDATORS.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return v.clone();
    }
    let raw: Result<Value, String> = match schema {
        Schema::Bundled(f) => bundle::get(f)
            .ok_or_else(|| format!("{f} isn't bundled"))
            .and_then(|b| serde_json::from_slice(b).map_err(|e| e.to_string())),
        Schema::File(p) => std::fs::read_to_string(p)
            .map_err(|e| format!("{}: {e}", p.display()))
            .and_then(|t| serde_json::from_str(&t).map_err(|e| format!("{}: {e}", p.display()))),
    };
    let compiled = raw.and_then(|raw| {
        jsonschema::options()
            .with_retriever(Retriever)
            .build(&raw)
            .map(Arc::new)
            .map_err(|e| e.to_string())
    });
    VALIDATORS.lock().unwrap_or_else(|e| e.into_inner()).insert(key, compiled.clone());
    compiled
}

/// `name` as one step of a JSON pointer.
fn step(name: &str) -> String {
    name.replace('~', "~0").replace('/', "~1")
}

/// Where an error is shown: at the key of what it is about when it is about
/// an object's members, else at the value — the nearest part of the document
/// that has a place, if that one has none.
fn place(text: &str, doc: &value::Doc, e: &jsonschema::ValidationError) -> Diagnostic {
    let pointer = e.instance_path().as_str().to_string();
    let (target, at_key) = match e.kind() {
        ValidationErrorKind::AdditionalProperties { unexpected }
        | ValidationErrorKind::UnevaluatedProperties { unexpected } => match unexpected.first() {
            Some(u) => (format!("{pointer}/{}", step(u)), true),
            None => (pointer, true),
        },
        ValidationErrorKind::Required { .. }
        | ValidationErrorKind::MinProperties { .. }
        | ValidationErrorKind::MaxProperties { .. }
        | ValidationErrorKind::PropertyNames { .. } => (pointer, true),
        _ => (pointer, false),
    };
    let mut p = target.as_str();
    let found = loop {
        if let Some(pl) = doc.places.get(p) {
            break Some(pl);
        }
        match p.rfind('/') {
            Some(i) => p = &p[..i],
            None => break None,
        }
    };
    let span = match found {
        Some(pl) if at_key || p != target => pl.key.clone().unwrap_or_else(|| pl.value.clone()),
        Some(pl) => pl.value.clone(),
        None => 0..1,
    };
    let mut message = e.to_string();
    if message.chars().count() > MAX_MESSAGE {
        message = message.chars().take(MAX_MESSAGE).collect::<String>() + "…";
    }
    Diagnostic { span: crate::json::clip_to_line(text, span), message }
}

/// The schema errors in `text`, a `lang` document at `path` (when it is a
/// local file), in text order; none when it has no schema, has syntax errors,
/// or is too large.
pub fn validate(lang: Lang, text: &str, path: Option<&Path>) -> Vec<Diagnostic> {
    if text.len() > MAX_BYTES {
        return Vec::new();
    }
    let doc = match lang {
        Lang::Json(opts) => value::json(text, opts),
        Lang::Yaml => value::yaml(text),
        Lang::Toml => value::toml(text),
        Lang::Xml => None,
    };
    let Some(doc) = doc else { return Vec::new() };
    let Some(schema) = choose(lang, text, &doc, path) else { return Vec::new() };
    let v = match validator(&schema) {
        Ok(v) => v,
        Err(e) => {
            let message = format!("The schema can't be used: {e}");
            return vec![Diagnostic { span: crate::json::clip_to_line(text, 0..1), message }];
        }
    };
    let mut diags: Vec<Diagnostic> =
        v.iter_errors(&doc.value).take(MAX_ERRORS).map(|e| place(text, &doc, &e)).collect();
    diags.sort_by_key(|d| d.span.start);
    diags
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str, diags: &[Diagnostic]) -> Vec<(usize, String)> {
        diags
            .iter()
            .map(|d| (text[..d.span.start].matches('\n').count(), d.message.clone()))
            .collect()
    }

    #[test]
    fn well_known_files_get_their_bundled_schema_by_path() {
        let doc = value::Doc::default();
        let at = |p: &str| choose(Lang::Yaml, "", &doc, Some(Path::new(p)));
        assert_eq!(
            at("/home/u/repo/.github/workflows/ci.yml"),
            Some(Schema::Bundled("github-workflow.json"))
        );
        assert_eq!(
            at("/srv/app/docker-compose.prod.yaml"),
            Some(Schema::Bundled("compose-spec.json"))
        );
        assert_eq!(at("/srv/app/.github/ci.yml"), None, "not in workflows/");
        assert_eq!(at("/srv/app/values.yaml"), None);
        let toml = choose(Lang::Toml, "", &doc, Some(Path::new("/x/Cargo.toml")));
        assert_eq!(toml, Some(Schema::Bundled("cargo.json")));
    }

    #[test]
    fn a_schema_named_in_the_file_wins_and_is_found_beside_it() {
        let dir = std::env::temp_dir().join(format!("rc_schema_named_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("thing.schema.json"),
            r#"{"type": "object", "required": ["name"], "properties": {"$schema": {"type": "string"}, "size": {"type": "integer"}}, "additionalProperties": false}"#,
        )
        .unwrap();
        let text = "{\n  \"$schema\": \"./thing.schema.json\",\n  \"size\": \"big\",\n  \"colour\": 1\n}\n";
        let path = dir.join("data.json");
        let f = lines(text, &validate(Lang::Json(Default::default()), text, Some(&path)));
        assert_eq!(f.len(), 3, "{f:?}");
        assert_eq!(f[0].0, 0, "a missing member is shown at the object: {f:?}");
        assert!(f.iter().any(|(l, m)| *l == 2 && m.contains("integer")), "{f:?}");
        assert!(
            f.iter().any(|(l, m)| *l == 3 && m.contains("colour")),
            "at the unexpected key: {f:?}"
        );
        // A document naming a schema that can't be found isn't validated.
        let other = "{\"$schema\": \"https://example.invalid/x.json\", \"a\": 1}";
        assert!(validate(Lang::Json(Default::default()), other, Some(&path)).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_github_workflow_is_checked_against_the_bundled_schema() {
        let path = Path::new("/r/.github/workflows/ci.yml");
        let good = "on: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n";
        assert_eq!(lines(good, &validate(Lang::Yaml, good, Some(path))), vec![]);
        let bad = "on: push\njobs:\n  build:\n    runs_on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n";
        let f = lines(bad, &validate(Lang::Yaml, bad, Some(path)));
        assert!(!f.is_empty(), "runs_on isn't runs-on");
        assert!(f.iter().all(|(l, _)| *l <= 3), "{f:?}");
    }

    #[test]
    fn cargo_toml_is_checked_with_its_places() {
        let path = Path::new("/r/Cargo.toml");
        let text = "[package]\nname = \"x\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nserde = { version = 1 }\n";
        let f = lines(text, &validate(Lang::Toml, text, Some(path)));
        assert!(f.iter().any(|(l, _)| *l == 6), "the numeric version is on line 7: {f:?}");
    }
}
