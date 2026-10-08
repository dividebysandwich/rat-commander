//! `git://` VFS backend — browse any revision of a repository as a directory.
//!
//! A container-backed backend in the shape of [`archive`](crate::vfs::archive)
//! and [`extfs`](crate::vfs::extfs), with one difference that makes it simpler
//! than either: **a commit's tree is immutable**, so a parsed listing never
//! needs invalidating. The others key their cache on the container's mtime; this
//! one needs no stamp at all.
//!
//! Everything comes from the `git` binary, which the rest of `src/git/` already
//! requires, so this adds no dependency and no new trust surface:
//!
//! * `git log --format=…`            → the revision list (the mount's root)
//! * `git ls-tree -r -l -z <oid>`    → one revision's whole tree, with sizes
//! * `git cat-file blob <oid>`       → one blob's bytes, streamed
//!
//! ## Path shape
//!
//! ```text
//! scheme    = "git"
//! container = <toplevel>/.git        a sentinel; commands run with -C its parent
//! path      = /                      the revision list
//!           = /<rev>                 that revision's tree
//!           = /<rev>/src/main.rs     a blob
//! ```
//!
//! The revision lives in the first path component because [`VfsPath`] has three
//! fields and no room for a fourth without touching every construction of it in
//! the tree. `container` carries the `.git` suffix so that `VfsPath::parent()`,
//! which leaves a container by way of `container.parent()`, lands `..` back in
//! the work tree rather than above the repository.
//!
//! History is **read-only**: every mutator returns
//! [`Unsupported`](crate::util::Error::Unsupported) rather than politely failing
//! later, so the dialogs refuse before any bytes move.

use crate::util::{Error, Result};
use crate::vfs::membuf::pipe_download;
use crate::vfs::tree::{self, Meta, TreeBuilder, TreeCache, VfsTree};
use crate::vfs::{BoxRead, BoxWrite, Capabilities, Vfs, VfsEntry, VfsKind, VfsPath, WriteMeta};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::process::Command;

/// Pipe capacity for streaming a blob out of `cat-file` (bounded memory, so a
/// huge blob in history costs a window rather than its whole length).
const PIPE_CAP: usize = 64 * 1024;

/// Refuse a tree larger than this rather than spend hundreds of megabytes on it.
const MAX_TREE_ENTRIES: usize = 200_000;

/// How much of a commit subject goes into a revision's directory name.
const SUBJECT_MAX: usize = 60;

/// What a listed name needs to fetch its bytes later: the blob's object id.
/// Directories are synthesized from the recursive listing and carry none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlobRef {
    pub oid: String,
}

/// One commit, as the revision list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rev {
    pub oid: String,
    pub short: String,
    /// Commit time, Unix seconds.
    pub time: i64,
    pub subject: String,
}

impl Rev {
    /// The directory name this revision is listed under, as
    /// `2026-09-12_13-45-02_a9ef3a7_Made-it-static`.
    ///
    /// Named timestamp-first so that **lexicographic order is chronological
    /// order** — the panel's default sort is by name, and a listing of commits
    /// that did not run in time order would be useless. The time is carried to
    /// the second, not just the day: most of a repository's commits share a date,
    /// and a day-resolution name would leave them sorted by their abbreviated
    /// object id, which is to say at random. Commits within the *same* second
    /// still tie, but those have no meaningful order to preserve.
    pub fn component(&self) -> String {
        let (y, mo, d, h, mi, sec) = crate::util::bytes::civil_parts(self.time);
        let subject = sanitize_subject(&self.subject);
        format!("{y:04}-{mo:02}-{d:02}_{h:02}-{mi:02}-{sec:02}_{}_{subject}", self.short)
    }

    pub fn mtime(&self) -> Option<SystemTime> {
        let t = u64::try_from(self.time).ok()?;
        SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(t))
    }

    /// How the panel border and title name this revision.
    #[allow(dead_code)] // shown on the panel border once the Git menu can mount one
    pub fn label(&self) -> String {
        format!("{} {}", self.short, self.subject)
    }
}

/// Reduce a commit subject to something that is safe as one path component and
/// short enough to read in a listing. Lossy on purpose: the abbreviated oid in
/// the same name is what actually identifies the commit.
fn sanitize_subject(subject: &str) -> String {
    let mut out = String::with_capacity(SUBJECT_MAX);
    let mut last_dash = false;
    for ch in subject.chars() {
        let keep = if ch.is_alphanumeric() { ch } else { '-' };
        if keep == '-' {
            // Runs of punctuation collapse rather than making `a---b`.
            if last_dash || out.is_empty() {
                last_dash = true;
                continue;
            }
            last_dash = true;
        } else {
            last_dash = false;
        }
        if out.chars().count() >= SUBJECT_MAX {
            break;
        }
        out.push(keep);
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// The abbreviated oid inside a revision directory name, which is all we ever
/// hand back to git. Sanitising the subject is therefore lossless for our
/// purposes: nothing is ever parsed back out of it.
///
/// The oid is the third underscore-separated field. Nothing before it contains
/// an underscore, and [`sanitize_subject`] leaves none in the subject either, so
/// the field index is exact rather than a guess.
fn oid_of_component(component: &str) -> Option<&str> {
    let oid = component.split('_').nth(2)?;
    let ok = !oid.is_empty() && oid.len() <= 64 && oid.chars().all(|c| c.is_ascii_hexdigit());
    ok.then_some(oid)
}

/// Split a `git://` inner path into its revision component and the path within
/// that revision. `/` (the revision list) yields `None`.
fn split_rev(inner: &str) -> Option<(String, String)> {
    let norm = tree::normalize(inner);
    if norm == "/" {
        return None;
    }
    let rest = &norm[1..];
    Some(match rest.find('/') {
        Some(i) => (rest[..i].to_string(), rest[i..].to_string()),
        None => (rest.to_string(), "/".to_string()),
    })
}

/// The work tree a `git://` container points at: `container` is `<toplevel>/.git`.
fn work_dir(container: &Path) -> Result<&Path> {
    container.parent().ok_or_else(|| Error::InvalidPath("not a git path".to_string()))
}

fn container_of(path: &VfsPath) -> Result<&PathBuf> {
    path.container.as_ref().ok_or_else(|| Error::InvalidPath("not a git path".to_string()))
}

/// Run a `git` command in `dir` and return its stdout, or a readable error.
async fn git_output(dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Error::other("git is not installed"),
            _ => Error::other(format!("git {}: {e}", args.first().copied().unwrap_or(""))),
        })?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let what = args.first().copied().unwrap_or("git");
        return Err(Error::other(if msg.is_empty() {
            format!("git {what} failed")
        } else {
            format!("git {what}: {msg}")
        }));
    }
    Ok(out.stdout)
}

/// Whether `dir` is inside a git repository, and the work tree's root if so.
#[allow(dead_code)] // the Git menu's entry point resolves a mount with this
pub async fn toplevel_of(dir: &Path) -> Option<PathBuf> {
    let out = git_output(dir, &["rev-parse", "--show-toplevel"]).await.ok()?;
    let text = String::from_utf8_lossy(&out).trim().to_string();
    if text.is_empty() {
        // A bare repository has no work tree; its own directory is the root.
        let out = git_output(dir, &["rev-parse", "--git-dir"]).await.ok()?;
        let g = String::from_utf8_lossy(&out).trim().to_string();
        return (!g.is_empty()).then(|| PathBuf::from(g));
    }
    Some(PathBuf::from(text))
}

/// The `container` for a repository rooted at `toplevel`.
///
/// The `.git` suffix is a sentinel rather than a path we ever open: it exists so
/// `VfsPath::parent()` leaves the mount at the work tree.
#[allow(dead_code)] // the Git menu's entry point builds a mount with this
pub fn container_for(toplevel: &Path) -> PathBuf {
    toplevel.join(".git")
}

/// Resolve a user-typed rev-spec (`HEAD~3`, a tag, a branch, an oid) to a
/// concrete commit. Done **before** a revision becomes a path component, so a
/// branch name containing `/` can never break path joining.
pub async fn resolve_rev(toplevel: &Path, spec: &str) -> Result<Rev> {
    let full = String::from_utf8_lossy(
        &git_output(toplevel, &["rev-parse", "--verify", "--quiet", &format!("{spec}^{{commit}}")])
            .await
            .map_err(|_| Error::other(format!("unknown revision '{spec}'")))?,
    )
    .trim()
    .to_string();
    if full.is_empty() {
        return Err(Error::other(format!("unknown revision '{spec}'")));
    }
    let out = git_output(toplevel, &["log", "-1", REV_FORMAT, &full]).await?;
    parse_rev_log(&String::from_utf8_lossy(&out))
        .into_iter()
        .next()
        .ok_or_else(|| Error::other(format!("unknown revision '{spec}'")))
}

const REV_FORMAT: &str = "--format=%H%x09%h%x09%at%x09%s";

/// List a repository's revisions, newest first.
pub async fn rev_list(toplevel: &Path, limit: usize) -> Result<Vec<Rev>> {
    let n = format!("-n{limit}");
    let out = git_output(toplevel, &["log", REV_FORMAT, &n]).await?;
    Ok(parse_rev_log(&String::from_utf8_lossy(&out)))
}

/// Parse `git log --format=%H%x09%h%x09%at%x09%s`.
///
/// A subject may contain anything at all, tabs included, so the split is bounded
/// to the three fields that precede it rather than done on every tab.
pub fn parse_rev_log(text: &str) -> Vec<Rev> {
    let mut revs = Vec::new();
    for line in text.lines() {
        let mut it = line.splitn(4, '\t');
        let (Some(oid), Some(short), Some(at)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        let Ok(time) = at.trim().parse::<i64>() else { continue };
        if oid.is_empty() || short.is_empty() {
            continue;
        }
        revs.push(Rev {
            oid: oid.to_string(),
            short: short.to_string(),
            time,
            subject: it.next().unwrap_or("").to_string(),
        });
    }
    revs
}

/// Parse `git ls-tree -r -l -z` into a tree.
///
/// Records are NUL-terminated and paths are **raw and unquoted** — which is why
/// `-z` is used rather than parsing `core.quotepath` output. Each record is
/// `<mode> <type> <oid> <size>\t<path>`; the split is at the **first** tab,
/// because the metadata half never contains one but a filename may.
pub fn parse_ls_tree_z(data: &[u8], mtime: Option<SystemTime>) -> Result<VfsTree<BlobRef>> {
    let mut b = TreeBuilder::<BlobRef>::new(mtime);
    for rec in data.split(|&c| c == 0) {
        if rec.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(rec);
        let Some(tab) = text.find('\t') else { continue };
        let (meta, path) = (&text[..tab], &text[tab + 1..]);
        let mut f = meta.split_whitespace();
        let (Some(mode), Some(kind), Some(oid)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        if path.is_empty() {
            continue;
        }
        // A gitlink's size column is `-`; a blob's is its byte length.
        let size = f.next().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);

        let (kind, meta) = match (kind, mode) {
            // A submodule. Its objects live in another repository, so it is
            // shown as an empty directory rather than descended into.
            ("commit", _) => (VfsKind::Dir, Meta::default()),
            (_, "120000") => (VfsKind::Symlink, Meta::default()),
            // The exec bit is what drives exec-first sort in the panel.
            (_, "100755") => (VfsKind::File, Meta::mode(0o755)),
            _ => (VfsKind::File, Meta::mode(0o644)),
        };
        b.insert(path, kind, size, meta, BlobRef { oid: oid.to_string() });
        if b.entry_count() > MAX_TREE_ENTRIES {
            return Err(Error::other(format!(
                "this revision holds more than {MAX_TREE_ENTRIES} files — too large to browse"
            )));
        }
    }
    Ok(b.finish())
}

/// One revision's files as `(path relative to the work tree, size)`.
///
/// The same `ls-tree` output the browsing backend parses, flattened instead of
/// grafted into a tree — which is the shape [`crate::sizes::from_paths`] wants.
/// Directories are not listed: `-r` reports only leaves, and the size tree
/// synthesizes the directories from the paths anyway.
pub fn parse_ls_tree_sizes(data: &[u8]) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    for rec in data.split(|&c| c == 0) {
        if rec.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(rec);
        let Some(tab) = text.find('\t') else { continue };
        let (meta, path) = (&text[..tab], &text[tab + 1..]);
        let mut f = meta.split_whitespace();
        let (Some(_mode), Some(kind), Some(_oid)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        // A submodule contributes no bytes of its own; its objects live in
        // another repository, and its size column is `-` rather than a number.
        if kind == "commit" || path.is_empty() {
            continue;
        }
        let Some(size) = f.next().and_then(|s| s.parse::<u64>().ok()) else { continue };
        out.push((path.to_string(), size));
    }
    out
}

/// Read one revision's files and their sizes, for the 3D timeline.
pub async fn tree_sizes(toplevel: &Path, oid: &str) -> Result<Vec<(String, u64)>> {
    let out = git_output(toplevel, &["ls-tree", "-r", "-l", "-z", oid]).await?;
    Ok(parse_ls_tree_sizes(&out))
}

/// A repository's history, mounted.
pub struct GitFs {
    /// Parsed revision trees, keyed by `<container>/<oid>`. A commit tree is
    /// immutable, so there is no stamp to validate and an entry never expires.
    trees: TreeCache<(), BlobRef>,
    /// The revision list, keyed by container. Rebuilt only when the mount is
    /// re-entered, since new commits do not appear under an open mount.
    revs: std::sync::Mutex<Vec<(PathBuf, Arc<Vec<Rev>>)>>,
    limit: usize,
}

impl GitFs {
    pub fn new(limit: usize) -> Self {
        GitFs { trees: TreeCache::new(), revs: std::sync::Mutex::new(Vec::new()), limit }
    }

    fn cached_revs(&self, container: &Path) -> Option<Arc<Vec<Rev>>> {
        let revs = self.revs.lock().unwrap();
        revs.iter().find(|(k, _)| k == container).map(|(_, v)| v.clone())
    }

    /// The revision list for a mount, fetched once per container.
    async fn revs(&self, container: &Path) -> Result<Arc<Vec<Rev>>> {
        if let Some(hit) = self.cached_revs(container) {
            return Ok(hit);
        }
        let dir = work_dir(container)?;
        let list = Arc::new(rev_list(dir, self.limit).await?);
        let mut revs = self.revs.lock().unwrap();
        if !revs.iter().any(|(k, _)| k == container) {
            revs.push((container.to_path_buf(), list.clone()));
        }
        Ok(list)
    }

    /// Find the revision a path component names.
    async fn rev_for(&self, container: &Path, component: &str) -> Result<Rev> {
        if let Some(r) = self.revs(container).await?.iter().find(|r| r.component() == component) {
            return Ok(r.clone());
        }
        // Not in the listed window (it is capped), but it may still be a real
        // commit — resolve the oid the component carries.
        let oid =
            oid_of_component(component).ok_or_else(|| Error::NotFound(component.to_string()))?;
        resolve_rev(work_dir(container)?, oid)
            .await
            .map_err(|_| Error::NotFound(component.to_string()))
    }

    /// The parsed tree for one revision.
    async fn tree(&self, container: &Path, rev: &Rev) -> Result<Arc<VfsTree<BlobRef>>> {
        let key = container.join(&rev.oid);
        let dir = work_dir(container)?;
        let mtime = rev.mtime();
        self.trees
            .get_or_build(&key, (), || async {
                let out = git_output(dir, &["ls-tree", "-r", "-l", "-z", &rev.oid]).await?;
                parse_ls_tree_z(&out, mtime)
            })
            .await
    }

    /// Resolve a path to the revision it names and the tree it lives in.
    /// `None` for the mount root, which is the revision list itself.
    async fn locate(&self, path: &VfsPath) -> Result<Option<(Rev, Arc<VfsTree<BlobRef>>, String)>> {
        let container = container_of(path)?;
        let inner = path.posix_path();
        let Some((component, sub)) = split_rev(&inner) else { return Ok(None) };
        let rev = self.rev_for(container, &component).await?;
        let tree = self.tree(container, &rev).await?;
        Ok(Some((rev, tree, sub)))
    }

    /// The revision list, as the mount root's listing.
    async fn list_revs(&self, container: &Path) -> Result<Vec<VfsEntry>> {
        Ok(self
            .revs(container)
            .await?
            .iter()
            .map(|r| VfsEntry {
                name: r.component(),
                kind: VfsKind::Dir,
                size: 0,
                mtime: r.mtime(),
                atime: None,
                ctime: None,
                btime: None,
                inode: None,
                mode: None,
                uid: None,
                gid: None,
                symlink_target: None,
                symlink_broken: false,
                symlink_dir: false,
            })
            .collect())
    }
}

#[async_trait::async_trait]
impl Vfs for GitFs {
    fn scheme(&self) -> &str {
        "git"
    }

    fn capabilities(&self) -> Capabilities {
        // History cannot be rewritten from a file manager, and saying so here is
        // what makes the dialogs refuse before any bytes move.
        Capabilities { symlinks: true, ..Capabilities::read_only() }
    }

    async fn read_dir(&self, dir: &VfsPath) -> Result<Vec<VfsEntry>> {
        match self.locate(dir).await? {
            None => self.list_revs(container_of(dir)?).await,
            Some((_, tree, sub)) => tree.read_dir(&sub),
        }
    }

    async fn stat(&self, path: &VfsPath) -> Result<VfsEntry> {
        match self.locate(path).await? {
            None => Ok(VfsEntry {
                name: String::new(),
                kind: VfsKind::Dir,
                size: 0,
                mtime: None,
                atime: None,
                ctime: None,
                btime: None,
                inode: None,
                mode: None,
                uid: None,
                gid: None,
                symlink_target: None,
                symlink_broken: false,
                symlink_dir: false,
            }),
            Some((rev, tree, sub)) if sub == "/" => {
                let mut e = tree.stat("/")?;
                e.name = rev.component();
                e.mtime = rev.mtime();
                Ok(e)
            }
            Some((_, tree, sub)) => tree.stat(&sub),
        }
    }

    async fn open_read(&self, path: &VfsPath) -> Result<BoxRead> {
        let Some((_, tree, sub)) = self.locate(path).await? else {
            return Err(Error::other("a revision list is not a file"));
        };
        let child = tree.child(&sub)?;
        if child.kind.is_dir() {
            return Err(Error::other(format!("\"{}\" is a directory", child.name)));
        }
        let oid = child.payload.oid.clone();
        let dir = work_dir(container_of(path)?)?.to_path_buf();
        // Fetched by object id rather than `<rev>:<path>`, which sidesteps every
        // pathspec quoting rule, and streamed rather than buffered so a very
        // large blob costs a pipe window instead of its own length.
        Ok(pipe_download(PIPE_CAP, move |mut w| async move {
            let mut ch = Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(["cat-file", "blob", &oid])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()?;
            let mut out =
                ch.stdout.take().ok_or_else(|| std::io::Error::other("git cat-file: no pipe"))?;
            tokio::io::copy(&mut out, &mut w).await?;
            let _ = ch.wait().await;
            Ok(())
        }))
    }

    async fn read_link(&self, path: &VfsPath) -> Result<String> {
        let Some((_, tree, sub)) = self.locate(path).await? else {
            return Err(Error::Unsupported);
        };
        let child = tree.child(&sub)?;
        if child.kind != VfsKind::Symlink {
            return Err(Error::other(format!("\"{}\" is not a symlink", child.name)));
        }
        // A link's target is the blob's contents. Read on demand rather than
        // during a listing, which would be one process per link.
        let dir = work_dir(container_of(path)?)?;
        let out = git_output(dir, &["cat-file", "blob", &child.payload.oid]).await?;
        Ok(String::from_utf8_lossy(&out).trim_end_matches('\n').to_string())
    }

    async fn open_write(&self, _path: &VfsPath, _meta: WriteMeta) -> Result<BoxWrite> {
        Err(Error::Unsupported)
    }
    async fn mkdir(&self, _path: &VfsPath) -> Result<()> {
        Err(Error::Unsupported)
    }
    async fn remove_file(&self, _path: &VfsPath) -> Result<()> {
        Err(Error::Unsupported)
    }
    async fn remove_dir(&self, _path: &VfsPath) -> Result<()> {
        Err(Error::Unsupported)
    }
    async fn rename(&self, _from: &VfsPath, _to: &VfsPath) -> Result<()> {
        Err(Error::Unsupported)
    }
}

#[cfg(test)]
mod tests;
