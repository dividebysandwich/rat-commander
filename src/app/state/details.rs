//! Drives the "Details" panel view: figures out what the *other* panel points
//! at and, for a directory or multi-item selection, walks it in the background
//! (so remote/large trees stay responsive) to tally its recursive size.

use super::*;
use crate::details::{
    DetailsData, DetailsKind, FileInfo, Preview, PreviewImage, PreviewLine, PreviewTreeLine, Tally,
};
use std::sync::Arc;
use tokio::io::AsyncReadExt;

/// Bytes read from the head of a text file for the preview.
const HEAD_BYTES: usize = 16 * 1024;
/// Maximum lines highlighted for a text preview.
const HEAD_LINES: usize = 200;
/// Largest image (bytes) decoded for a thumbnail preview.
const MAX_IMAGE_BYTES: u64 = 30 * 1024 * 1024;
/// Largest archive (bytes) listed for a preview (avoids decompressing huge
/// tarballs just to hover over them).
const MAX_ARCHIVE_BYTES: u64 = 50 * 1024 * 1024;
/// Longest edge of a decoded thumbnail (the graphics layer fits it to the cell
/// area; the ASCII fallback downsamples further).
const THUMB_MAX: u32 = 480;
/// Directory-tree preview limits.
const MAX_TREE_DEPTH: u16 = 2;
const MAX_TREE_LINES: usize = 200;

/// What a Details panel should display, derived from the source panel.
enum Plan {
    Empty,
    File(VfsEntry),
    /// One or more roots to recursively size; `(name, kind, size)`.
    Tally { label: String, roots: Vec<(String, VfsKind, u64)> },
}

impl AppState {
    /// Refresh both panels' Details state. Cheap when nothing changed; called
    /// once per loop iteration (after every key/mouse/tick/event). Restarts the
    /// background size scan when the source panel's cursor/selection changes.
    pub fn update_details(&mut self) {
        for viewer in 0..2 {
            if self.panels[viewer].format != ViewFormat::Details {
                // Left Details mode: cancel any scan and forget the state.
                if self.details[viewer].cancel.is_some() || !self.details[viewer].key.is_empty() {
                    if let Some(c) = self.details[viewer].cancel.take() {
                        c.cancel();
                    }
                    self.details[viewer] = DetailsData::default();
                }
                continue;
            }
            let source = 1 - viewer;
            let key = self.details_key(source);
            if key != self.details[viewer].key {
                self.details[viewer].key = key;
                self.start_details(viewer);
            }
        }
    }

    /// A signature of what the source panel currently points at, so a change
    /// (navigation, cursor move, or selection edit) triggers a recompute.
    fn details_key(&self, source: usize) -> String {
        let p = &self.panels[source];
        if p.format == ViewFormat::Details {
            return "\u{0}details".to_string(); // source isn't a normal listing
        }
        let cursor = p.current_entry().map(|e| e.name.as_str()).unwrap_or("");
        format!("{}\u{0}{}\u{0}{}", p.cwd.display(), cursor, p.selection.signature())
    }

    /// (Re)build `details[viewer]` from the source panel, starting a background
    /// size scan when a directory or selection needs one.
    fn start_details(&mut self, viewer: usize) {
        let source = 1 - viewer;
        if let Some(c) = self.details[viewer].cancel.take() {
            c.cancel();
        }
        self.details[viewer].generation = self.details[viewer].generation.wrapping_add(1);
        let generation = self.details[viewer].generation;

        // Decide what to show (immutable borrow of the source panel only).
        let (cwd, backend, plan) = {
            let p = &self.panels[source];
            let plan = if p.format == ViewFormat::Details {
                Plan::Empty
            } else if !p.selection.is_empty() {
                let roots: Vec<(String, VfsKind, u64)> = p
                    .entries
                    .iter()
                    .filter(|e| e.name != ".." && p.selection.is_marked(&e.name))
                    .map(|e| (e.name.clone(), e.kind, e.size))
                    .collect();
                let label = format!(
                    "{} item{} selected",
                    roots.len(),
                    if roots.len() == 1 { "" } else { "s" }
                );
                Plan::Tally { label, roots }
            } else if let Some(e) = p.current_entry().filter(|e| e.name != "..") {
                if e.kind == VfsKind::Dir {
                    Plan::Tally {
                        label: format!("{}/", e.name),
                        roots: vec![(e.name.clone(), e.kind, e.size)],
                    }
                } else {
                    Plan::File(e.clone())
                }
            } else {
                Plan::Empty
            };
            (p.cwd.clone(), p.backend.clone(), plan)
        };

        match plan {
            Plan::Empty => self.details[viewer].kind = DetailsKind::Empty,
            Plan::File(e) => {
                let fi = self.file_info(&cwd, &e);
                self.details[viewer].kind = DetailsKind::File(fi);
            }
            Plan::Tally { label, roots } => {
                // Seed the immediate counts (files are sized straight away); only
                // real directories need the recursive background walk.
                let (mut total, mut files, mut dirs, mut has_dirs) = (0u64, 0u64, 0u64, false);
                for (_, kind, size) in &roots {
                    if *kind == VfsKind::Dir {
                        dirs += 1;
                        has_dirs = true;
                    } else {
                        files += 1;
                        total += size;
                    }
                }
                self.details[viewer].kind =
                    DetailsKind::Tally(Tally { label, total, files, dirs, scanning: has_dirs });
                if has_dirs {
                    let cancel = CancelToken::new();
                    self.details[viewer].cancel = Some(cancel.clone());
                    let roots: Vec<(VfsPath, VfsKind, u64)> =
                        roots.into_iter().map(|(n, k, s)| (cwd.join(&n), k, s)).collect();
                    let tx = self.tx.clone();
                    tokio::spawn(async move {
                        scan_tally(backend, roots, viewer, generation, cancel, tx).await;
                    });
                }
            }
        }
        // Kick off the preview load for whatever the source panel points at,
        // sharing this recompute's generation so a stale result is dropped.
        self.start_preview(viewer);
    }

    /// Start (or clear) the background preview for the item under the source
    /// panel's cursor. Uses the current `details[viewer].generation`.
    fn start_preview(&mut self, viewer: usize) {
        let source = 1 - viewer;
        let generation = self.details[viewer].generation;
        let dark = self.dark_ui();
        let (backend, spec) = {
            let p = &self.panels[source];
            // No preview for a Details source, a multi-item selection, or `..`.
            let spec = if p.format == ViewFormat::Details || !p.selection.is_empty() {
                None
            } else {
                p.current_entry()
                    .filter(|e| e.name != "..")
                    .map(|e| (p.cwd.join(&e.name), e.kind, e.name.clone(), e.size))
            };
            (p.backend.clone(), spec)
        };
        match spec {
            None => self.details[viewer].preview = Preview::None,
            Some((path, kind, name, size)) => {
                self.details[viewer].preview = Preview::Loading;
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let preview = build_preview(backend, path, kind, name, size, dark).await;
                    let _ = tx
                        .send(AppEvent::DetailsPreview { viewer, generation, preview: Box::new(preview) })
                        .await;
                });
            }
        }
    }

    /// Apply a completed preview load (ignored when a newer recompute superseded it).
    pub(in crate::app::state) fn apply_details_preview(
        &mut self,
        viewer: usize,
        generation: u64,
        preview: Preview,
    ) {
        let Some(d) = self.details.get_mut(viewer) else {
            return;
        };
        if d.generation != generation {
            return;
        }
        d.preview = preview;
    }

    /// Build the render-ready file overview, resolving owner/group names here
    /// (where the lookups live) so the renderer just formats strings.
    fn file_info(&self, cwd: &VfsPath, e: &VfsEntry) -> FileInfo {
        let owner = e
            .uid
            .map(|u| uid_name(u).unwrap_or_else(|| u.to_string()))
            .unwrap_or_else(|| "—".to_string());
        let group = e
            .gid
            .map(|g| gid_name(g).unwrap_or_else(|| g.to_string()))
            .unwrap_or_else(|| "—".to_string());
        FileInfo {
            name: e.name.clone(),
            dir: cwd.display(),
            kind: e.kind,
            size: e.size,
            mode: e.mode,
            owner,
            group,
            mtime: e.mtime,
            atime: e.atime,
            ctime: e.ctime,
            inode: e.inode,
            symlink_target: e.symlink_target.clone(),
        }
    }

    /// Apply a tally update from a background scan (ignored if stale).
    pub(in crate::app::state) fn apply_details_tally(
        &mut self,
        viewer: usize,
        generation: u64,
        total: u64,
        files: u64,
        dirs: u64,
        done: bool,
    ) {
        let Some(d) = self.details.get_mut(viewer) else {
            return;
        };
        if d.generation != generation {
            return;
        }
        if let DetailsKind::Tally(t) = &mut d.kind {
            t.total = total;
            t.files = files;
            t.dirs = dirs;
            t.scanning = !done;
        }
    }
}

/// Recursively walk `roots` (never following symlinks), accumulating the total
/// file size plus file/dir counts, sending throttled [`AppEvent::DetailsTally`]
/// updates until done or cancelled.
async fn scan_tally(
    backend: Arc<dyn Vfs>,
    roots: Vec<(VfsPath, VfsKind, u64)>,
    viewer: usize,
    generation: u64,
    cancel: CancelToken,
    tx: AppSender,
) {
    let (mut total, mut files, mut dirs) = (0u64, 0u64, 0u64);
    let mut stack: Vec<VfsPath> = Vec::new();
    for (path, kind, size) in roots {
        if kind == VfsKind::Dir {
            dirs += 1;
            stack.push(path);
        } else {
            files += 1;
            total += size;
        }
    }

    let mut last = std::time::Instant::now();
    while let Some(dir) = stack.pop() {
        if cancel.is_cancelled() {
            return; // a newer scan superseded this one; drop it silently
        }
        let entries = backend.read_dir(&dir).await.unwrap_or_default();
        for e in entries {
            if e.name == ".." || e.name == "." {
                continue;
            }
            if e.kind == VfsKind::Dir && e.symlink_target.is_none() {
                dirs += 1;
                stack.push(dir.join(&e.name));
            } else {
                files += 1;
                total += e.size;
            }
        }
        // Throttle to ~12 updates/sec so a deep local tree can't flood the loop.
        if last.elapsed() >= std::time::Duration::from_millis(80) {
            let _ = tx.try_send(AppEvent::DetailsTally {
                viewer,
                generation,
                total,
                files,
                dirs,
                done: false,
            });
            last = std::time::Instant::now();
        }
    }
    let _ = tx
        .send(AppEvent::DetailsTally { viewer, generation, total, files, dirs, done: true })
        .await;
}

/// Build a preview for the item at `path`: a directory → a shallow tree; an image
/// file → a decoded thumbnail; an archive → its top-level listing; anything else
/// text-like → a syntax-highlighted head. Never panics; unsupported/binary/failed
/// items yield [`Preview::None`].
async fn build_preview(
    backend: Arc<dyn Vfs>,
    path: VfsPath,
    kind: VfsKind,
    name: String,
    size: u64,
    dark: bool,
) -> Preview {
    if kind == VfsKind::Dir {
        return build_tree_preview(&backend, &path).await;
    }
    if kind != VfsKind::File {
        return Preview::None;
    }
    if crate::util::img::is_image_name(&name)
        && size <= MAX_IMAGE_BYTES
        && let Some(pi) = load_image_preview(&backend, &path).await
    {
        return Preview::Image(pi);
    }
    // Native archives on local disk: list the root without mounting a panel.
    if ArchiveFormat::from_name(&name).is_some()
        && path.scheme == "file"
        && let Some(list) = list_archive_preview(&path.path).await
    {
        return Preview::Archive(list);
    }
    build_text_preview(&backend, &path, &name, dark).await
}

/// Read up to `n` bytes from the head of `path`.
async fn read_head(backend: &Arc<dyn Vfs>, path: &VfsPath, n: usize) -> Option<Vec<u8>> {
    let reader = backend.open_read(path).await.ok()?;
    let mut buf = Vec::new();
    reader.take(n as u64).read_to_end(&mut buf).await.ok()?;
    Some(buf)
}

/// Decode an image into a thumbnail (aspect preserved) plus a short EXIF summary,
/// preferring an embedded EXIF thumbnail when present. The decode (CPU-heavy)
/// runs on the blocking pool.
async fn load_image_preview(backend: &Arc<dyn Vfs>, path: &VfsPath) -> Option<PreviewImage> {
    let bytes = read_head(backend, path, MAX_IMAGE_BYTES as usize).await?;
    let (thumb, exif) = tokio::task::spawn_blocking(move || {
        let thumb = crate::util::img::decode_scaled(&bytes, THUMB_MAX, true)?;
        Some((thumb, crate::util::img::exif_summary(&bytes)))
    })
    .await
    .ok()??;
    let sig = crate::util::img::image_sig(&thumb);
    Some(PreviewImage { img: thumb, sig, exif })
}

/// List a local archive's top-level entries (capped), or `None` when it is too
/// large or can't be read.
async fn list_archive_preview(container: &Path) -> Option<Vec<String>> {
    let meta = tokio::fs::metadata(container).await.ok()?;
    if meta.len() > MAX_ARCHIVE_BYTES {
        return None;
    }
    let root = VfsPath::archive(container.to_path_buf(), "/");
    let fs = crate::vfs::archive::ArchiveFs::new();
    let entries = fs.read_dir(&root).await.ok()?;
    let mut names: Vec<String> = entries
        .into_iter()
        .filter(|e| e.name != "..")
        .map(|e| if e.kind == VfsKind::Dir { format!("{}/", e.name) } else { e.name })
        .collect();
    names.sort();
    names.truncate(500);
    (!names.is_empty()).then_some(names)
}

/// Read the head of a text file and syntax-highlight it. Returns [`Preview::None`]
/// when the content looks binary.
async fn build_text_preview(
    backend: &Arc<dyn Vfs>,
    path: &VfsPath,
    name: &str,
    dark: bool,
) -> Preview {
    let Some(bytes) = read_head(backend, path, HEAD_BYTES).await else {
        return Preview::None;
    };
    // Binary heuristic: any NUL byte, or many U+FFFD replacements after a lossy
    // decode, means "not text".
    if bytes.contains(&0) {
        return Preview::None;
    }
    let text = String::from_utf8_lossy(&bytes);
    let total = text.chars().count().max(1);
    let bad = text.chars().filter(|&c| c == '\u{FFFD}').count();
    if bad * 20 > total {
        return Preview::None;
    }

    let mut hl = crate::syntax::Highlighter::for_file(name, dark);
    let mut lines: Vec<PreviewLine> = Vec::new();
    for (i, raw) in text.lines().take(HEAD_LINES).enumerate() {
        let display = expand_tabs(raw);
        let runs = if let Some(h) = hl.as_mut() {
            h.process_next(&display);
            h.line(i).to_vec()
        } else {
            Vec::new()
        };
        lines.push(PreviewLine { text: display, runs });
    }
    if lines.is_empty() {
        Preview::None
    } else {
        Preview::Text(lines)
    }
}

/// Expand tabs to 4-column tab stops (so highlight runs align to the display text).
fn expand_tabs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut col = 0usize;
    for c in s.chars() {
        if c == '\t' {
            let n = 4 - (col % 4);
            out.extend(std::iter::repeat_n(' ', n));
            col += n;
        } else {
            out.push(c);
            col += 1;
        }
    }
    out
}

/// Build a shallow directory tree (dirs first, alphabetical), bounded in depth
/// and line count.
async fn build_tree_preview(backend: &Arc<dyn Vfs>, dir: &VfsPath) -> Preview {
    let mut out: Vec<PreviewTreeLine> = Vec::new();
    walk_tree(backend, dir, 0, &mut out).await;
    if out.is_empty() {
        Preview::None
    } else {
        Preview::Tree(out)
    }
}

/// Recursive helper for [`build_tree_preview`] (boxed for async recursion).
fn walk_tree<'a>(
    backend: &'a Arc<dyn Vfs>,
    dir: &'a VfsPath,
    depth: u16,
    out: &'a mut Vec<PreviewTreeLine>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        if out.len() >= MAX_TREE_LINES {
            return;
        }
        let mut entries = backend.read_dir(dir).await.unwrap_or_default();
        entries.retain(|e| e.name != ".." && e.name != ".");
        // Directories first, then alphabetical.
        entries.sort_by(|a, b| {
            (b.kind == VfsKind::Dir)
                .cmp(&(a.kind == VfsKind::Dir))
                .then_with(|| a.name.cmp(&b.name))
        });
        for e in entries {
            if out.len() >= MAX_TREE_LINES {
                break;
            }
            let is_dir = e.kind == VfsKind::Dir;
            out.push(PreviewTreeLine { depth, name: e.name.clone(), is_dir });
            if is_dir && e.symlink_target.is_none() && depth < MAX_TREE_DEPTH {
                let sub = dir.join(&e.name);
                walk_tree(backend, &sub, depth + 1, out).await;
            }
        }
    })
}
