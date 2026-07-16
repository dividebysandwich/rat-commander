//! `extfs` VFS backend — browse a file through a Midnight-Commander `extfs.d`
//! script (`uzip`, `iso9660`, `rpm`, `deb`, …).
//!
//! Structurally a sibling of [`ArchiveFs`](crate::vfs::archive): a container
//! (the file on local disk) is exposed as a directory tree built from an
//! in-memory member list, cached and keyed on the container's mtime. The
//! difference is where the data comes from — instead of an in-process archive
//! crate, we shell out to the MC script:
//!
//! * `SCRIPT list <archive>`                        → the member listing
//! * `SCRIPT copyout <archive> <name> <extractto>`  → read one member
//! * `SCRIPT copyin  <archive> <name> <sourcefile>` → add/replace a member
//! * `SCRIPT rm/rmdir/mkdir <archive> <name>`       → mutate
//!
//! The scheme *is* the script prefix (`"uzip"`), so one `ExtfsFs` instance is
//! registered per prefix on demand and serves every container of that prefix.

use crate::util::{Error, Result};
use crate::vfs::membuf::{pipe_upload, MemReader};
use crate::vfs::remote::perms_to_mode;
use crate::vfs::{BoxRead, BoxWrite, Capabilities, Vfs, VfsEntry, VfsKind, VfsPath, WriteMeta};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Pipe capacity for streaming an upload into `copyin` (bounded memory).
const PIPE_CAP: usize = 64 * 1024;

/// A member of the listing produced by `SCRIPT list`.
struct ExtfsListEntry {
    /// Path relative to the mount root, no leading slash (e.g. `data/edit.html`).
    path: String,
    kind: VfsKind,
    size: u64,
    mode: Option<u32>,
    symlink_target: Option<String>,
}

/// One child within a directory of the mount.
struct ChildMeta {
    name: String,
    kind: VfsKind,
    size: u64,
    mode: Option<u32>,
    symlink_target: Option<String>,
}

/// In-memory directory tree built from a `list` run.
struct ExtfsTree {
    mtime: Option<SystemTime>,
    /// Inner dir path (`/`, `/a`) → its children.
    dirs: HashMap<String, Vec<ChildMeta>>,
}

impl ExtfsTree {
    fn read_dir(&self, inner: &str) -> Result<Vec<VfsEntry>> {
        let norm = normalize_inner(inner);
        let children = self
            .dirs
            .get(&norm)
            .ok_or_else(|| Error::NotFound(norm.clone()))?;
        Ok(children.iter().map(|c| self.entry_of(c)).collect())
    }

    fn stat(&self, inner: &str) -> Result<VfsEntry> {
        let norm = normalize_inner(inner);
        if norm == "/" || self.dirs.contains_key(&norm) {
            return Ok(dir_entry(&base_name(&norm), self.mtime));
        }
        let parent = parent_inner(&norm);
        let name = base_name(&norm);
        let child = self
            .dirs
            .get(&parent)
            .and_then(|c| c.iter().find(|c| c.name == name))
            .ok_or_else(|| Error::NotFound(norm.clone()))?;
        Ok(self.entry_of(child))
    }

    fn entry_of(&self, c: &ChildMeta) -> VfsEntry {
        VfsEntry {
            name: c.name.clone(),
            kind: c.kind,
            size: c.size,
            mtime: self.mtime,
            atime: None,
            ctime: None,
            inode: None,
            mode: c.mode,
            uid: None,
            gid: None,
            symlink_target: c.symlink_target.clone(),
            symlink_broken: false,
        }
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

/// Normalize an inner path to `/`, `/a`, `/a/b` form (leading slash, no trailing).
fn normalize_inner(inner: &str) -> String {
    let trimmed = inner.replace('\\', "/");
    let trimmed = trimmed.trim_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        format!("/{trimmed}")
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

/// The local-disk file backing an extfs path (its `container`).
fn container_of(path: &VfsPath) -> Result<&PathBuf> {
    path.container
        .as_ref()
        .ok_or_else(|| Error::InvalidPath("not an extfs path".to_string()))
}

/// The member name to pass to the script: relative to the mount root (MC passes
/// archive-root-relative paths, without a leading slash).
fn inner_rel(path: &VfsPath) -> String {
    path.posix_path().trim_start_matches('/').to_string()
}

/// A unique temp path for copyout/copyin scratch. Shares the sweepable
/// `rc-tmp-` prefix with the rest of the app (see [`crate::util::temp`]).
fn scratch_path(tag: &str) -> PathBuf {
    crate::util::temp::rc_temp_path(&format!("extfs-{tag}"))
}

/// A file exposed via an MC `extfs.d` script.
pub struct ExtfsFs {
    prefix: String,
    script: PathBuf,
    cache: Mutex<HashMap<PathBuf, Arc<ExtfsTree>>>,
}

impl ExtfsFs {
    pub fn new(prefix: String, script: PathBuf) -> Self {
        ExtfsFs {
            prefix,
            script,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Get (or rebuild) the tree for `container`, keyed on its mtime so external
    /// or our-own changes invalidate the cache automatically.
    async fn tree(&self, container: &Path) -> Result<Arc<ExtfsTree>> {
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
        let out = Command::new(&self.script)
            .arg("list")
            .arg(container)
            .output()
            .await
            .map_err(|e| Error::other(format!("extfs '{}' list: {e}", self.prefix)))?;
        if !out.status.success() {
            return Err(Error::other(format!(
                "extfs '{}' list failed (exit {})",
                self.prefix, out.status
            )));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut dirs: HashMap<String, Vec<ChildMeta>> = HashMap::new();
        dirs.insert("/".to_string(), Vec::new());
        for line in text.lines() {
            if let Some(e) = parse_extfs_line(line) {
                insert_extfs_path(&mut dirs, &e);
            }
        }
        let tree = Arc::new(ExtfsTree {
            mtime: cur_mtime,
            dirs,
        });
        self.cache
            .lock()
            .unwrap()
            .insert(container.to_path_buf(), tree.clone());
        Ok(tree)
    }

    fn invalidate(&self, container: &Path) {
        self.cache.lock().unwrap().remove(container);
    }

    /// Run a mutating one-shot script command, mapping a nonzero exit to an error.
    async fn run_mut(&self, cmd: &str, container: &Path, name: &str) -> Result<()> {
        let status = Command::new(&self.script)
            .arg(cmd)
            .arg(container)
            .arg(name)
            .status()
            .await
            .map_err(|e| Error::other(format!("extfs '{}' {cmd}: {e}", self.prefix)))?;
        if status.success() {
            self.invalidate(container);
            Ok(())
        } else {
            Err(Error::other(format!(
                "extfs '{}' {cmd} failed (exit {status})",
                self.prefix
            )))
        }
    }
}

#[async_trait::async_trait]
impl Vfs for ExtfsFs {
    fn scheme(&self) -> &str {
        &self.prefix
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            // Mutations map to copyin/rm/mkdir/rmdir; a script that doesn't
            // support one returns nonzero and we surface a clean error.
            writable: true,
            permissions: false,
            ownership: false,
            symlinks: false,
            random_access: false,
            inode: false,
            server_rename: false, // extfs has no rename op
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
        let inner = inner_rel(path);
        let tmp = scratch_path("out");
        let status = Command::new(&self.script)
            .arg("copyout")
            .arg(&container)
            .arg(&inner)
            .arg(&tmp)
            .status()
            .await
            .map_err(|e| Error::other(format!("extfs '{}' copyout: {e}", self.prefix)))?;
        if !status.success() {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(Error::other(format!(
                "extfs '{}' copyout failed (exit {status})",
                self.prefix
            )));
        }
        let data = tokio::fs::read(&tmp).await?;
        let _ = tokio::fs::remove_file(&tmp).await;
        Ok(Box::new(MemReader::new(data)))
    }

    async fn open_write(&self, path: &VfsPath, _meta: WriteMeta) -> Result<BoxWrite> {
        let script = self.script.clone();
        let container = container_of(path)?.clone();
        let inner = inner_rel(path);
        let prefix = self.prefix.clone();
        // Buffer the incoming bytes to a scratch file, then hand it to `copyin`
        // when the writer is shut down. The container's mtime then changes, so
        // the next read re-lists (mtime-keyed cache) — no manual invalidation.
        Ok(pipe_upload(PIPE_CAP, move |mut rx| async move {
            let tmp = scratch_path("in");
            {
                let mut f = tokio::fs::File::create(&tmp).await?;
                tokio::io::copy(&mut rx, &mut f).await?;
                f.shutdown().await?;
            }
            let status = Command::new(&script)
                .arg("copyin")
                .arg(&container)
                .arg(&inner)
                .arg(&tmp)
                .status()
                .await?;
            let _ = tokio::fs::remove_file(&tmp).await;
            if !status.success() {
                return Err(std::io::Error::other(format!(
                    "extfs '{prefix}' copyin failed (exit {status})"
                )));
            }
            Ok(())
        }))
    }

    async fn mkdir(&self, path: &VfsPath) -> Result<()> {
        let container = container_of(path)?.clone();
        self.run_mut("mkdir", &container, &inner_rel(path)).await
    }

    async fn remove_file(&self, path: &VfsPath) -> Result<()> {
        let container = container_of(path)?.clone();
        self.run_mut("rm", &container, &inner_rel(path)).await
    }

    async fn remove_dir(&self, path: &VfsPath) -> Result<()> {
        let container = container_of(path)?.clone();
        self.run_mut("rmdir", &container, &inner_rel(path)).await
    }

    async fn rename(&self, _from: &VfsPath, _to: &VfsPath) -> Result<()> {
        // extfs has no rename op; moves fall back to copy+delete (the engine
        // handles that since `server_rename` is false).
        Err(Error::Unsupported)
    }
}

/// Parse one line of `SCRIPT list` output: a flat "modified `ls -l`" listing
/// (`PERMS LINKS OWNER GROUP SIZE DATETIME [PATH/]NAME [-> target]`). Returns
/// `None` for blank/`.`/`..`/malformed lines (which MC also silently drops).
fn parse_extfs_line(line: &str) -> Option<ExtfsListEntry> {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.is_empty() {
        return None;
    }
    let toks: Vec<&str> = line.split_whitespace().collect();
    if toks.len() < 6 {
        return None;
    }
    let perms = toks[0];
    if perms.len() < 10 {
        return None;
    }
    let kind = match perms.chars().next().unwrap() {
        'd' => VfsKind::Dir,
        'l' => VfsKind::Symlink,
        '-' => VfsKind::File,
        _ => VfsKind::Other,
    };
    let size = toks[4].parse::<u64>().unwrap_or(0);
    // DATETIME starts at token 5. A compound date (`MM-DD-YYYY hh:mm`,
    // `YYYY/MM/DD HH:MM:SS`, `YYYY-MM-DD hh:mm`) spans 2 tokens; the classic
    // `Mon DD hh:mm[:ss]` / `Mon DD YYYY` spans 3.
    let name_start = if toks[5].contains('-') || toks[5].contains('/') {
        7
    } else {
        8
    };
    if toks.len() <= name_start {
        return None;
    }
    let rest = toks[name_start..].join(" ");
    let (name, symlink_target) = if kind == VfsKind::Symlink {
        match rest.split_once(" -> ") {
            Some((n, t)) => (n.to_string(), Some(t.to_string())),
            None => (rest, None),
        }
    } else {
        (rest, None)
    };
    // MC never passes archive-root-relative names with a leading `./` or `/`;
    // strip both so the tree keys line up.
    let mut name = name.as_str();
    while let Some(stripped) = name.strip_prefix("./") {
        name = stripped;
    }
    let name = name.trim_matches('/');
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    Some(ExtfsListEntry {
        path: name.to_string(),
        kind,
        size,
        mode: Some(perms_to_mode(perms)),
        symlink_target,
    })
}

/// Insert a listed member (with its full relative path) into the dir map,
/// synthesizing any missing intermediate directories.
fn insert_extfs_path(dirs: &mut HashMap<String, Vec<ChildMeta>>, entry: &ExtfsListEntry) {
    let norm = entry.path.trim_matches('/');
    if norm.is_empty() {
        return;
    }
    let comps: Vec<&str> = norm.split('/').collect();
    let mut parent = "/".to_string();
    for (i, comp) in comps.iter().enumerate() {
        let is_last = i == comps.len() - 1;
        let child_norm = if parent == "/" {
            format!("/{comp}")
        } else {
            format!("{parent}/{comp}")
        };
        let kind = if is_last { entry.kind } else { VfsKind::Dir };
        let (csize, cmode, ctarget) = if is_last {
            (entry.size, entry.mode, entry.symlink_target.clone())
        } else {
            (0, None, None)
        };

        let list = dirs.entry(parent.clone()).or_default();
        if let Some(existing) = list.iter_mut().find(|c| c.name == **comp) {
            // A concrete file/symlink supersedes a synthesized dir placeholder.
            if is_last && kind != VfsKind::Dir {
                existing.kind = kind;
                existing.size = csize;
                existing.mode = cmode;
                existing.symlink_target = ctarget;
            }
        } else {
            list.push(ChildMeta {
                name: (*comp).to_string(),
                kind,
                size: csize,
                mode: cmode,
                symlink_target: ctarget,
            });
        }
        if kind == VfsKind::Dir {
            dirs.entry(child_norm.clone()).or_default();
        }
        parent = child_norm;
    }
}

/// Locate the executable `extfs.d` script for `prefix`, searching the MC system
/// directories and rat-commander's own `extfs.d/`. Unix-only; returns `None`
/// elsewhere (the feature is then inert).
#[cfg(unix)]
pub fn find_extfs_script(prefix: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share/mc/extfs.d"));
    }
    dirs.push(PathBuf::from("/usr/lib/mc/extfs.d"));
    dirs.push(PathBuf::from("/usr/libexec/mc/extfs.d"));
    dirs.push(PathBuf::from("/usr/local/libexec/mc/extfs.d"));
    if let Some(d) = crate::config::paths::extfs_dir() {
        dirs.push(d);
    }

    for dir in dirs {
        // The trailing `+` (for filesystems not tied to a file) is a script-name
        // attribute, not part of the prefix — accept either spelling.
        for name in [prefix.to_string(), format!("{prefix}+")] {
            let cand = dir.join(&name);
            if let Ok(meta) = std::fs::metadata(&cand)
                && meta.is_file()
                && meta.permissions().mode() & 0o111 != 0
            {
                return Some(cand);
            }
        }
    }
    None
}

#[cfg(not(unix))]
pub fn find_extfs_script(_prefix: &str) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> ExtfsListEntry {
        parse_extfs_line(line).expect("should parse")
    }

    #[test]
    fn parses_classic_file_line() {
        let e = parse("-rw-r--r-- 1 user group 1435 Mar 30 21:19 data/edit.html");
        assert_eq!(e.path, "data/edit.html");
        assert_eq!(e.kind, VfsKind::File);
        assert_eq!(e.size, 1435);
        assert_eq!(e.mode, Some(0o644));
        assert!(e.symlink_target.is_none());
    }

    #[test]
    fn parses_various_date_formats() {
        // Mon DD YYYY
        assert_eq!(parse("-rw-r--r-- 1 u g 10 Jan  5  2020 a.txt").path, "a.txt");
        // Mon DD hh:mm:ss
        assert_eq!(
            parse("-rw-r--r-- 1 u g 10 Jan  5 21:19:03 a.txt").path,
            "a.txt"
        );
        // MM-DD-YYYY hh:mm
        assert_eq!(parse("-rw-r--r-- 1 u g 10 03-30-2000 21:19 a.txt").path, "a.txt");
        // uzip's YYYY/MM/DD HH:MM:SS
        assert_eq!(
            parse("-rw-r--r-- 1 u g 10 2000/03/30 21:19:27 a.txt").path,
            "a.txt"
        );
        // ISO YYYY-MM-DD hh:mm
        assert_eq!(parse("-rw-r--r-- 1 u g 10 2000-03-30 21:19 a.txt").path, "a.txt");
    }

    #[test]
    fn parses_directory_and_leading_dot_slash() {
        let e = parse("drwxr-xr-x 2 u g 0 Mar 30 21:19 ./somedir/");
        assert_eq!(e.path, "somedir");
        assert_eq!(e.kind, VfsKind::Dir);
    }

    #[test]
    fn parses_symlink_target() {
        let e = parse("lrwxrwxrwx 1 u g 0 Mar 30 21:19 link -> ../target/file");
        assert_eq!(e.path, "link");
        assert_eq!(e.kind, VfsKind::Symlink);
        assert_eq!(e.symlink_target.as_deref(), Some("../target/file"));
    }

    #[test]
    fn parses_name_with_spaces() {
        let e = parse("-rw-r--r-- 1 u g 5 Mar 30 21:19 my file name.txt");
        assert_eq!(e.path, "my file name.txt");
    }

    #[test]
    fn rejects_dot_dotdot_blank_and_short() {
        assert!(parse_extfs_line("drwxr-xr-x 2 u g 0 Mar 30 21:19 .").is_none());
        assert!(parse_extfs_line("drwxr-xr-x 2 u g 0 Mar 30 21:19 ..").is_none());
        assert!(parse_extfs_line("").is_none());
        assert!(parse_extfs_line("garbage").is_none());
    }

    /// End-to-end against the real MC `uzip` script + `zip`/`unzip`, when they
    /// are installed. Skipped otherwise so CI without MC still passes.
    #[tokio::test]
    async fn real_uzip_list_and_copyout() {
        use tokio::io::AsyncReadExt;

        let script = super::find_extfs_script("uzip");
        let have_zip = std::process::Command::new("zip")
            .arg("-v")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        let (Some(script), true) = (script, have_zip) else {
            eprintln!("skipping real_uzip test: uzip script or zip not available");
            return;
        };

        // Build a small zip in a scratch dir.
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rc_extfs_it_{}_{nanos}", std::process::id()));
        std::fs::create_dir_all(root.join("somedir")).unwrap();
        std::fs::write(root.join("somedir/a.txt"), b"alpha content").unwrap();
        std::fs::write(root.join("readme"), b"top level readme").unwrap();
        let container = root.join("test.zip");
        let ok = std::process::Command::new("zip")
            .current_dir(&root)
            .arg("-r")
            .arg("test.zip")
            .arg("somedir")
            .arg("readme")
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "zip should create the archive");

        let fs = ExtfsFs::new("uzip".to_string(), script);

        // Root listing.
        let root_p = VfsPath::extfs("uzip", &container, "/");
        let mut names: Vec<String> = fs
            .read_dir(&root_p)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        names.sort();
        assert_eq!(names, vec!["readme", "somedir"]);

        // Nested listing.
        let sub = VfsPath::extfs("uzip", &container, "/somedir");
        let subnames: Vec<String> = fs
            .read_dir(&sub)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert!(subnames.contains(&"a.txt".to_string()));

        // copyout via open_read returns the exact bytes.
        let member = VfsPath::extfs("uzip", &container, "/somedir/a.txt");
        let mut r = fs.open_read(&member).await.unwrap();
        let mut buf = Vec::new();
        r.read_to_end(&mut buf).await.unwrap();
        assert_eq!(buf, b"alpha content");

        std::fs::remove_dir_all(&root).ok();
    }

    /// Exercise the write path (copyin + rm) against the real `uzip` script.
    #[tokio::test]
    async fn real_uzip_copyin_and_remove() {
        use crate::vfs::WriteMeta;
        use tokio::io::AsyncWriteExt;

        let script = super::find_extfs_script("uzip");
        let have_zip = std::process::Command::new("zip")
            .arg("-v")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        let (Some(script), true) = (script, have_zip) else {
            eprintln!("skipping real_uzip write test: uzip script or zip not available");
            return;
        };

        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rc_extfs_wt_{}_{nanos}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("seed"), b"seed").unwrap();
        let container = root.join("w.zip");
        assert!(std::process::Command::new("zip")
            .current_dir(&root)
            .arg("w.zip")
            .arg("seed")
            .output()
            .unwrap()
            .status
            .success());

        let fs = ExtfsFs::new("uzip".to_string(), script);

        // copyin a new member via open_write.
        let member = VfsPath::extfs("uzip", &container, "/added.txt");
        let mut w = fs.open_write(&member, WriteMeta::default()).await.unwrap();
        w.write_all(b"injected payload").await.unwrap();
        w.shutdown().await.unwrap(); // runs copyin
        let names: Vec<String> = fs
            .read_dir(&VfsPath::extfs("uzip", &container, "/"))
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert!(names.contains(&"added.txt".to_string()), "copyin should add the member: {names:?}");

        // rm the seed member.
        fs.remove_file(&VfsPath::extfs("uzip", &container, "/seed"))
            .await
            .unwrap();
        let after: Vec<String> = fs
            .read_dir(&VfsPath::extfs("uzip", &container, "/"))
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert!(!after.contains(&"seed".to_string()), "rm should remove the member: {after:?}");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn builds_tree_with_intermediate_dirs() {
        let mut dirs: HashMap<String, Vec<ChildMeta>> = HashMap::new();
        dirs.insert("/".to_string(), Vec::new());
        for line in [
            "-rw-r--r-- 1 u g 5 Mar 30 21:19 data/a.txt",
            "-rw-r--r-- 1 u g 4 Mar 30 21:19 data/b.txt",
            "-rw-r--r-- 1 u g 2 Mar 30 21:19 readme",
        ] {
            insert_extfs_path(&mut dirs, &parse(line));
        }
        let tree = ExtfsTree {
            mtime: None,
            dirs,
        };
        let mut root: Vec<String> = tree
            .read_dir("/")
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        root.sort();
        assert_eq!(root, vec!["data", "readme"]);
        let mut sub: Vec<String> = tree
            .read_dir("/data")
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        sub.sort();
        assert_eq!(sub, vec!["a.txt", "b.txt"]);
        assert_eq!(tree.stat("/readme").unwrap().kind, VfsKind::File);
        assert_eq!(tree.stat("/data").unwrap().kind, VfsKind::Dir);
    }
}
