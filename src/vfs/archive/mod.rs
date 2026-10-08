//! `archive://` VFS backend — browse and edit archives like directories.
//!
//! Every mutation rebuilds the whole container (that is what the archive
//! formats allow), so this module has two layers:
//!
//! * the [`Vfs`] implementation, where `mkdir` / `rename` / `remove_*` /
//!   `open_write` each rebuild once, so the generic ops engine, the panels and
//!   the dialogs treat an archive exactly like any other backend; and
//! * the bulk free functions at the bottom (`create_archive`, `add_to_archive`,
//!   `remove_from_archive`), which `AppState` uses for whole-selection
//!   operations so dropping fifty files into a zip is *one* rebuild rather than
//!   fifty.

mod cpio;
mod deb;
pub mod formats;
mod rpm;

use crate::util::{Error, Result};
use crate::vfs::{BoxRead, BoxWrite, Capabilities, Vfs, VfsEntry, VfsKind, VfsPath, WriteMeta};
use formats::{ArchiveFormat, FullEntry, RawEntry, normalize};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::SystemTime;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// In-memory directory tree built from an archive's member list.
struct ChildMeta {
    name: String,
    kind: VfsKind,
    size: u64,
    mtime: Option<SystemTime>,
    mode: Option<u32>,
}

/// Identity of the container file a tree was built from. Both halves are
/// compared: an mtime alone is not enough on a filesystem with coarse
/// timestamps, where two rebuilds a moment apart can land in the same tick.
type Stamp = (Option<SystemTime>, u64);

struct ArchiveTree {
    format: ArchiveFormat,
    stamp: Stamp,
    /// The container file's own mtime, used for the archive root and as the
    /// fallback for members whose format records no timestamp.
    mtime: Option<SystemTime>,
    /// Inner dir path (`/a`) -> its children.
    dirs: HashMap<String, Vec<ChildMeta>>,
}

impl ArchiveTree {
    fn read_dir(&self, inner: &str) -> Result<Vec<VfsEntry>> {
        let norm = normalize(inner);
        let children = self.dirs.get(&norm).ok_or_else(|| Error::NotFound(norm.clone()))?;
        Ok(children.iter().map(|c| self.entry(c)).collect())
    }

    fn stat(&self, inner: &str) -> Result<VfsEntry> {
        let norm = normalize(inner);
        if norm == "/" {
            return Ok(dir_entry(String::new(), self.mtime, None));
        }
        // The parent's child list is the single source of truth for what a name
        // is, so `stat` and `read_dir` can never disagree about it.
        let parent = parent_inner(&norm);
        let name = base_name(&norm);
        self.dirs
            .get(&parent)
            .and_then(|c| c.iter().find(|c| c.name == name))
            .map(|c| self.entry(c))
            .ok_or(Error::NotFound(norm))
    }

    fn entry(&self, c: &ChildMeta) -> VfsEntry {
        VfsEntry {
            name: c.name.clone(),
            kind: c.kind,
            size: c.size,
            // A member's own timestamp when the format recorded one; otherwise
            // the container's, which at least dates the content plausibly.
            mtime: c.mtime.or(self.mtime),
            atime: None,
            ctime: None,
            btime: None,
            inode: None,
            mode: c.mode,
            uid: None,
            gid: None,
            symlink_target: None,
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

fn base_name(inner: &str) -> String {
    inner.rsplit('/').next().unwrap_or("").to_string()
}

fn parent_inner(inner: &str) -> String {
    match inner.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => inner[..i].to_string(),
    }
}

/// Cache of parsed archive trees, shared with the writers `open_write` hands
/// out so a committed member invalidates the listing immediately.
type Cache = Mutex<HashMap<PathBuf, Arc<ArchiveTree>>>;

/// The local-disk archive backend.
pub struct ArchiveFs {
    cache: Arc<Cache>,
}

impl ArchiveFs {
    pub fn new() -> Self {
        ArchiveFs { cache: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Get (or rebuild) the tree for an archive, keyed on the container's
    /// mtime *and* length so a change — ours or an outside one — invalidates
    /// the cache automatically.
    async fn tree(&self, container: &Path) -> Result<Arc<ArchiveTree>> {
        let stamp = stamp_of(container).await;
        {
            let cache = self.cache.lock().unwrap();
            if let Some(t) = cache.get(container)
                && t.stamp == stamp
            {
                return Ok(t.clone());
            }
        }
        let path = container.to_path_buf();
        let tree = tokio::task::spawn_blocking(move || build_tree(&path))
            .await
            .map_err(|e| Error::other(e.to_string()))??;
        let arc = Arc::new(tree);
        self.cache.lock().unwrap().insert(container.to_path_buf(), arc.clone());
        Ok(arc)
    }

    /// Run a rebuild for `path`'s container on a blocking thread and drop the
    /// cached tree, so the very next listing reflects the change even if the
    /// filesystem's timestamps are too coarse to notice it.
    async fn mutate_path<F>(&self, path: &VfsPath, edit: F) -> Result<()>
    where
        F: FnOnce(&str, &mut Vec<FullEntry>) -> Result<bool> + Send + 'static,
    {
        let container = container_of(path)?.clone();
        let inner = normalize(&path.path.to_string_lossy());
        let target = container.clone();
        let result =
            tokio::task::spawn_blocking(move || mutate(&target, |entries| edit(&inner, entries)))
                .await
                .map_err(|e| Error::other(e.to_string()))?;
        self.cache.lock().unwrap().remove(&container);
        result
    }
}

/// The container's (mtime, length), or `(None, 0)` if it cannot be read.
async fn stamp_of(container: &Path) -> Stamp {
    match tokio::fs::metadata(container).await {
        Ok(m) => (m.modified().ok(), m.len()),
        Err(_) => (None, 0),
    }
}

impl Default for ArchiveFs {
    fn default() -> Self {
        Self::new()
    }
}

fn container_of(path: &VfsPath) -> Result<&PathBuf> {
    path.container.as_ref().ok_or_else(|| Error::InvalidPath("not an archive path".to_string()))
}

#[async_trait::async_trait]
impl Vfs for ArchiveFs {
    fn scheme(&self) -> &str {
        "archive"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            writable: true,
            // zip/tar keep a unix mode, 7z does not — `set_permissions` reports
            // the difference per archive rather than disabling the menu item
            // for the formats that can oblige.
            permissions: true,
            ownership: false,
            symlinks: false,
            random_access: false,
            inode: false,
            server_rename: true,
            // Nothing is written until a member writer is shut down, so an
            // aborted copy leaves no partial member behind.
            atomic_write: true,
        }
    }

    async fn read_dir(&self, dir: &VfsPath) -> Result<Vec<VfsEntry>> {
        let container = container_of(dir)?;
        let tree = self.tree(container).await?;
        tree.read_dir(&dir.path.to_string_lossy())
    }

    async fn stat(&self, path: &VfsPath) -> Result<VfsEntry> {
        let container = container_of(path)?;
        let tree = self.tree(container).await?;
        tree.stat(&path.path.to_string_lossy())
    }

    async fn open_read(&self, path: &VfsPath) -> Result<BoxRead> {
        let container = container_of(path)?.clone();
        let tree = self.tree(&container).await?;
        let format = tree.format;
        let inner = path.path.to_string_lossy().into_owned();
        let data =
            tokio::task::spawn_blocking(move || formats::read_entry(format, &container, &inner))
                .await
                .map_err(|e| Error::other(e.to_string()))??;
        Ok(Box::new(BytesReader { data, pos: 0 }))
    }

    async fn open_write(&self, path: &VfsPath, meta: WriteMeta) -> Result<BoxWrite> {
        let container = container_of(path)?.clone();
        // Fail here rather than at shutdown, so a copy into a read-only format
        // is refused before its bytes have been read.
        writable_format(&container)?;
        Ok(Box::new(MemberWriter {
            container,
            inner: normalize(&path.path.to_string_lossy()),
            buf: Vec::new(),
            append: meta.append,
            mtime: meta.mtime,
            mode: meta.mode,
            cache: self.cache.clone(),
            commit: None,
            done: false,
        }))
    }

    async fn mkdir(&self, path: &VfsPath) -> Result<()> {
        self.mutate_path(path, |inner, entries| {
            if inner == "/" {
                return Err(Error::other("the archive root already exists"));
            }
            let kinds = Kinds::of(entries);
            if kinds.kind(inner).is_some() {
                return Err(Error::other(format!("\"{}\" already exists", base_name(inner))));
            }
            kinds.require_parent(inner)?;
            entries.push(FullEntry::dir(inner).with_meta(Some(SystemTime::now()), Some(0o755)));
            Ok(true)
        })
        .await
    }

    async fn remove_file(&self, path: &VfsPath) -> Result<()> {
        self.mutate_path(path, |inner, entries| {
            match Kinds::of(entries).kind(inner) {
                Some(VfsKind::Dir) => {
                    return Err(Error::other(format!("\"{}\" is a directory", base_name(inner))));
                }
                Some(_) => {}
                None => return Err(Error::NotFound(inner.to_string())),
            }
            entries.retain(|e| normalize(&e.path) != inner);
            Ok(true)
        })
        .await
    }

    async fn remove_dir(&self, path: &VfsPath) -> Result<()> {
        self.mutate_path(path, |inner, entries| {
            match Kinds::of(entries).kind(inner) {
                Some(VfsKind::Dir) => {}
                Some(_) => {
                    return Err(Error::other(format!(
                        "\"{}\" is not a directory",
                        base_name(inner)
                    )));
                }
                // A directory that no member declares exists only as long as it
                // has children, so a recursive delete that just removed the last
                // one finds nothing left to remove. That is the delete having
                // succeeded, not a missing path.
                None => return Ok(false),
            }
            let prefix = format!("{inner}/");
            if entries.iter().any(|e| normalize(&e.path).starts_with(&prefix)) {
                return Err(Error::other(format!("\"{}\" is not empty", base_name(inner))));
            }
            entries.retain(|e| normalize(&e.path) != inner);
            Ok(true)
        })
        .await
    }

    async fn rename(&self, from: &VfsPath, to: &VfsPath) -> Result<()> {
        let container = container_of(from)?.clone();
        // Renaming across two archives is not a rename; the caller falls back to
        // copy + delete, which reads and writes each side separately.
        if container_of(to)? != &container {
            return Err(Error::Unsupported);
        }
        let dst = normalize(&to.path.to_string_lossy());
        self.mutate_path(from, move |src, entries| {
            if src == "/" || dst == "/" {
                return Err(Error::other("cannot rename the archive root"));
            }
            let kinds = Kinds::of(entries);
            if kinds.kind(src).is_none() {
                return Err(Error::NotFound(src.to_string()));
            }
            if kinds.kind(&dst).is_some() {
                return Err(Error::other(format!("\"{}\" already exists", base_name(&dst))));
            }
            if dst.starts_with(&format!("{src}/")) {
                return Err(Error::other("cannot move a directory into itself"));
            }
            kinds.require_parent(&dst)?;
            // Re-path the member and everything beneath it. A directory that
            // exists only implicitly has no member of its own — re-pathing its
            // children is exactly what moves it.
            let prefix = format!("{src}/");
            for e in entries.iter_mut() {
                let p = normalize(&e.path);
                if p == src {
                    e.path = dst.clone();
                } else if let Some(rest) = p.strip_prefix(&prefix) {
                    e.path = format!("{dst}/{rest}");
                }
            }
            Ok(true)
        })
        .await
    }

    async fn set_permissions(&self, path: &VfsPath, mode: u32) -> Result<()> {
        let stores_mode =
            ArchiveFormat::from_path(container_of(path)?).is_some_and(ArchiveFormat::stores_mode);
        if !stores_mode {
            return Err(Error::Unsupported);
        }
        self.mutate_path(path, move |inner, entries| set_meta(entries, inner, None, Some(mode)))
            .await
    }

    async fn set_mtime(&self, path: &VfsPath, mtime: SystemTime) -> Result<()> {
        self.mutate_path(path, move |inner, entries| set_meta(entries, inner, Some(mtime), None))
            .await
    }
}

/// Apply a timestamp and/or mode to one member. Returns `false` — no rebuild —
/// when the member already carries those values, which is the common case:
/// a copy stamps the member as it writes it and the engine then re-applies the
/// same mode, and a rebuild per copied file would be ruinous.
fn set_meta(
    entries: &mut [FullEntry],
    inner: &str,
    mtime: Option<SystemTime>,
    mode: Option<u32>,
) -> Result<bool> {
    let e = entries
        .iter_mut()
        .find(|e| normalize(&e.path) == inner)
        .ok_or_else(|| Error::NotFound(inner.to_string()))?;
    let mut changed = false;
    if let Some(t) = mtime
        && e.mtime != Some(t)
    {
        e.mtime = Some(t);
        changed = true;
    }
    if let Some(m) = mode
        && e.mode.map(|old| old & 0o7777) != Some(m & 0o7777)
    {
        e.mode = Some(m);
        changed = true;
    }
    Ok(changed)
}

/// A boxed in-memory async reader over a single extracted entry.
struct BytesReader {
    data: Vec<u8>,
    pos: usize,
}

impl AsyncRead for BytesReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let remaining = self.data.len() - self.pos;
        let n = remaining.min(buf.remaining());
        if n > 0 {
            let start = self.pos;
            buf.put_slice(&self.data[start..start + n]);
            self.pos += n;
        }
        Poll::Ready(Ok(()))
    }
}

/// A writer that buffers one member and splices it into the archive when the
/// caller shuts it down.
///
/// Archives have no per-member write API — every change rebuilds the whole
/// container — so the write lands in one step at the end instead of streaming.
/// Nothing is committed before `shutdown`, which is what
/// [`Capabilities::atomic_write`] tells the ops engine: an aborted copy leaves
/// the archive exactly as it was, with no half-written member to clean up (and,
/// crucially, without deleting the member that was already there).
struct MemberWriter {
    container: PathBuf,
    inner: String,
    buf: Vec<u8>,
    append: bool,
    mtime: Option<SystemTime>,
    mode: Option<u32>,
    cache: Arc<Cache>,
    /// The in-flight rebuild, once `shutdown` has started it.
    commit: Option<tokio::task::JoinHandle<Result<()>>>,
    done: bool,
}

impl AsyncWrite for MemberWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.buf.extend_from_slice(data);
        Poll::Ready(Ok(data.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        // There is nothing to flush to: the member is committed by `shutdown`.
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.done {
            return Poll::Ready(Ok(()));
        }
        if this.commit.is_none() {
            let container = this.container.clone();
            let inner = std::mem::take(&mut this.inner);
            let data = std::mem::take(&mut this.buf);
            let (append, mtime, mode) = (this.append, this.mtime, this.mode);
            this.commit = Some(tokio::task::spawn_blocking(move || {
                write_member(&container, &inner, data, append, mtime, mode)
            }));
        }
        let handle = this.commit.as_mut().expect("just started");
        let outcome = match Pin::new(handle).poll(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Ok(r)) => r,
            Poll::Ready(Err(e)) => Err(Error::other(e.to_string())),
        };
        this.done = true;
        this.commit = None;
        this.cache.lock().unwrap().remove(&this.container);
        Poll::Ready(outcome.map_err(|e| std::io::Error::other(e.to_string())))
    }
}

fn build_tree(container: &Path) -> Result<ArchiveTree> {
    let format = ArchiveFormat::from_path(container)
        .ok_or_else(|| Error::other("unknown archive format"))?;
    let meta = std::fs::metadata(container).ok();
    let mtime = meta.as_ref().and_then(|m| m.modified().ok());
    let stamp = (mtime, meta.map(|m| m.len()).unwrap_or(0));
    let raw = formats::list_entries(format, container)?;
    let mut dirs: HashMap<String, Vec<ChildMeta>> = HashMap::new();
    dirs.insert("/".to_string(), Vec::new());
    for entry in &raw {
        insert_path(&mut dirs, entry);
    }
    Ok(ArchiveTree { format, stamp, mtime, dirs })
}

/// Graft one archive member onto the tree, creating the directories its path
/// implies.
///
/// A name that appears as a directory anywhere — an explicit member, or simply
/// because something lives under it — stays a directory. Some writers store a
/// folder as a zero-length *file* entry alongside its children; treating that
/// name as a file would hide the whole subtree, and would make `stat` and
/// `read_dir` disagree about what it is.
fn insert_path(dirs: &mut HashMap<String, Vec<ChildMeta>>, entry: &RawEntry) {
    let norm = normalize(&entry.path);
    if norm == "/" {
        return;
    }
    let comps: Vec<&str> = norm[1..].split('/').collect();
    let mut parent = "/".to_string();
    for (i, comp) in comps.iter().enumerate() {
        let is_last = i + 1 == comps.len();
        // Only the last component can be a file; every prefix is a directory.
        let as_dir = !is_last || entry.is_dir;
        let child_norm =
            if parent == "/" { format!("/{comp}") } else { format!("{parent}/{comp}") };

        let list = dirs.entry(parent.clone()).or_default();
        let is_dir = match list.iter_mut().find(|c| c.name == *comp) {
            Some(existing) => {
                if as_dir {
                    existing.kind = VfsKind::Dir;
                    existing.size = 0;
                }
                if is_last {
                    existing.mtime = entry.mtime.or(existing.mtime);
                    existing.mode = entry.mode.or(existing.mode);
                    if existing.kind == VfsKind::File {
                        existing.size = entry.size;
                    }
                }
                existing.kind.is_dir()
            }
            None => {
                list.push(ChildMeta {
                    name: (*comp).to_string(),
                    kind: if as_dir { VfsKind::Dir } else { VfsKind::File },
                    size: if as_dir { 0 } else { entry.size },
                    mtime: is_last.then_some(entry.mtime).flatten(),
                    mode: is_last.then_some(entry.mode).flatten(),
                });
                as_dir
            }
        };
        // Every directory gets a listing of its own, so `read_dir` succeeds for
        // exactly the names `stat` calls directories.
        if is_dir {
            dirs.entry(child_norm.clone()).or_default();
        }
        parent = child_norm;
    }
}

// ---------------------------------------------------------------------------
// Mutation helpers (run on a blocking thread by AppState / the Vfs impl)
// ---------------------------------------------------------------------------

/// What each inner path in a member list currently is, precomputed so the
/// conflict checks below are lookups rather than repeated scans.
struct Kinds {
    dirs: HashSet<String>,
    files: HashSet<String>,
}

impl Kinds {
    fn of(entries: &[FullEntry]) -> Self {
        let mut dirs = HashSet::new();
        let mut files = HashSet::new();
        dirs.insert("/".to_string());
        for e in entries {
            let p = normalize(&e.path);
            if p == "/" {
                continue;
            }
            if e.is_dir {
                dirs.insert(p.clone());
            } else {
                files.insert(p.clone());
            }
            let mut cur = parent_inner(&p);
            while cur != "/" {
                dirs.insert(cur.clone());
                cur = parent_inner(&cur);
            }
        }
        // Same rule as the browse tree: a name with children is a directory.
        files.retain(|f| !dirs.contains(f));
        Kinds { dirs, files }
    }

    fn kind(&self, inner: &str) -> Option<VfsKind> {
        if self.dirs.contains(inner) {
            Some(VfsKind::Dir)
        } else if self.files.contains(inner) {
            Some(VfsKind::File)
        } else {
            None
        }
    }

    /// A filesystem refuses to create an entry whose parent directory is
    /// missing (or is a file); so does an archive.
    fn require_parent(&self, inner: &str) -> Result<()> {
        let parent = parent_inner(inner);
        match self.kind(&parent) {
            Some(VfsKind::Dir) => Ok(()),
            Some(_) => Err(Error::other(format!("\"{parent}\" is not a directory"))),
            None => Err(Error::NotFound(parent)),
        }
    }
}

/// The archive's format, if it is one we can rewrite.
fn writable_format(container: &Path) -> Result<ArchiveFormat> {
    let format = ArchiveFormat::from_path(container)
        .ok_or_else(|| Error::other("unknown archive format"))?;
    if !format.writable() {
        return Err(Error::other("this archive format is read-only"));
    }
    Ok(format)
}

/// Read every member, let `edit` change the list, and write it back. `edit`
/// returns `false` to say nothing changed, which skips the (expensive) rewrite.
fn mutate<F>(container: &Path, edit: F) -> Result<()>
where
    F: FnOnce(&mut Vec<FullEntry>) -> Result<bool>,
{
    let format = writable_format(container)?;
    let mut entries = formats::read_all(format, container)?;
    if !edit(&mut entries)? {
        return Ok(());
    }
    write_swap(format, container, &entries)
}

/// Create a new archive `dest` from local `sources` (files/dirs). Entry names
/// are taken relative to each source's parent directory.
pub fn create_archive(format: ArchiveFormat, dest: &Path, sources: &[PathBuf]) -> Result<()> {
    let entries = read_staged(&stage(sources, "/")?)?;
    formats::write_all(format, dest, &entries)
}

/// Add local `sources` into an existing archive under `dest_inner` (rebuild).
/// A member of the same name is replaced, the way copying onto an existing file
/// replaces it; a file landing on a directory (or the reverse) is refused, the
/// way a filesystem refuses it.
pub fn add_to_archive(container: &Path, dest_inner: &str, sources: &[PathBuf]) -> Result<()> {
    let staged = stage(sources, dest_inner)?;
    mutate(container, |entries| {
        let kinds = Kinds::of(entries);
        for s in &staged {
            match (kinds.kind(&s.inner), s.is_dir) {
                (Some(VfsKind::Dir), false) => {
                    return Err(Error::other(format!(
                        "\"{}\" is a directory in the archive",
                        s.inner
                    )));
                }
                (Some(VfsKind::File), true) => {
                    return Err(Error::other(format!("\"{}\" is a file in the archive", s.inner)));
                }
                _ => {}
            }
        }
        // The sources land *in* `dest_inner`, so it has to be a directory there.
        let dest = normalize(dest_inner);
        match kinds.kind(&dest) {
            Some(VfsKind::Dir) => {}
            Some(_) => return Err(Error::other(format!("\"{dest}\" is not a directory"))),
            None => return Err(Error::NotFound(dest)),
        }
        entries.extend(read_staged(&staged)?);
        Ok(true)
    })
}

/// The members `add_to_archive` would replace: existing files the incoming
/// sources land on. Reads only the archive's index, not its contents, so the
/// caller can ask before committing to a full rebuild.
pub fn add_conflicts(
    container: &Path,
    dest_inner: &str,
    sources: &[PathBuf],
) -> Result<Vec<String>> {
    let format = writable_format(container)?;
    let existing: Vec<FullEntry> = formats::list_entries(format, container)?
        .into_iter()
        .map(
            |e| if e.is_dir { FullEntry::dir(e.path) } else { FullEntry::file(e.path, Vec::new()) },
        )
        .collect();
    let kinds = Kinds::of(&existing);
    Ok(stage(sources, dest_inner)?
        .into_iter()
        .filter(|s| !s.is_dir && kinds.kind(&s.inner) == Some(VfsKind::File))
        .map(|s| s.inner)
        .collect())
}

/// Remove inner paths (and their subtrees) from an archive (rebuild).
pub fn remove_from_archive(container: &Path, remove: &HashSet<String>) -> Result<()> {
    let doomed: Vec<String> = remove.iter().map(|r| normalize(r)).collect();
    mutate(container, |entries| {
        entries.retain(|e| {
            let p = normalize(&e.path);
            !doomed.iter().any(|r| p == *r || p.starts_with(&format!("{r}/")))
        });
        Ok(true)
    })
}

/// Write one member's bytes, replacing (or appending to) whatever is there.
/// Used by the [`MemberWriter`] `open_write` hands out.
fn write_member(
    container: &Path,
    inner: &str,
    data: Vec<u8>,
    append: bool,
    mtime: Option<SystemTime>,
    mode: Option<u32>,
) -> Result<()> {
    mutate(container, |entries| {
        let kinds = Kinds::of(entries);
        match kinds.kind(inner) {
            Some(VfsKind::Dir) => {
                return Err(Error::other(format!("\"{}\" is a directory", base_name(inner))));
            }
            Some(_) => {}
            None => kinds.require_parent(inner)?,
        }
        let data = if append {
            let mut prev = entries
                .iter()
                .find(|e| normalize(&e.path) == inner)
                .map(|e| e.data.clone())
                .unwrap_or_default();
            prev.extend_from_slice(&data);
            prev
        } else {
            data
        };
        // The writers keep a replaced member's original position, so pushing is
        // an overwrite in place rather than a move to the end.
        entries.push(
            FullEntry::file(inner, data)
                .with_meta(Some(mtime.unwrap_or_else(SystemTime::now)), mode),
        );
        Ok(true)
    })
}

fn write_swap(format: ArchiveFormat, container: &Path, entries: &[FullEntry]) -> Result<()> {
    let tmp = container.with_extension("rc-tmp");
    // Rebuild into a sibling temp, then atomically swap it in. On any failure —
    // a bad write or a failed rename — remove the temp so a failed archive edit
    // never leaves a stray `.rc-tmp` next to the user's archive.
    let result = (|| -> Result<()> {
        formats::write_all(format, &tmp, entries)?;
        // The swap replaces the file, so carry the archive's own permissions
        // over rather than leaving the rebuilt copy at the process umask.
        if let Ok(meta) = std::fs::metadata(container) {
            let _ = std::fs::set_permissions(&tmp, meta.permissions());
        }
        std::fs::rename(&tmp, container)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// One local path staged for insertion, with the inner path it will take.
struct Staged {
    inner: String,
    is_dir: bool,
    source: PathBuf,
}

/// Walk `sources` and work out where each file/directory lands inside the
/// archive, without reading any file content — so a caller that only needs the
/// names (the overwrite check) does not pay for the bytes.
fn stage(sources: &[PathBuf], dest_inner: &str) -> Result<Vec<Staged>> {
    let mut out = Vec::new();
    for src in sources {
        let base = src.parent().unwrap_or(Path::new("/"));
        stage_one(src, base, dest_inner, &mut out)?;
    }
    Ok(out)
}

fn stage_one(path: &Path, base: &Path, dest_inner: &str, out: &mut Vec<Staged>) -> Result<()> {
    let meta = std::fs::metadata(path)?;
    let rel = path.strip_prefix(base).unwrap_or(path);
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    let inner = join_inner(dest_inner, &rel_str);

    if meta.is_dir() {
        out.push(Staged { inner, is_dir: true, source: path.to_path_buf() });
        let mut children: Vec<PathBuf> =
            std::fs::read_dir(path)?.filter_map(|e| e.ok().map(|e| e.path())).collect();
        children.sort();
        for child in children {
            stage_one(&child, base, dest_inner, out)?;
        }
    } else {
        out.push(Staged { inner, is_dir: false, source: path.to_path_buf() });
    }
    Ok(())
}

/// Read the staged files' bytes and metadata into archive members. The
/// timestamps and modes ride along so the archived copy keeps them.
fn read_staged(staged: &[Staged]) -> Result<Vec<FullEntry>> {
    staged
        .iter()
        .map(|s| {
            let meta = std::fs::metadata(&s.source)?;
            let entry = if s.is_dir {
                FullEntry::dir(s.inner.clone())
            } else {
                FullEntry::file(s.inner.clone(), std::fs::read(&s.source)?)
            };
            Ok(entry.with_meta(meta.modified().ok(), local_mode(&meta)))
        })
        .collect()
}

/// The unix permission bits of a local file, where the platform has them.
#[cfg(unix)]
fn local_mode(meta: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(meta.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn local_mode(_meta: &std::fs::Metadata) -> Option<u32> {
    None
}

fn join_inner(dir: &str, name: &str) -> String {
    let d = normalize(dir);
    let n = name.trim_matches('/');
    if d == "/" { format!("/{n}") } else { format!("{d}/{n}") }
}

#[cfg(test)]
mod tests;
