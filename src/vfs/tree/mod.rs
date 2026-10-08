//! Shared directory tree for container-backed VFS backends.
//!
//! Several backends expose a single *container* — an archive, an ISO image, a
//! git revision, a document — as a directory tree. They all face the same three
//! problems: a flat member list has to be grafted into directories, the result
//! has to be cached so listing a directory is not a re-parse, and `read_dir` and
//! `stat` must never disagree about what a name is.
//!
//! [`archive`](crate::vfs::archive) and [`extfs`](crate::vfs::extfs) each solved
//! those separately, and the two copies had drifted apart on the one question
//! that matters (see [`TreeBuilder::insert`]). This module is the single answer,
//! taking `archive`'s rule, which is the correct one.
//!
//! It is deliberately *not* a `trait ReadOnlyVfs` with a blanket `impl Vfs`: the
//! backends diverge exactly where such a trait would have to unify them — how
//! `open_read` gets its bytes, what `capabilities` says, and whether they are
//! writable at all. What they share is a data structure, so that is what this
//! module provides. Each backend still implements [`Vfs`](crate::vfs::Vfs).

use crate::util::{Error, Result};
use crate::vfs::{VfsEntry, VfsKind};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// One child within a directory of a mount.
///
/// `T` is whatever the backend needs to fetch this entry's content later — a
/// git blob id, an ISO extent, an index into a parsed document. Backends with
/// nothing to carry use `()`.
#[derive(Debug, Clone)]
pub struct Child<T> {
    pub name: String,
    pub kind: VfsKind,
    pub size: u64,
    pub mtime: Option<SystemTime>,
    pub mode: Option<u32>,
    pub symlink_target: Option<String>,
    pub payload: T,
}

/// The optional per-entry metadata handed to [`TreeBuilder::insert`]. Split out
/// so the common call — a plain file with nothing but a size — stays short.
#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub mtime: Option<SystemTime>,
    pub mode: Option<u32>,
    pub symlink_target: Option<String>,
}

impl Meta {
    pub fn mode(mode: u32) -> Self {
        Meta { mode: Some(mode), ..Default::default() }
    }
}

/// A built, read-only directory tree: inner dir path (`/`, `/a`, `/a/b`) → its
/// children.
#[derive(Debug)]
pub struct VfsTree<T> {
    dirs: HashMap<String, Vec<Child<T>>>,
    /// Stands in for the container's own timestamp, and is the fallback for any
    /// entry whose format recorded none.
    root_mtime: Option<SystemTime>,
}

impl<T> VfsTree<T> {
    #[allow(dead_code)] // consumed by the `iso` backend's directory walk
    pub fn is_dir(&self, inner: &str) -> bool {
        self.dirs.contains_key(&normalize(inner))
    }

    pub fn read_dir(&self, inner: &str) -> Result<Vec<VfsEntry>> {
        let norm = normalize(inner);
        let children = self.dirs.get(&norm).ok_or(Error::NotFound(norm))?;
        Ok(children.iter().map(|c| self.entry(c)).collect())
    }

    /// The entry for one path. The parent's child list is the single source of
    /// truth for what a name is, so this and [`read_dir`](Self::read_dir) can
    /// never disagree about it.
    pub fn stat(&self, inner: &str) -> Result<VfsEntry> {
        let norm = normalize(inner);
        if norm == "/" {
            return Ok(dir_entry(String::new(), self.root_mtime, None));
        }
        self.child(&norm).map(|c| self.entry(c))
    }

    /// The child record for one path, for backends that need more than a
    /// [`VfsEntry`] carries.
    pub fn child(&self, inner: &str) -> Result<&Child<T>> {
        let norm = normalize(inner);
        let parent = parent_inner(&norm);
        let name = base_name(&norm);
        self.dirs
            .get(&parent)
            .and_then(|c| c.iter().find(|c| c.name == name))
            .ok_or(Error::NotFound(norm))
    }

    /// The fetch payload for one path — how a backend turns a listed name back
    /// into bytes.
    #[allow(dead_code)] // the convenience form of `child`; used by backends' tests
    pub fn payload(&self, inner: &str) -> Result<&T> {
        self.child(inner).map(|c| &c.payload)
    }

    fn entry(&self, c: &Child<T>) -> VfsEntry {
        VfsEntry {
            name: c.name.clone(),
            kind: c.kind,
            size: c.size,
            // The entry's own timestamp when the format recorded one; otherwise
            // the container's, which at least dates the content plausibly.
            mtime: c.mtime.or(self.root_mtime),
            atime: None,
            ctime: None,
            btime: None,
            inode: None,
            mode: c.mode,
            uid: None,
            gid: None,
            symlink_target: c.symlink_target.clone(),
            symlink_broken: false,
            symlink_dir: false,
        }
    }
}

fn dir_entry(name: String, mtime: Option<SystemTime>, mode: Option<u32>) -> VfsEntry {
    VfsEntry {
        name,
        kind: VfsKind::Dir,
        size: 0,
        mtime,
        atime: None,
        ctime: None,
        btime: None,
        inode: None,
        mode,
        uid: None,
        gid: None,
        symlink_target: None,
        symlink_broken: false,
        symlink_dir: false,
    }
}

/// Builds a [`VfsTree`] from a flat member list.
pub struct TreeBuilder<T> {
    dirs: HashMap<String, Vec<Child<T>>>,
    /// Where each name sits in its directory's child list.
    ///
    /// Without this, grafting scans a directory's children to find the name it
    /// is about to insert, which is quadratic in the size of that directory —
    /// and one directory holding tens of thousands of files is an ordinary
    /// thing in a repository or a release tarball. The child lists stay vectors
    /// so a backend that reports its members in a meaningful order keeps it.
    pos: HashMap<String, HashMap<String, usize>>,
    root_mtime: Option<SystemTime>,
    entries: usize,
}

impl<T: Default> TreeBuilder<T> {
    pub fn new(root_mtime: Option<SystemTime>) -> Self {
        let mut dirs = HashMap::new();
        dirs.insert("/".to_string(), Vec::new());
        TreeBuilder { dirs, pos: HashMap::new(), root_mtime, entries: 0 }
    }

    /// Graft one member — given by its full path within the container — onto the
    /// tree, synthesizing every directory its path implies.
    ///
    /// **A name that appears as a directory anywhere stays a directory**, whether
    /// it was listed as one or merely has something living under it. Some archive
    /// writers store a folder as a zero-length *file* entry alongside its
    /// children, and some extfs scripts list a path before the directory holding
    /// it; treating such a name as a file would hide the whole subtree and would
    /// make `stat` and `read_dir` disagree about what it is.
    pub fn insert(&mut self, path: &str, kind: VfsKind, size: u64, meta: Meta, payload: T) {
        let norm = normalize(path);
        if norm == "/" {
            return;
        }
        let comps: Vec<&str> = norm[1..].split('/').collect();
        let last = comps.len() - 1;
        let mut meta = Some(meta);
        let mut payload = Some(payload);
        let mut parent = "/".to_string();
        for (i, comp) in comps.iter().enumerate() {
            let is_last = i == last;
            // Only the last component can be a non-directory; every prefix of a
            // path is a directory by construction.
            let as_dir = !is_last || kind.is_dir();
            let child_norm =
                if parent == "/" { format!("/{comp}") } else { format!("{parent}/{comp}") };

            let list = self.dirs.entry(parent.clone()).or_default();
            let at = self.pos.entry(parent.clone()).or_default().get(*comp).copied();
            let is_dir = match at.map(|i| &mut list[i]) {
                Some(existing) => {
                    if as_dir {
                        existing.kind = VfsKind::Dir;
                        existing.size = 0;
                    }
                    if is_last {
                        let m = meta.take().unwrap_or_default();
                        existing.mtime = m.mtime.or(existing.mtime);
                        existing.mode = m.mode.or(existing.mode);
                        if m.symlink_target.is_some() {
                            existing.symlink_target = m.symlink_target;
                        }
                        // Never demote a directory to the file some writer also
                        // recorded under the same name.
                        if !existing.kind.is_dir() {
                            existing.kind = kind;
                            existing.size = size;
                        }
                        // Adopt the fetch payload unless this is a file entry
                        // that a directory of the same name has suppressed.
                        if (as_dir || !existing.kind.is_dir())
                            && let Some(p) = payload.take()
                        {
                            existing.payload = p;
                        }
                    }
                    existing.kind.is_dir()
                }
                None => {
                    let m = if is_last { meta.take().unwrap_or_default() } else { Meta::default() };
                    let slot = self.pos.get_mut(&parent).expect("created above");
                    slot.insert((*comp).to_string(), list.len());
                    self.entries += 1;
                    list.push(Child {
                        name: (*comp).to_string(),
                        kind: if as_dir { VfsKind::Dir } else { kind },
                        size: if as_dir { 0 } else { size },
                        mtime: m.mtime,
                        mode: m.mode,
                        symlink_target: m.symlink_target,
                        payload: if is_last {
                            payload.take().unwrap_or_default()
                        } else {
                            T::default()
                        },
                    });
                    as_dir
                }
            };
            // Every directory gets a listing of its own, so `read_dir` succeeds
            // for exactly the names `stat` calls directories.
            if is_dir {
                self.dirs.entry(child_norm.clone()).or_default();
            }
            parent = child_norm;
        }
    }

    /// How many entries have been grafted so far, so a backend can stop parsing
    /// a container far larger than it is willing to hold.
    pub fn entry_count(&self) -> usize {
        self.entries
    }

    pub fn finish(self) -> VfsTree<T> {
        VfsTree { dirs: self.dirs, root_mtime: self.root_mtime }
    }
}

// ---------------------------------------------------------------------------
// Inner-path helpers. Inner paths are always POSIX and always absolute.
// ---------------------------------------------------------------------------

/// Normalize an inner path to `/`, `/a`, `/a/b` form — leading slash, no
/// trailing slash, no empty components, and the OS separator folded to `/` so a
/// `PathBuf` joined on Windows still addresses the same member.
pub fn normalize(inner: &str) -> String {
    let slashed = inner.replace('\\', "/");
    let mut out = String::with_capacity(slashed.len() + 1);
    for comp in slashed.split('/').filter(|c| !c.is_empty() && *c != ".") {
        out.push('/');
        out.push_str(comp);
    }
    if out.is_empty() { "/".to_string() } else { out }
}

/// The final component of a normalized inner path; empty for the root.
pub fn base_name(inner: &str) -> String {
    inner.rsplit('/').next().unwrap_or("").to_string()
}

/// The parent of a normalized inner path; the root is its own parent.
pub fn parent_inner(inner: &str) -> String {
    match inner.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => inner[..i].to_string(),
    }
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

/// Parsed trees, keyed by container path and guarded by a caller-chosen
/// validity stamp.
///
/// The stamp is what makes a cached tree still true: `archive` uses the
/// container's `(mtime, len)`, `extfs` its mtime alone, and a git revision needs
/// none at all — a commit tree is immutable — so it uses `()`.
pub struct TreeCache<S, T> {
    entries: Mutex<HashMap<PathBuf, Stamped<S, T>>>,
}

/// A cached tree and the stamp it was valid for.
type Stamped<S, T> = (S, Arc<VfsTree<T>>);

impl<S, T> Default for TreeCache<S, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S, T> TreeCache<S, T> {
    pub fn new() -> Self {
        TreeCache { entries: Mutex::new(HashMap::new()) }
    }

    /// Drop the cached tree for `key`, so the next listing rebuilds even if the
    /// filesystem's timestamps are too coarse to have noticed our own write.
    pub fn invalidate(&self, key: &Path) {
        self.entries.lock().unwrap().remove(key);
    }
}

impl<S: PartialEq + Clone, T> TreeCache<S, T> {
    /// The cached tree for `key` if its stamp still matches, otherwise the one
    /// `build` produces.
    ///
    /// `build` is a future rather than a closure because the backends differ:
    /// `archive` and `iso` parse on a blocking thread, `extfs` awaits a
    /// subprocess, and `git` awaits two. Whichever it is, the lock is never held
    /// across it.
    pub async fn get_or_build<F, Fut>(
        &self,
        key: &Path,
        stamp: S,
        build: F,
    ) -> Result<Arc<VfsTree<T>>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<VfsTree<T>>>,
    {
        if let Some(hit) = self.get(key, &stamp) {
            return Ok(hit);
        }
        let tree = Arc::new(build().await?);
        self.entries.lock().unwrap().insert(key.to_path_buf(), (stamp, tree.clone()));
        Ok(tree)
    }

    /// The cached tree for `key`, if one is present and its stamp matches.
    pub fn get(&self, key: &Path, stamp: &S) -> Option<Arc<VfsTree<T>>> {
        let entries = self.entries.lock().unwrap();
        entries.get(key).filter(|(s, _)| s == stamp).map(|(_, t)| t.clone())
    }
}

/// The `(mtime, len)` of a container, or `(None, 0)` if it cannot be read.
///
/// Both halves are compared: an mtime alone is not enough on a filesystem with
/// coarse timestamps, where two rebuilds a moment apart can land in the same
/// tick and the second would then be served from a stale tree.
pub async fn stamp_mtime_len(container: &Path) -> (Option<SystemTime>, u64) {
    match tokio::fs::metadata(container).await {
        Ok(m) => (m.modified().ok(), m.len()),
        Err(_) => (None, 0),
    }
}

/// The mtime of a container, or `None` if it cannot be read.
pub async fn stamp_mtime(container: &Path) -> Option<SystemTime> {
    tokio::fs::metadata(container).await.ok().and_then(|m| m.modified().ok())
}

#[cfg(test)]
mod tests;
