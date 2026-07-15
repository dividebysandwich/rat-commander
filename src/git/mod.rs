//! Lightweight Git integration: reads a directory's VCS status in the background
//! (branch, ahead/behind, and a per-file state for the entries shown in a panel)
//! and offers a couple of actions (stage/unstage, and reading the `HEAD` blob for
//! a diff). It shells out to the `git` CLI — the one external tool this feature
//! inherently needs — and degrades to "no git info" when git or a repo is absent.
//!
//! The parsing of `git status --porcelain` output is factored into pure functions
//! ([`parse_branch`], [`parse_status_z`]) so it can be unit-tested without a repo.

pub mod ops;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The VCS state of a single listing entry, in precedence order (a directory that
/// aggregates several nested changes keeps the highest-precedence one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitState {
    /// Untracked (`??`).
    Untracked,
    /// Staged in the index, with a clean worktree (`M `, `A `, …).
    Staged,
    /// Modified in the worktree (` M`, `MM`, `AM`, …), i.e. has unstaged changes.
    Modified,
    /// Unmerged / conflicted (`UU`, `AA`, `DD`, `AU`, …).
    Conflict,
}

impl GitState {
    /// Higher = more important; used to fold several nested states onto a
    /// containing directory.
    fn rank(self) -> u8 {
        match self {
            GitState::Untracked => 0,
            GitState::Staged => 1,
            GitState::Modified => 2,
            GitState::Conflict => 3,
        }
    }

    /// The single-character status glyph shown before the file name.
    pub fn glyph(self) -> char {
        match self {
            GitState::Untracked => '?',
            GitState::Staged => '+',
            GitState::Modified => '>',
            GitState::Conflict => '!',
        }
    }

    /// Whether this entry currently has changes staged in the index (so the
    /// stage/unstage toggle knows which way to go).
    pub fn is_staged(self) -> bool {
        matches!(self, GitState::Staged)
    }
}

/// A directory's Git status: the branch line plus a map from each immediate child
/// name (as shown in the panel) to its folded [`GitState`].
#[derive(Debug, Clone, Default)]
pub struct GitStatus {
    pub branch: String,
    pub ahead: usize,
    pub behind: usize,
    /// Keyed by the entry name as listed in the panel (an immediate child of the
    /// scanned directory).
    pub files: HashMap<String, GitState>,
    /// The repository root, for path-relative actions (stage/diff).
    pub root: PathBuf,
}

impl GitStatus {
    /// The state to show for the listing entry `name`, if any.
    pub fn state_of(&self, name: &str) -> Option<GitState> {
        self.files.get(name).copied()
    }

    /// A compact one-line branch label for the panel border, e.g.
    /// `main ↑2 ↓1` (arrows only when non-zero).
    pub fn branch_label(&self) -> String {
        let mut s = self.branch.clone();
        if self.ahead > 0 {
            s.push_str(&format!(" ↑{}", self.ahead));
        }
        if self.behind > 0 {
            s.push_str(&format!(" ↓{}", self.behind));
        }
        s
    }
}

/// Parse the porcelain `--branch` header (`## name...upstream [ahead N, behind M]`)
/// into `(branch, ahead, behind)`.
pub fn parse_branch(header: &str) -> (String, usize, usize) {
    // Strip the leading "## ".
    let body = header.strip_prefix("## ").unwrap_or(header);
    // Detached / initial states have no `...upstream`.
    if body.starts_with("HEAD (no branch)") {
        return ("HEAD (detached)".to_string(), 0, 0);
    }
    // "No commits yet on <branch>"
    if let Some(rest) = body.strip_prefix("No commits yet on ") {
        let name = rest.split_whitespace().next().unwrap_or("").to_string();
        return (name, 0, 0);
    }
    // The branch name is up to the first "..." (before the upstream) or the whole
    // token when there is no upstream.
    let name_end = body.find("...").unwrap_or_else(|| {
        body.find([' ', '[']).unwrap_or(body.len())
    });
    let branch = body[..name_end].trim().to_string();

    let (mut ahead, mut behind) = (0usize, 0usize);
    if let Some(open) = body.find('[')
        && let Some(close) = body[open..].find(']')
    {
        let inside = &body[open + 1..open + close];
        for part in inside.split(',') {
            let part = part.trim();
            if let Some(n) = part.strip_prefix("ahead ") {
                ahead = n.trim().parse().unwrap_or(0);
            } else if let Some(n) = part.strip_prefix("behind ") {
                behind = n.trim().parse().unwrap_or(0);
            }
        }
    }
    (branch, ahead, behind)
}

/// Map an `XY` porcelain code to a [`GitState`].
fn code_to_state(x: u8, y: u8) -> GitState {
    // Unmerged (conflict) combinations.
    let unmerged = matches!(
        (x, y),
        (b'U', _) | (_, b'U') | (b'D', b'D') | (b'A', b'A')
    );
    if unmerged {
        return GitState::Conflict;
    }
    if x == b'?' && y == b'?' {
        return GitState::Untracked;
    }
    // A worktree change (Y set) means there are unstaged modifications.
    if y != b' ' && y != 0 {
        return GitState::Modified;
    }
    // Otherwise it is staged (index change, clean worktree).
    GitState::Staged
}

/// Parse NUL-separated `git status --porcelain=v1 -z --branch` output (`bytes`)
/// into a [`GitStatus`], keeping only entries under `rel_dir` (the scanned
/// directory relative to the repo root) and folding them onto their immediate
/// child name. `root` is stored on the result for later path-relative actions.
pub fn parse_status_z(bytes: &[u8], rel_dir: &Path, root: PathBuf) -> GitStatus {
    let text = String::from_utf8_lossy(bytes);
    let fields: Vec<&str> = text.split('\0').collect();
    let mut status = GitStatus { root, ..Default::default() };

    let mut i = 0;
    while i < fields.len() {
        let f = fields[i];
        if f.is_empty() {
            i += 1;
            continue;
        }
        if let Some(rest) = f.strip_prefix("## ") {
            let (branch, ahead, behind) = parse_branch(&format!("## {rest}"));
            status.branch = branch;
            status.ahead = ahead;
            status.behind = behind;
            i += 1;
            continue;
        }
        if f.len() < 3 {
            i += 1;
            continue;
        }
        let bytes = f.as_bytes();
        let (x, y) = (bytes[0], bytes[1]);
        let path = &f[3..];
        // Rename/copy entries carry an extra NUL field (the source path) we skip.
        if x == b'R' || x == b'C' {
            i += 2;
        } else {
            i += 1;
        }
        let state = code_to_state(x, y);
        // `path` is relative to the repo root; keep only entries inside the
        // scanned directory and fold onto the immediate child.
        if let Some(child) = child_under(path, rel_dir) {
            status
                .files
                .entry(child)
                .and_modify(|s| {
                    if state.rank() > s.rank() {
                        *s = state;
                    }
                })
                .or_insert(state);
        }
    }
    status
}

/// Given a repo-root-relative `path` and the scanned directory `rel_dir` (also
/// repo-root-relative), return the immediate child name of `rel_dir` that `path`
/// falls under, or `None` if `path` is outside `rel_dir`. An entry directly in
/// `rel_dir` returns its own file name; a nested entry returns the containing
/// top-level subdirectory (so directories flag "contains changes").
fn child_under(path: &str, rel_dir: &Path) -> Option<String> {
    // Normalise a trailing slash git adds to untracked directories.
    let path = path.trim_end_matches('/');
    let p = Path::new(path);
    let rest = if rel_dir.as_os_str().is_empty() {
        p
    } else {
        p.strip_prefix(rel_dir).ok()?
    };
    let first = rest.components().next()?;
    Some(first.as_os_str().to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------------
// Subprocess wrappers (async). All return "no info" on any failure.
// ---------------------------------------------------------------------------

/// Read the Git status of `dir` (branch + per-child file states). `None` when
/// `dir` is not in a work tree, `git` is missing, or the command fails.
pub async fn status(dir: &Path) -> Option<GitStatus> {
    let root = repo_root(dir).await?;
    let rel_dir = dir.strip_prefix(&root).unwrap_or(Path::new("")).to_path_buf();
    let out = run(
        dir,
        &[
            "-c",
            "core.quotepath=false",
            "status",
            "--porcelain=v1",
            "--branch",
            "-z",
            "--",
            ".",
        ],
    )
    .await?;
    Some(parse_status_z(&out, &rel_dir, root))
}

/// The work-tree root containing `dir`, or `None`.
async fn repo_root(dir: &Path) -> Option<PathBuf> {
    let out = run(dir, &["rev-parse", "--show-toplevel"]).await?;
    let s = String::from_utf8_lossy(&out);
    let line = s.lines().next()?.trim();
    (!line.is_empty()).then(|| PathBuf::from(line))
}

/// Stage `name` (relative to `dir`) — `git add`.
pub async fn stage(dir: &Path, name: &str) -> Result<(), String> {
    run_ok(dir, &["add", "--", name]).await
}

/// Unstage `name` (relative to `dir`) — `git restore --staged`.
pub async fn unstage(dir: &Path, name: &str) -> Result<(), String> {
    run_ok(dir, &["restore", "--staged", "--", name]).await
}

/// The `HEAD` blob for `rel_path` (relative to the repo root), or `None` when the
/// file is new/untracked (no committed version) or on error.
pub async fn head_blob(root: &Path, rel_path: &Path) -> Option<Vec<u8>> {
    let spec = format!("HEAD:{}", rel_path.to_string_lossy());
    run(root, &["show", &spec]).await
}

/// Run `git -C <dir> <args>` and return stdout bytes on success (exit 0), else
/// `None`. Never panics; a missing `git` binary just yields `None`.
async fn run(dir: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("-C").arg(dir).args(args);
    cmd.stdin(std::process::Stdio::null());
    let out = cmd.output().await.ok()?;
    out.status.success().then_some(out.stdout)
}

/// Like [`run`] but for commands whose output we don't need; maps failure to an
/// error string (stderr).
async fn run_ok(dir: &Path, args: &[&str]) -> Result<(), String> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("-C").arg(dir).args(args);
    cmd.stdin(std::process::Stdio::null());
    let out = cmd.output().await.map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() { "git command failed".into() } else { err })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_branch_with_ahead_behind() {
        assert_eq!(
            parse_branch("## main...origin/main [ahead 2, behind 1]"),
            ("main".to_string(), 2, 1)
        );
        assert_eq!(parse_branch("## main...origin/main [ahead 3]"), ("main".to_string(), 3, 0));
        assert_eq!(parse_branch("## dev...origin/dev"), ("dev".to_string(), 0, 0));
        assert_eq!(parse_branch("## main"), ("main".to_string(), 0, 0));
        assert_eq!(parse_branch("## HEAD (no branch)"), ("HEAD (detached)".to_string(), 0, 0));
        assert_eq!(parse_branch("## No commits yet on trunk"), ("trunk".to_string(), 0, 0));
    }

    #[test]
    fn maps_porcelain_codes_to_states() {
        assert_eq!(code_to_state(b'?', b'?'), GitState::Untracked);
        assert_eq!(code_to_state(b'M', b' '), GitState::Staged); // staged, clean tree
        assert_eq!(code_to_state(b'A', b' '), GitState::Staged);
        assert_eq!(code_to_state(b' ', b'M'), GitState::Modified); // unstaged edit
        assert_eq!(code_to_state(b'M', b'M'), GitState::Modified); // staged + more edits
        assert_eq!(code_to_state(b'U', b'U'), GitState::Conflict);
        assert_eq!(code_to_state(b'A', b'A'), GitState::Conflict);
    }

    #[test]
    fn parses_status_at_repo_root() {
        // NUL-separated: branch header, then a few entries.
        let out = "## main...origin/main [ahead 1]\0 M src.rs\0?? new.txt\0M  staged.rs\0";
        let st = parse_status_z(out.as_bytes(), Path::new(""), PathBuf::from("/repo"));
        assert_eq!(st.branch, "main");
        assert_eq!(st.ahead, 1);
        assert_eq!(st.state_of("src.rs"), Some(GitState::Modified));
        assert_eq!(st.state_of("new.txt"), Some(GitState::Untracked));
        assert_eq!(st.state_of("staged.rs"), Some(GitState::Staged));
        assert_eq!(st.root, PathBuf::from("/repo"));
    }

    #[test]
    fn folds_nested_changes_onto_the_child_directory() {
        // Scanned dir is repo-root; a change deep under `sub/` flags `sub`.
        let out = "## main\0 M sub/deep/a.rs\0?? sub/deep/b.rs\0";
        let st = parse_status_z(out.as_bytes(), Path::new(""), PathBuf::from("/r"));
        // Highest-precedence state wins for the folded directory (Modified > Untracked).
        assert_eq!(st.state_of("sub"), Some(GitState::Modified));
    }

    #[test]
    fn scans_a_subdirectory_relative_to_root() {
        // Panel is at repo/src; paths are repo-root-relative.
        let out = "## main\0 M src/main.rs\0?? src/mod/new.rs\0 M other/x.rs\0";
        let st = parse_status_z(out.as_bytes(), Path::new("src"), PathBuf::from("/r"));
        assert_eq!(st.state_of("main.rs"), Some(GitState::Modified));
        assert_eq!(st.state_of("mod"), Some(GitState::Untracked)); // nested → dir
        assert_eq!(st.state_of("x.rs"), None, "entries outside the scanned dir are ignored");
    }

    #[test]
    fn untracked_directory_with_trailing_slash() {
        let out = "## main\0?? newdir/\0";
        let st = parse_status_z(out.as_bytes(), Path::new(""), PathBuf::from("/r"));
        assert_eq!(st.state_of("newdir"), Some(GitState::Untracked));
    }

    #[test]
    fn rename_entry_skips_its_source_field() {
        // "R  new\0old\0" then a normal entry that must still parse.
        let out = "## main\0R  new.rs\0old.rs\0?? z.txt\0";
        let st = parse_status_z(out.as_bytes(), Path::new(""), PathBuf::from("/r"));
        assert_eq!(st.state_of("new.rs"), Some(GitState::Staged));
        assert_eq!(st.state_of("z.txt"), Some(GitState::Untracked));
        assert_eq!(st.state_of("old.rs"), None, "the rename source is consumed, not mapped");
    }

    #[test]
    fn branch_label_shows_arrows_only_when_nonzero() {
        let mut st = GitStatus { branch: "main".into(), ..Default::default() };
        assert_eq!(st.branch_label(), "main");
        st.ahead = 2;
        assert_eq!(st.branch_label(), "main ↑2");
        st.behind = 3;
        assert_eq!(st.branch_label(), "main ↑2 ↓3");
    }

    // --- Real-repo integration (skips cleanly if `git` is unavailable) ---

    fn git_ok(dir: &Path, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Build a throwaway repo with a committed file, a staged addition, an
    /// unstaged modification and an untracked file. Returns `None` (skip) if git
    /// isn't usable here.
    fn make_repo() -> Option<PathBuf> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("rc_git_it_{}_{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).ok()?;
        if !git_ok(&dir, &["init", "-q"]) {
            let _ = std::fs::remove_dir_all(&dir);
            return None;
        }
        git_ok(&dir, &["config", "user.email", "t@example.com"]);
        git_ok(&dir, &["config", "user.name", "Test"]);
        std::fs::write(dir.join("tracked.txt"), b"one\n").ok()?;
        git_ok(&dir, &["add", "tracked.txt"]);
        git_ok(&dir, &["commit", "-qm", "init"]);
        // Modify the tracked file (unstaged), add a new staged file, and an
        // untracked file.
        std::fs::write(dir.join("tracked.txt"), b"one\ntwo\n").ok()?;
        std::fs::write(dir.join("added.txt"), b"new\n").ok()?;
        git_ok(&dir, &["add", "added.txt"]);
        std::fs::write(dir.join("untracked.txt"), b"stray\n").ok()?;
        Some(dir)
    }

    #[tokio::test]
    async fn status_reads_a_real_repo() {
        let Some(dir) = make_repo() else {
            eprintln!("git unavailable; skipping status_reads_a_real_repo");
            return;
        };
        let st = super::status(&dir).await.expect("a git status");
        assert!(!st.branch.is_empty(), "a branch name");
        assert_eq!(st.state_of("tracked.txt"), Some(GitState::Modified));
        assert_eq!(st.state_of("added.txt"), Some(GitState::Staged));
        assert_eq!(st.state_of("untracked.txt"), Some(GitState::Untracked));
        assert_eq!(st.root, std::fs::canonicalize(&dir).unwrap_or(dir.clone()));

        // Staging the modified file flips its state, and HEAD still has the old
        // content for a diff.
        super::stage(&dir, "tracked.txt").await.expect("stage");
        let st2 = super::status(&dir).await.expect("status after stage");
        assert_eq!(st2.state_of("tracked.txt"), Some(GitState::Staged));
        let head = super::head_blob(&st.root, Path::new("tracked.txt")).await.expect("HEAD blob");
        assert_eq!(head, b"one\n", "HEAD holds the committed version");

        // Unstaging restores the modified state.
        super::unstage(&dir, "tracked.txt").await.expect("unstage");
        let st3 = super::status(&dir).await.expect("status after unstage");
        assert_eq!(st3.state_of("tracked.txt"), Some(GitState::Modified));

        // A directory outside a repo has no status.
        let plain = std::env::temp_dir().join(format!("rc_nogit_{}", std::process::id()));
        std::fs::create_dir_all(&plain).unwrap();
        assert!(super::status(&plain).await.is_none(), "non-repo → None");

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&plain);
    }
}
