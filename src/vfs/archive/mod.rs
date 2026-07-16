//! `archive://` VFS backend — browse archives like directories.
//!
//! Reading (browse + extract) goes through the [`Vfs`] trait so the generic
//! ops engine can extract archive→local for free. Mutations (create / add /
//! remove) are *not* per-file `open_write` calls — they rebuild the whole
//! archive in one pass via the free functions at the bottom of this module,
//! driven by `AppState`.

pub mod formats;

use crate::util::{Error, Result};
use crate::vfs::{BoxRead, BoxWrite, Capabilities, Vfs, VfsEntry, VfsKind, VfsPath, WriteMeta};
use formats::{normalize, ArchiveFormat, FullEntry};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::SystemTime;
use tokio::io::{AsyncRead, ReadBuf};

/// In-memory directory tree built from an archive's member list.
struct ChildMeta {
    name: String,
    kind: VfsKind,
    size: u64,
}

struct ArchiveTree {
    format: ArchiveFormat,
    mtime: Option<SystemTime>,
    /// Inner dir path (`/a`) -> its children.
    dirs: HashMap<String, Vec<ChildMeta>>,
}

impl ArchiveTree {
    fn read_dir(&self, inner: &str, mtime: Option<SystemTime>) -> Result<Vec<VfsEntry>> {
        let norm = normalize(inner);
        let children = self
            .dirs
            .get(&norm)
            .ok_or_else(|| Error::NotFound(norm.clone()))?;
        Ok(children
            .iter()
            .map(|c| VfsEntry {
                name: c.name.clone(),
                kind: c.kind,
                size: c.size,
                mtime,
                atime: None,
                ctime: None,
                inode: None,
                mode: None,
                uid: None,
                gid: None,
                symlink_target: None,
                symlink_broken: false,
            })
            .collect())
    }

    fn stat(&self, inner: &str, mtime: Option<SystemTime>) -> Result<VfsEntry> {
        let norm = normalize(inner);
        if norm == "/" || self.dirs.contains_key(&norm) {
            return Ok(dir_entry(&base_name(&norm), mtime));
        }
        let parent = parent_inner(&norm);
        let name = base_name(&norm);
        let child = self
            .dirs
            .get(&parent)
            .and_then(|c| c.iter().find(|c| c.name == name))
            .ok_or_else(|| Error::NotFound(norm.clone()))?;
        Ok(VfsEntry {
            name,
            kind: child.kind,
            size: child.size,
            mtime,
            atime: None,
            ctime: None,
            inode: None,
            mode: None,
            uid: None,
            gid: None,
            symlink_target: None,
            symlink_broken: false,
        })
    }
}

fn dir_entry(name: &str, mtime: Option<SystemTime>) -> VfsEntry {
    VfsEntry {
        name: name.to_string(),
        kind: VfsKind::Dir,
        size: 0,
        mtime,
        atime: None,
        ctime: None,
        inode: None,
        mode: None,
        uid: None,
        gid: None,
        symlink_target: None,
        symlink_broken: false,
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

/// The local-disk archive backend.
pub struct ArchiveFs {
    cache: Mutex<HashMap<PathBuf, Arc<ArchiveTree>>>,
}

impl ArchiveFs {
    pub fn new() -> Self {
        ArchiveFs {
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Get (or rebuild) the tree for an archive, keyed on the file's mtime so
    /// external/our-own changes invalidate the cache automatically.
    async fn tree(&self, container: &Path) -> Result<Arc<ArchiveTree>> {
        let cur_mtime = tokio::fs::metadata(container)
            .await
            .ok()
            .and_then(|m| m.modified().ok());
        {
            let cache = self.cache.lock().unwrap();
            if let Some(t) = cache.get(container)
                && t.mtime == cur_mtime
            {
                return Ok(t.clone());
            }
        }
        let path = container.to_path_buf();
        let tree = tokio::task::spawn_blocking(move || build_tree(&path))
            .await
            .map_err(|e| Error::other(e.to_string()))??;
        let arc = Arc::new(tree);
        self.cache
            .lock()
            .unwrap()
            .insert(container.to_path_buf(), arc.clone());
        Ok(arc)
    }
}

impl Default for ArchiveFs {
    fn default() -> Self {
        Self::new()
    }
}

fn container_of(path: &VfsPath) -> Result<&PathBuf> {
    path.container
        .as_ref()
        .ok_or_else(|| Error::InvalidPath("not an archive path".to_string()))
}

#[async_trait::async_trait]
impl Vfs for ArchiveFs {
    fn scheme(&self) -> &str {
        "archive"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            writable: false, // mutations are special-cased (rebuild), not per-file
            permissions: false,
            ownership: false,
            symlinks: false,
            random_access: false,
            inode: false,
            server_rename: false,
        }
    }

    async fn read_dir(&self, dir: &VfsPath) -> Result<Vec<VfsEntry>> {
        let container = container_of(dir)?;
        let tree = self.tree(container).await?;
        tree.read_dir(&dir.path.to_string_lossy(), tree.mtime)
    }

    async fn stat(&self, path: &VfsPath) -> Result<VfsEntry> {
        let container = container_of(path)?;
        let tree = self.tree(container).await?;
        tree.stat(&path.path.to_string_lossy(), tree.mtime)
    }

    async fn open_read(&self, path: &VfsPath) -> Result<BoxRead> {
        let container = container_of(path)?.clone();
        let tree = self.tree(&container).await?;
        let format = tree.format;
        let inner = path.path.to_string_lossy().into_owned();
        let data = tokio::task::spawn_blocking(move || formats::read_entry(format, &container, &inner))
            .await
            .map_err(|e| Error::other(e.to_string()))??;
        Ok(Box::new(BytesReader { data, pos: 0 }))
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

fn build_tree(container: &Path) -> Result<ArchiveTree> {
    let format =
        ArchiveFormat::from_path(container).ok_or_else(|| Error::other("unknown archive format"))?;
    let mtime = std::fs::metadata(container)
        .ok()
        .and_then(|m| m.modified().ok());
    let raw = formats::list_entries(format, container)?;
    let mut dirs: HashMap<String, Vec<ChildMeta>> = HashMap::new();
    dirs.insert("/".to_string(), Vec::new());
    for entry in raw {
        insert_path(&mut dirs, &entry.path, entry.is_dir, entry.size);
    }
    Ok(ArchiveTree {
        format,
        mtime,
        dirs,
    })
}

fn insert_path(dirs: &mut HashMap<String, Vec<ChildMeta>>, norm: &str, is_dir: bool, size: u64) {
    if norm == "/" {
        return;
    }
    let comps: Vec<&str> = norm.trim_matches('/').split('/').collect();
    let mut parent = "/".to_string();
    for (i, comp) in comps.iter().enumerate() {
        let is_last = i == comps.len() - 1;
        let child_norm = if parent == "/" {
            format!("/{comp}")
        } else {
            format!("{parent}/{comp}")
        };
        let kind = if is_last && !is_dir {
            VfsKind::File
        } else {
            VfsKind::Dir
        };
        let csize = if is_last { size } else { 0 };

        let list = dirs.entry(parent.clone()).or_default();
        if let Some(existing) = list.iter_mut().find(|c| c.name == *comp) {
            if kind == VfsKind::File {
                existing.kind = VfsKind::File;
                existing.size = csize;
            }
        } else {
            list.push(ChildMeta {
                name: (*comp).to_string(),
                kind,
                size: csize,
            });
        }
        if kind == VfsKind::Dir {
            dirs.entry(child_norm.clone()).or_default();
        }
        parent = child_norm;
    }
}

// ---------------------------------------------------------------------------
// Mutation helpers (run on a blocking thread by AppState)
// ---------------------------------------------------------------------------

/// Create a new archive `dest` from local `sources` (files/dirs). Entry names
/// are taken relative to each source's parent directory.
pub fn create_archive(format: ArchiveFormat, dest: &Path, sources: &[PathBuf]) -> Result<()> {
    let mut entries = Vec::new();
    for src in sources {
        let base = src.parent().unwrap_or(Path::new("/"));
        collect_entries(src, base, "/", &mut entries)?;
    }
    formats::write_all(format, dest, &entries)
}

/// Add local `sources` into an existing archive under `dest_inner` (rebuild).
pub fn add_to_archive(
    container: &Path,
    dest_inner: &str,
    sources: &[PathBuf],
) -> Result<()> {
    let format = ArchiveFormat::from_path(container)
        .ok_or_else(|| Error::other("unknown archive format"))?;
    if !format.writable() {
        return Err(Error::other("this archive format is read-only"));
    }
    let mut entries = formats::read_all(format, container)?;
    for src in sources {
        let base = src.parent().unwrap_or(Path::new("/"));
        collect_entries(src, base, dest_inner, &mut entries)?;
    }
    write_swap(format, container, &entries)
}

/// Remove inner paths (and their subtrees) from an archive (rebuild).
pub fn remove_from_archive(container: &Path, remove: &HashSet<String>) -> Result<()> {
    let format = ArchiveFormat::from_path(container)
        .ok_or_else(|| Error::other("unknown archive format"))?;
    if !format.writable() {
        return Err(Error::other("this archive format is read-only"));
    }
    let entries = formats::read_all(format, container)?;
    let kept: Vec<FullEntry> = entries
        .into_iter()
        .filter(|e| {
            !remove.iter().any(|r| {
                let r = normalize(r);
                e.path == r || e.path.starts_with(&format!("{r}/"))
            })
        })
        .collect();
    write_swap(format, container, &kept)
}

fn write_swap(format: ArchiveFormat, container: &Path, entries: &[FullEntry]) -> Result<()> {
    let tmp = container.with_extension("rc-tmp");
    // Rebuild into a sibling temp, then atomically swap it in. On any failure —
    // a bad write or a failed rename — remove the temp so a failed archive edit
    // never leaves a stray `.rc-tmp` next to the user's archive.
    let result = (|| -> Result<()> {
        formats::write_all(format, &tmp, entries)?;
        std::fs::rename(&tmp, container)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Recursively collect a local path into archive entries rooted at `dest_inner`.
fn collect_entries(
    path: &Path,
    base: &Path,
    dest_inner: &str,
    out: &mut Vec<FullEntry>,
) -> Result<()> {
    let meta = std::fs::metadata(path)?;
    let rel = path.strip_prefix(base).unwrap_or(path);
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    let inner = join_inner(dest_inner, &rel_str);

    if meta.is_dir() {
        out.push(FullEntry {
            path: inner,
            is_dir: true,
            data: Vec::new(),
        });
        let mut children: Vec<PathBuf> = std::fs::read_dir(path)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        children.sort();
        for child in children {
            collect_entries(&child, base, dest_inner, out)?;
        }
    } else {
        let data = std::fs::read(path)?;
        out.push(FullEntry {
            path: inner,
            is_dir: false,
            data,
        });
    }
    Ok(())
}

fn join_inner(dir: &str, name: &str) -> String {
    let d = normalize(dir);
    let n = name.trim_matches('/');
    if d == "/" {
        format!("/{n}")
    } else {
        format!("{d}/{n}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    fn unique_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("rc_arc_{tag}_{}_{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A failed archive rebuild must not leave a stray `.rc-tmp` beside the
    /// archive. We force the rename to fail (the target path is a directory)
    /// after the temp has been written, and assert it was cleaned up.
    #[test]
    fn write_swap_removes_the_temp_when_the_swap_fails() {
        let dir = unique_dir("swapfail");
        // `container` is an existing directory, so renaming the temp file onto it
        // fails — but only after `write_all` has created the temp.
        let container = dir.join("archive.zip");
        std::fs::create_dir(&container).unwrap();
        let tmp = container.with_extension("rc-tmp");

        let entries = vec![FullEntry { path: "f.txt".into(), is_dir: false, data: b"x".to_vec() }];
        let result = write_swap(ArchiveFormat::Zip, &container, &entries);
        assert!(result.is_err(), "renaming onto a directory fails");
        assert!(!tmp.exists(), "the .rc-tmp was cleaned up, not left behind: {tmp:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    fn make_sources(root: &Path) -> Vec<PathBuf> {
        std::fs::create_dir_all(root.join("data")).unwrap();
        std::fs::write(root.join("data/a.txt"), b"alpha").unwrap();
        std::fs::write(root.join("data/b.txt"), b"beta").unwrap();
        std::fs::write(root.join("readme"), b"hi").unwrap();
        vec![root.join("data"), root.join("readme")]
    }

    async fn names(fs: &ArchiveFs, container: &Path, inner: &str) -> Vec<String> {
        let p = VfsPath::archive(container, inner);
        let mut v: Vec<String> = fs.read_dir(&p).await.unwrap().into_iter().map(|e| e.name).collect();
        v.sort();
        v
    }

    async fn read_entry_bytes(fs: &ArchiveFs, container: &Path, inner: &str) -> Vec<u8> {
        let p = VfsPath::archive(container, inner);
        let mut r = fs.open_read(&p).await.unwrap();
        let mut buf = Vec::new();
        r.read_to_end(&mut buf).await.unwrap();
        buf
    }

    #[tokio::test]
    async fn create_browse_extract_each_format() {
        for (ext, _fmt) in [("zip", ()), ("tar.gz", ()), ("7z", ())] {
            let root = unique_dir(ext.replace('.', "_").as_str());
            let sources = make_sources(&root);
            let container = root.join(format!("out.{ext}"));
            let format = ArchiveFormat::from_path(&container).unwrap();
            create_archive(format, &container, &sources).unwrap();

            let fs = ArchiveFs::new();
            assert_eq!(names(&fs, &container, "/").await, vec!["data", "readme"], "root of {ext}");
            assert_eq!(names(&fs, &container, "/data").await, vec!["a.txt", "b.txt"], "data/ of {ext}");
            assert_eq!(read_entry_bytes(&fs, &container, "/data/a.txt").await, b"alpha", "{ext}");
            assert_eq!(read_entry_bytes(&fs, &container, "/readme").await, b"hi", "{ext}");

            std::fs::remove_dir_all(&root).ok();
        }
    }

    #[tokio::test]
    async fn add_and_remove_in_zip() {
        let root = unique_dir("zipmut");
        let sources = make_sources(&root);
        let container = root.join("m.zip");
        create_archive(ArchiveFormat::Zip, &container, &sources).unwrap();

        // Add a new local file into /data.
        std::fs::write(root.join("c.txt"), b"gamma").unwrap();
        add_to_archive(&container, "/data", &[root.join("c.txt")]).unwrap();
        let fs = ArchiveFs::new();
        assert!(names(&fs, &container, "/data").await.contains(&"c.txt".to_string()));

        // Remove /readme.
        let mut set = HashSet::new();
        set.insert("/readme".to_string());
        remove_from_archive(&container, &set).unwrap();
        let fs2 = ArchiveFs::new();
        assert_eq!(names(&fs2, &container, "/").await, vec!["data"]);

        std::fs::remove_dir_all(&root).ok();
    }
}
