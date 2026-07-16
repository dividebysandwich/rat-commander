//! Temp-file helpers: one naming scheme for the program's scratch files, plus a
//! startup sweep that reclaims ones a previous run leaked.
//!
//! Most temp files are cleaned on the normal path (a viewer deletes its fetched
//! copy on close, the send server deletes its zip when the dialog closes). The
//! gap is an *in-flight* temp whose delivering event is dropped because the app
//! quit first — the file then lingers in `/tmp`. Naming every scratch path with a
//! common prefix lets [`sweep_stale`] find and remove those on the next launch.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// Shared prefix for every scratch file/dir this program creates in the system
/// temp directory.
const PREFIX: &str = "rc-tmp-";

/// Prefixes used before the unified scheme; still swept so leaks from an older
/// build are reclaimed once after upgrading.
const LEGACY_PREFIXES: &[&str] = &["rc_fetch_", "rc_extview_", "rc-send-"];

/// Only reclaim leaked temps older than this, so a concurrently-running second
/// instance's freshly-created scratch files are never touched.
const STALE_AGE: Duration = Duration::from_secs(60 * 60);

/// A unique, uncreated path in the system temp directory, named
/// `rc-tmp-<tag>-<pid>-<nanos>-<n>`. The caller creates and writes it. `tag` is a
/// short human hint (e.g. `"fetch"`, `"send"`) for anyone reading `ls /tmp`.
pub fn rc_temp_path(tag: &str) -> PathBuf {
    // pid + wall-clock + a process-local counter: unique even for two calls in
    // the same nanosecond, and across concurrent instances.
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("{PREFIX}{tag}-{}-{nanos}-{n}", std::process::id()))
}

/// Reclaim scratch files/dirs a previous run leaked. Best-effort and quiet: only
/// entries matching our naming scheme and older than [`STALE_AGE`] are removed,
/// so nothing a live instance is using is disturbed. Call once at startup, before
/// creating any temp of our own.
pub fn sweep_stale() {
    sweep_dir(&std::env::temp_dir(), STALE_AGE);
}

/// The sweep, parameterised for testing. Removes matching entries in `dir` whose
/// mtime is at least `min_age` old.
fn sweep_dir(dir: &Path, min_age: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let ours = name.starts_with(PREFIX) || LEGACY_PREFIXES.iter().any(|p| name.starts_with(p));
        if !ours {
            continue;
        }
        // Skip anything recent — it may belong to a running instance.
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age >= min_age);
        if !old {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            let _ = std::fs::remove_dir_all(&path);
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rc_temp_path_is_prefixed_unique_and_uncreated() {
        let a = rc_temp_path("fetch");
        let b = rc_temp_path("fetch");
        assert_ne!(a, b, "each call is unique");
        for p in [&a, &b] {
            let name = p.file_name().unwrap().to_str().unwrap();
            assert!(name.starts_with("rc-tmp-fetch-"), "carries the shared prefix: {name}");
            assert!(!p.exists(), "the helper does not create the file");
        }
    }

    #[test]
    fn sweep_removes_only_old_matching_entries() {
        // A private directory so the test never touches the real /tmp.
        let dir = rc_temp_path("sweeptest");
        std::fs::create_dir_all(&dir).unwrap();
        let old = SystemTime::now() - Duration::from_secs(3600);
        let touch = |name: &str, backdate: bool| {
            let p = dir.join(name);
            std::fs::write(&p, b"x").unwrap();
            if backdate {
                std::fs::OpenOptions::new().write(true).open(&p).unwrap().set_modified(old).unwrap();
            }
            p
        };
        let old_ours = touch("rc-tmp-fetch-1-2-3", true);
        let old_legacy = touch("rc_fetch_123_x", true);
        let fresh_ours = touch("rc-tmp-send-9-9-9", false);
        let old_other = touch("something-else.txt", true);
        // A whole matching directory (e.g. an extracted-archive temp) is removed
        // recursively.
        let old_dir = dir.join("rc-tmp-extract-1-2-3");
        std::fs::create_dir(&old_dir).unwrap();
        let inner = old_dir.join("inner");
        std::fs::write(&inner, b"y").unwrap();
        std::fs::OpenOptions::new().write(true).open(&inner).unwrap().set_modified(old).unwrap();
        std::fs::File::open(&old_dir).unwrap().set_modified(old).unwrap();

        // Sweep with a 30-minute threshold: the backdated entries qualify.
        sweep_dir(&dir, Duration::from_secs(1800));

        assert!(!old_ours.exists(), "an old rc-tmp- file is reclaimed");
        assert!(!old_legacy.exists(), "an old legacy-prefixed file is reclaimed");
        assert!(!old_dir.exists(), "an old matching directory is reclaimed recursively");
        assert!(fresh_ours.exists(), "a fresh rc-tmp- file (a live instance's) is left alone");
        assert!(old_other.exists(), "an unrelated file is never touched");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sweep_spares_recent_matching_entries() {
        let dir = rc_temp_path("sweepfresh");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("rc-tmp-fetch-1-2-3");
        std::fs::write(&p, b"x").unwrap();
        // A 1-hour threshold against a just-created file: nothing is removed.
        sweep_dir(&dir, Duration::from_secs(3600));
        assert!(p.exists(), "a recent temp is never swept");
        std::fs::remove_dir_all(&dir).ok();
    }
}
