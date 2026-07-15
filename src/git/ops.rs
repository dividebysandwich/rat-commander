//! The Git menu's actions: thin, uniform wrappers around the `git` CLI.
//!
//! Every action funnels through [`run_text`], which captures stdout **and**
//! stderr so the caller can show exactly what git said in the raw-output dialog
//! (git reports most of its useful chatter — "Switched to branch…", push
//! summaries, merge conflicts — on stderr). Commands are built as owned
//! `Vec<String>` argument lists so they can be moved into a background task: the
//! network ones (fetch/pull/push/clone) must not block the UI.
//!
//! [`RepoInfo`] reads the branches and remotes that populate the guided dialogs
//! (checkout's branch dropdown, push's remote dropdown); its parsing is factored
//! into pure functions so it is unit-testable without a repository.

use std::path::Path;

/// What a git invocation produced: whether it succeeded, and its combined output.
#[derive(Debug, Clone, Default)]
pub struct GitOutput {
    pub ok: bool,
    /// stdout followed by stderr, trimmed. Empty when git said nothing.
    pub text: String,
}

impl GitOutput {
    /// An immediate failure that never reached git (e.g. a bad selection).
    pub fn failed(msg: impl Into<String>) -> Self {
        GitOutput { ok: false, text: msg.into() }
    }
}

/// Run `git -C <dir> <args>`, capturing stdout+stderr. A missing `git` binary is
/// reported as a failed [`GitOutput`] rather than panicking.
pub async fn run_text(dir: &Path, args: &[String]) -> GitOutput {
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("-C").arg(dir).args(args);
    cmd.stdin(std::process::Stdio::null());
    let out = match cmd.output().await {
        Ok(o) => o,
        Err(e) => return GitOutput::failed(format!("cannot run git: {e}")),
    };
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    let err = String::from_utf8_lossy(&out.stderr);
    if !err.trim().is_empty() {
        if !text.trim().is_empty() {
            text.push('\n');
        }
        text.push_str(&err);
    }
    GitOutput { ok: out.status.success(), text: text.trim_end().to_string() }
}

/// Build an owned argument list from string-ish parts.
fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// Append `-- <names>` so file names can never be read as revisions or options.
fn with_paths(mut args: Vec<String>, names: &[String]) -> Vec<String> {
    args.push("--".into());
    args.extend(names.iter().cloned());
    args
}

// ---------------------------------------------------------------------------
// Argument builders. Each returns the argv for one action; the caller runs it
// through `run_text` on a background task.
// ---------------------------------------------------------------------------

pub fn init_args() -> Vec<String> {
    argv(&["init"])
}

/// `git clone <url> [dir]` — an empty `dir` lets git derive it from the URL.
pub fn clone_args(url: &str, dir: &str) -> Vec<String> {
    let mut a = argv(&["clone", "--progress", url]);
    if !dir.trim().is_empty() {
        a.push(dir.trim().to_string());
    }
    a
}

pub fn add_args(names: &[String]) -> Vec<String> {
    with_paths(argv(&["add"]), names)
}

pub fn unstage_args(names: &[String]) -> Vec<String> {
    with_paths(argv(&["restore", "--staged"]), names)
}

/// `git rm -r [--cached]` — `-r` so a selected directory can be removed too.
pub fn remove_args(names: &[String], cached: bool) -> Vec<String> {
    let mut a = argv(&["rm", "-r"]);
    if cached {
        a.push("--cached".into());
    }
    with_paths(a, names)
}

/// `git restore` — discard worktree changes. With `staged`, also reset the index.
pub fn restore_args(names: &[String], staged: bool) -> Vec<String> {
    let mut a = argv(&["restore"]);
    if staged {
        a.push("--staged".into());
        a.push("--worktree".into());
    }
    with_paths(a, names)
}

/// `git commit` with an optional `-a` (stage tracked changes) and `--amend`.
pub fn commit_args(message: &str, all: bool, amend: bool) -> Vec<String> {
    let mut a = argv(&["commit"]);
    if all {
        a.push("-a".into());
    }
    if amend {
        a.push("--amend".into());
    }
    a.push("-m".into());
    a.push(message.to_string());
    a
}

pub fn fetch_args(all: bool, prune: bool) -> Vec<String> {
    let mut a = argv(&["fetch", "--progress"]);
    if all {
        a.push("--all".into());
    }
    if prune {
        a.push("--prune".into());
    }
    a
}

pub fn pull_args(rebase: bool) -> Vec<String> {
    let mut a = argv(&["pull", "--progress"]);
    if rebase {
        a.push("--rebase".into());
    }
    a
}

/// `git push` with mutually-exclusive force flags — `--force-with-lease` is the
/// safe one and wins when both are ticked, since it refuses to clobber work that
/// arrived on the remote after the last fetch.
pub fn push_args(remote: &str, branch: &str, force: bool, lease: bool, upstream: bool) -> Vec<String> {
    let mut a = argv(&["push", "--progress"]);
    if lease {
        a.push("--force-with-lease".into());
    } else if force {
        a.push("--force".into());
    }
    if upstream {
        a.push("--set-upstream".into());
    }
    if !remote.trim().is_empty() {
        a.push(remote.trim().to_string());
        if !branch.trim().is_empty() {
            a.push(branch.trim().to_string());
        }
    }
    a
}

/// `git checkout` — with `create`, start a new branch at the current HEAD.
/// Checking out `origin/x` without `create` lands on a detached HEAD, so a plain
/// remote branch is checked out by its short name instead, which makes git set up
/// the tracking branch.
pub fn checkout_args(branch: &str, create: bool) -> Vec<String> {
    let mut a = argv(&["checkout"]);
    if create {
        a.push("-b".into());
        a.push(branch.trim().to_string());
        return a;
    }
    a.push(local_name_for(branch.trim()).to_string());
    a
}

/// The name to check out for `branch`: `origin/feature` → `feature`, so git
/// creates a tracking branch rather than detaching HEAD. Names that aren't a
/// `remote/branch` pair are returned unchanged.
pub fn local_name_for(branch: &str) -> &str {
    match branch.split_once('/') {
        // Only strip a leading remote segment; a local branch may legitimately
        // contain slashes (e.g. `feature/foo`), which we must not touch.
        Some((_, rest)) if branch.starts_with("origin/") || branch.starts_with("upstream/") => rest,
        _ => branch,
    }
}

/// `git reset --<mode> <target>`.
pub fn reset_args(mode: &str, target: &str) -> Vec<String> {
    let mut a = argv(&["reset"]);
    a.push(format!("--{mode}"));
    let t = target.trim();
    a.push(if t.is_empty() { "HEAD".to_string() } else { t.to_string() });
    a
}

pub fn status_args() -> Vec<String> {
    argv(&["status"])
}

/// A readable, bounded history: graph + one line per commit.
pub fn log_args() -> Vec<String> {
    argv(&[
        "log",
        "--graph",
        "--decorate",
        "--abbrev-commit",
        "-n",
        "200",
        "--pretty=format:%h %ad %an: %s",
        "--date=short",
    ])
}

// ---------------------------------------------------------------------------
// Repository info for the guided dialogs.
// ---------------------------------------------------------------------------

/// The branches and remotes shown in the checkout / push / reset dialogs.
#[derive(Debug, Clone, Default)]
pub struct RepoInfo {
    /// The checked-out branch (empty on a detached HEAD).
    pub current: String,
    /// Local branch names.
    pub local: Vec<String>,
    /// Remote-tracking branch names (`origin/main`), minus the `*/HEAD` aliases.
    pub remote: Vec<String>,
    /// Configured remote names (`origin`, `upstream`).
    pub remotes: Vec<String>,
}

impl RepoInfo {
    /// Branch choices for a checkout dropdown: the current branch first (so the
    /// dialog opens on it), then the other locals, then remote-tracking ones.
    pub fn checkout_choices(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(self.local.len() + self.remote.len());
        if !self.current.is_empty() {
            out.push(self.current.clone());
        }
        out.extend(self.local.iter().filter(|b| **b != self.current).cloned());
        out.extend(self.remote.iter().cloned());
        out
    }
}

/// Split newline-separated `git for-each-ref` output into trimmed names, dropping
/// blanks and the `origin/HEAD -> …` symbolic aliases.
pub fn parse_refs(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.contains("->") && !l.ends_with("/HEAD"))
        .map(str::to_string)
        .collect()
}

/// Read the branches and remotes of the repository containing `dir`. Fields are
/// individually best-effort: a repo with no commits still yields its remotes.
pub async fn repo_info(dir: &Path) -> RepoInfo {
    let text = |args: Vec<String>| async move {
        let o = run_text(dir, &args).await;
        if o.ok { o.text } else { String::new() }
    };
    let current = text(argv(&["rev-parse", "--abbrev-ref", "HEAD"])).await;
    let local = text(argv(&["for-each-ref", "--format=%(refname:short)", "refs/heads"])).await;
    let remote = text(argv(&["for-each-ref", "--format=%(refname:short)", "refs/remotes"])).await;
    let remotes = text(argv(&["remote"])).await;
    RepoInfo {
        // A detached HEAD reports the literal "HEAD"; treat that as "no branch".
        current: match current.trim() {
            "HEAD" | "" => String::new(),
            b => b.to_string(),
        },
        local: parse_refs(local.as_bytes()),
        remote: parse_refs(remote.as_bytes()),
        remotes: parse_refs(remotes.as_bytes()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_refs_drops_blanks_and_head_aliases() {
        let out = "main\nfeature/x\n\norigin/main\norigin/HEAD -> origin/main\n";
        assert_eq!(parse_refs(out.as_bytes()), vec!["main", "feature/x", "origin/main"]);
        assert!(parse_refs(b"").is_empty());
    }

    #[test]
    fn checkout_choices_lead_with_the_current_branch() {
        let info = RepoInfo {
            current: "dev".into(),
            local: vec!["main".into(), "dev".into()],
            remote: vec!["origin/main".into()],
            remotes: vec!["origin".into()],
        };
        // Current first, no duplicate of it, remotes last.
        assert_eq!(info.checkout_choices(), vec!["dev", "main", "origin/main"]);
    }

    #[test]
    fn local_name_strips_only_a_remote_prefix() {
        assert_eq!(local_name_for("origin/feature"), "feature");
        assert_eq!(local_name_for("upstream/main"), "main");
        // A local branch with slashes is left alone, as is a plain name.
        assert_eq!(local_name_for("feature/foo"), "feature/foo");
        assert_eq!(local_name_for("main"), "main");
    }

    #[test]
    fn push_prefers_force_with_lease_over_force() {
        let both = push_args("origin", "main", true, true, false);
        assert!(both.contains(&"--force-with-lease".to_string()));
        assert!(!both.contains(&"--force".to_string()), "lease must win over a raw force");
        let raw = push_args("origin", "main", true, false, false);
        assert!(raw.contains(&"--force".to_string()));
        // Plain push carries neither, and names the remote + branch.
        let plain = push_args("origin", "main", false, false, false);
        assert_eq!(plain, vec!["push", "--progress", "origin", "main"]);
        // Upstream tracking.
        let up = push_args("origin", "main", false, false, true);
        assert!(up.contains(&"--set-upstream".to_string()));
        // No remote → a bare `git push` (use whatever is configured).
        assert_eq!(push_args("", "", false, false, false), vec!["push", "--progress"]);
    }

    #[test]
    fn path_args_are_separated_by_a_double_dash() {
        let names = vec!["a.rs".to_string(), "-weird-name".to_string()];
        let a = add_args(&names);
        let dd = a.iter().position(|s| s == "--").expect("a -- separator");
        // Everything after `--` is a path, so a leading-dash name is never an option.
        assert_eq!(&a[dd + 1..], &["a.rs".to_string(), "-weird-name".to_string()]);
        assert!(remove_args(&names, true).contains(&"--cached".to_string()));
        assert!(remove_args(&names, false).contains(&"-r".to_string()));
    }

    #[test]
    fn commit_builds_message_and_flags() {
        assert_eq!(commit_args("msg", false, false), vec!["commit", "-m", "msg"]);
        let full = commit_args("fix", true, true);
        assert!(full.contains(&"-a".to_string()) && full.contains(&"--amend".to_string()));
        // The message stays a single argv entry even with spaces/newlines.
        let multi = commit_args("line one\nline two", false, false);
        assert_eq!(multi.last().unwrap(), "line one\nline two");
    }

    #[test]
    fn checkout_creates_or_switches() {
        assert_eq!(checkout_args("feat", true), vec!["checkout", "-b", "feat"]);
        assert_eq!(checkout_args("main", false), vec!["checkout", "main"]);
        // A remote branch is checked out by its short name (tracking, not detached).
        assert_eq!(checkout_args("origin/dev", false), vec!["checkout", "dev"]);
    }

    #[test]
    fn reset_defaults_to_head() {
        assert_eq!(reset_args("hard", ""), vec!["reset", "--hard", "HEAD"]);
        assert_eq!(reset_args("soft", "HEAD~2"), vec!["reset", "--soft", "HEAD~2"]);
    }

    #[test]
    fn clone_omits_an_empty_target_dir() {
        assert_eq!(clone_args("u://r", ""), vec!["clone", "--progress", "u://r"]);
        assert_eq!(clone_args("u://r", " dst "), vec!["clone", "--progress", "u://r", "dst"]);
    }

    // --- Real-repo integration (skips cleanly when `git` is unavailable) ---

    /// A throwaway repo with one commit on a known branch, plus a second branch.
    fn make_repo() -> Option<std::path::PathBuf> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("rc_gitops_{}_{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).ok()?;
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        };
        if !git(&["init", "-q", "-b", "main"]) {
            let _ = std::fs::remove_dir_all(&dir);
            return None;
        }
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "Test"]);
        std::fs::write(dir.join("f.txt"), b"x\n").ok()?;
        git(&["add", "f.txt"]);
        git(&["commit", "-qm", "init"]);
        git(&["branch", "dev"]);
        git(&["remote", "add", "origin", "https://example.invalid/r.git"]);
        Some(dir)
    }

    #[tokio::test]
    async fn repo_info_reads_branches_and_remotes() {
        let Some(dir) = make_repo() else {
            eprintln!("git unavailable; skipping repo_info_reads_branches_and_remotes");
            return;
        };
        let info = repo_info(&dir).await;
        assert_eq!(info.current, "main");
        assert!(info.local.contains(&"main".to_string()));
        assert!(info.local.contains(&"dev".to_string()));
        assert_eq!(info.remotes, vec!["origin"]);
        // The current branch leads the checkout list.
        assert_eq!(info.checkout_choices().first().map(String::as_str), Some("main"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn run_text_reports_success_and_failure_output() {
        let Some(dir) = make_repo() else {
            eprintln!("git unavailable; skipping run_text_reports_success_and_failure_output");
            return;
        };
        // A successful command with stdout.
        let out = run_text(&dir, &status_args()).await;
        assert!(out.ok, "status succeeds in a repo");
        assert!(out.text.contains("main"), "status names the branch: {}", out.text);

        // A failing command: the message comes back on stderr and ok is false.
        let out = run_text(&dir, &checkout_args("no-such-branch", false)).await;
        assert!(!out.ok, "checking out a missing branch fails");
        assert!(!out.text.is_empty(), "git's stderr is captured");

        // Checkout actually switches branches.
        let out = run_text(&dir, &checkout_args("dev", false)).await;
        assert!(out.ok, "checkout dev: {}", out.text);
        assert_eq!(repo_info(&dir).await.current, "dev");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn add_commit_roundtrip_changes_status() {
        let Some(dir) = make_repo() else {
            eprintln!("git unavailable; skipping add_commit_roundtrip_changes_status");
            return;
        };
        std::fs::write(dir.join("new.txt"), b"hi\n").unwrap();
        let names = vec!["new.txt".to_string()];
        assert!(run_text(&dir, &add_args(&names)).await.ok, "add");
        assert!(run_text(&dir, &commit_args("add new", false, false)).await.ok, "commit");
        // The tree is clean again, so the file no longer shows as untracked.
        let st = run_text(&dir, &status_args()).await;
        assert!(!st.text.contains("new.txt"), "committed file is no longer pending: {}", st.text);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
