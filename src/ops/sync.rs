//! Directory synchronisation (mirror): walking two trees and planning the work
//! needed to reconcile them.
//!
//! Planning is split in two so the interesting half is testable without any I/O:
//! [`walk`] reads a tree into a `rel path → `[`SyncEntry`] map (over any VFS
//! backend — local, remote or archive), and [`plan`] is a pure function turning
//! two such maps into an ordered list of [`SyncStep`]s. The engine then executes
//! that list ([`OpKind::Sync`](super::OpKind::Sync)), so a mirror is one ordinary
//! task: it reports progress, can be cancelled, and can be sent to the background
//! like any copy.
//!
//! Sides are indexed `0` (the source panel) and `1` (the destination panel)
//! throughout, matching `OpRequest`'s `src_fs` / `dst_fs`.

use crate::util::Result;
use crate::vfs::{Vfs, VfsKind, VfsPath};
use futures::future::BoxFuture;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// Timestamps within this window count as equal. Filesystems disagree about
/// resolution (FAT rounds to 2 s, several remote backends report whole seconds),
/// so without a tolerance an unchanged file would be re-copied on every run.
const MTIME_TOLERANCE: Duration = Duration::from_secs(2);

/// How the two directories should be reconciled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncMode {
    /// Mirror source → destination. The source is authoritative: anything that
    /// differs is overwritten from it, and with `delete_extraneous` anything the
    /// source doesn't have is removed, leaving the destination an exact copy.
    OneWay { delete_extraneous: bool },
    /// Reconcile both ways: for a file on both sides the newer one wins, and a
    /// file missing from either side is copied over. Nothing is ever deleted.
    TwoWay,
}

impl SyncMode {
    fn deletes(self) -> bool {
        matches!(self, SyncMode::OneWay { delete_extraneous: true })
    }

    pub fn two_way(self) -> bool {
        matches!(self, SyncMode::TwoWay)
    }
}

/// Whether a walked tree carries no usable timestamps at all — it holds files,
/// but not one of them reports an mtime.
///
/// Some backends (FTP and SCP, whose listings don't surface a reliable time)
/// report `None` for every entry. Comparison then falls back to size alone, which
/// is fine for a one-way sync but makes "newer wins" undecidable, so the caller
/// refuses a two-way run rather than silently doing something else.
pub fn lacks_times(tree: &Tree) -> bool {
    let mut files = tree.values().filter(|e| !e.is_dir).peekable();
    files.peek().is_some() && !tree.values().any(|e| !e.is_dir && e.mtime.is_some())
}

/// A walked entry, keyed in the map by its path relative to the walk root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncEntry {
    pub size: u64,
    pub mtime: Option<SystemTime>,
    pub is_dir: bool,
}

/// A walked tree: `relative path → entry`. A `BTreeMap` so iteration is
/// lexicographic, which conveniently means a parent always precedes its children
/// (`"a"` < `"a/b"`) — the plan relies on that for both pruning and mkdir order.
pub type Tree = BTreeMap<String, SyncEntry>;

/// One step of a sync plan. `side` / `from` index the two backends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncStep {
    /// Create `path` on `side` — mirrors directories that would otherwise be
    /// lost because they hold no files.
    MkDir { side: usize, path: VfsPath, rel: String },
    /// Copy `src` (on side `from`) to `dst` (on the other side).
    Copy { from: usize, src: VfsPath, dst: VfsPath, rel: String, size: u64 },
    /// Recursively delete `path` on `side`. `files` is how many files that
    /// removes (the whole subtree for a directory), so progress totals add up.
    Delete { side: usize, path: VfsPath, rel: String, files: u64, size: u64 },
}

impl SyncStep {
    /// A one-line description for the preview list, e.g. `"→ sub/a.txt"`.
    pub fn label(&self) -> String {
        match self {
            SyncStep::MkDir { side, rel, .. } => {
                format!("{} mkdir {rel}/", if *side == 0 { "←" } else { "→" })
            }
            SyncStep::Copy { from, rel, .. } => {
                format!("{} {rel}", if *from == 0 { "→" } else { "←" })
            }
            SyncStep::Delete { rel, .. } => format!("✗ delete {rel}"),
        }
    }
}

/// Totals for the preview's summary line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncCounts {
    pub copies: u64,
    pub deletes: u64,
    pub mkdirs: u64,
    /// Bytes to transfer (copies only).
    pub bytes: u64,
}

/// A planned mirror: what to do, and enough context to describe it.
#[derive(Debug, Clone)]
pub struct SyncPlan {
    pub steps: Vec<SyncStep>,
    pub mode: SyncMode,
    /// Display labels for the two roots (the panel directories), for the preview.
    pub roots: [String; 2],
}

impl SyncPlan {
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    pub fn counts(&self) -> SyncCounts {
        let mut c = SyncCounts::default();
        for s in &self.steps {
            match s {
                SyncStep::MkDir { .. } => c.mkdirs += 1,
                SyncStep::Copy { size, .. } => {
                    c.copies += 1;
                    c.bytes += size;
                }
                SyncStep::Delete { files, .. } => c.deletes += files.max(&1),
            }
        }
        c
    }
}

/// How `a`'s timestamp relates to `b`'s, with anything inside
/// [`MTIME_TOLERANCE`] treated as equal. An unknown timestamp on either side
/// compares equal, so a backend that reports no mtime falls back to size alone.
fn cmp_mtime(a: Option<SystemTime>, b: Option<SystemTime>) -> Ordering {
    let (Some(a), Some(b)) = (a, b) else {
        return Ordering::Equal;
    };
    match a.duration_since(b) {
        Ok(d) if d > MTIME_TOLERANCE => Ordering::Greater,
        Ok(_) => Ordering::Equal,
        Err(e) if e.duration() > MTIME_TOLERANCE => Ordering::Less,
        Err(_) => Ordering::Equal,
    }
}

/// Whether two files need reconciling at all.
fn differs(a: &SyncEntry, b: &SyncEntry) -> bool {
    a.size != b.size || cmp_mtime(a.mtime, b.mtime) != Ordering::Equal
}

/// Whether `rel` lies inside any of `dirs` (which are being removed wholesale).
fn under_any(rel: &str, dirs: &[String]) -> bool {
    dirs.iter().any(|d| rel.len() > d.len() + 1 && rel.starts_with(d.as_str()) && rel.as_bytes()[d.len()] == b'/')
}

/// Files and bytes under `dir` in `tree` (the directory itself excluded).
fn subtree_totals(tree: &Tree, dir: &str) -> (u64, u64) {
    let mut files = 0;
    let mut bytes = 0;
    for (rel, e) in tree {
        if !e.is_dir && under_any(rel, std::slice::from_ref(&dir.to_string())) {
            files += 1;
            bytes += e.size;
        }
    }
    (files, bytes)
}

/// Whether `tree` holds `rel` as the same kind (file vs directory).
fn same_kind(tree: &Tree, rel: &str, is_dir: bool) -> bool {
    tree.get(rel).is_some_and(|e| e.is_dir == is_dir)
}

/// Reconcile two walked trees into an ordered list of steps.
///
/// Every relative path in either tree is classified by what each side holds
/// there (nothing / a file / a directory), which makes the rules exhaustive.
///
/// The result is ordered **clash-deletes → mkdirs → copies → extraneous-deletes**:
/// a destination entry standing where the source needs a different *kind* must go
/// before we write over it, directories must exist before files land inside them,
/// and merely-extraneous removals are left until last so a failed transfer never
/// costs data that was only about to be replaced. Within each phase the sorted
/// key order keeps parents ahead of children.
///
/// A *kind clash* — one side has a file where the other has a directory — is only
/// resolved when the mode may delete (the offending destination entry is removed
/// and replaced). Otherwise it is left alone rather than guessed at, so a two-way
/// sync can never destroy the losing side.
pub fn plan(a: &Tree, b: &Tree, roots: [&VfsPath; 2], mode: SyncMode) -> Vec<SyncStep> {
    // Deletes that must precede the write that replaces them, vs. those that are
    // just tidying the destination and are safest performed last.
    let mut clash_deletes: Vec<SyncStep> = Vec::new();
    let mut mkdirs: Vec<SyncStep> = Vec::new();
    let mut copies: Vec<SyncStep> = Vec::new();
    let mut extra_deletes: Vec<SyncStep> = Vec::new();
    // Subtrees not to descend into: either removed wholesale, or sitting under an
    // unresolved kind clash (nothing below a path can be reconciled while the two
    // sides disagree about what that path even is).
    //
    // Skipping a removed subtree is safe: the walk always records a parent, so if
    // the source had anything below such a path it would hold the directory too,
    // and the path would never have been pruned in the first place.
    let mut pruned: Vec<String> = Vec::new();

    let mkdir_step = |side: usize, rel: &str| SyncStep::MkDir {
        side,
        path: roots[side].join(rel),
        rel: rel.to_string(),
    };
    let copy_step = |from: usize, rel: &str, size: u64| SyncStep::Copy {
        from,
        src: roots[from].join(rel),
        dst: roots[1 - from].join(rel),
        rel: rel.to_string(),
        size,
    };
    let del_step = |rel: &str, e: &SyncEntry, tree: &Tree| {
        // A directory is removed recursively, so it accounts for its whole
        // subtree in the progress totals.
        let (files, size) =
            if e.is_dir { subtree_totals(tree, rel) } else { (1, e.size) };
        SyncStep::Delete { side: 1, path: roots[1].join(rel), rel: rel.to_string(), files, size }
    };

    // Every path either side knows about, parents first.
    let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
    keys.sort_unstable();
    keys.dedup();

    for rel in keys {
        if under_any(rel, &pruned) {
            continue;
        }
        match (a.get(rel), b.get(rel)) {
            // Source-only.
            (Some(ea), None) => {
                if ea.is_dir {
                    mkdirs.push(mkdir_step(1, rel));
                } else {
                    copies.push(copy_step(0, rel, ea.size));
                }
            }
            // Destination-only: two-way brings it back, a mirror removes it, and
            // an additive one-way leaves it be.
            (None, Some(eb)) => {
                if mode.two_way() {
                    if eb.is_dir {
                        mkdirs.push(mkdir_step(0, rel));
                    } else {
                        copies.push(copy_step(1, rel, eb.size));
                    }
                } else if mode.deletes() {
                    extra_deletes.push(del_step(rel, eb, b));
                    if eb.is_dir {
                        pruned.push(rel.clone());
                    }
                }
            }
            (Some(ea), Some(eb)) => match (ea.is_dir, eb.is_dir) {
                // Both directories: nothing to do here; their contents are
                // separate keys.
                (true, true) => {}
                (false, false) if differs(ea, eb) => {
                    // One-way: the source always wins. Two-way: the newer file
                    // does, and a tie (equal times, different sizes) goes to the
                    // source, so the active panel decides rather than chance.
                    let from = if mode.two_way() && cmp_mtime(ea.mtime, eb.mtime) == Ordering::Less
                    {
                        1
                    } else {
                        0
                    };
                    let size = if from == 0 { ea.size } else { eb.size };
                    copies.push(copy_step(from, rel, size));
                }
                (false, false) => {} // identical
                // Kind clash: only a mode that may delete can resolve it.
                _ => {
                    if mode.deletes() {
                        clash_deletes.push(del_step(rel, eb, b));
                        if ea.is_dir {
                            // The destination's file makes way for the directory;
                            // the source's own children are reconciled as usual.
                            mkdirs.push(mkdir_step(1, rel));
                        } else {
                            // The destination's directory is gone, so skip its
                            // contents and write the source's file in its place.
                            pruned.push(rel.clone());
                            copies.push(copy_step(0, rel, ea.size));
                        }
                    } else {
                        // Left as-is: neither side's children can be reconciled
                        // while the two disagree about what this path is.
                        pruned.push(rel.clone());
                    }
                }
            },
            (None, None) => unreachable!("keys come from one of the two trees"),
        }
    }

    clash_deletes.extend(mkdirs);
    clash_deletes.extend(copies);
    clash_deletes.extend(extra_deletes);
    clash_deletes
}

/// Read a whole tree into a [`Tree`], keyed by `/`-separated paths relative to
/// `root`. Symlinks are recorded as files, so they are recreated rather than
/// followed. Errors from unreadable subdirectories abort the walk — a partial
/// picture would plan wrong (and, with delete-extraneous, dangerously so).
pub async fn walk(fs: &Arc<dyn Vfs>, root: &VfsPath) -> Result<Tree> {
    let mut out = Tree::new();
    walk_into(fs, root, String::new(), &mut out).await?;
    Ok(out)
}

fn walk_into<'a>(
    fs: &'a Arc<dyn Vfs>,
    dir: &'a VfsPath,
    prefix: String,
    out: &'a mut Tree,
) -> BoxFuture<'a, Result<()>> {
    Box::pin(async move {
        for child in fs.read_dir(dir).await? {
            if child.name == ".." || child.name == "." {
                continue;
            }
            let rel = if prefix.is_empty() {
                child.name.clone()
            } else {
                format!("{prefix}/{}", child.name)
            };
            let is_dir = child.kind == VfsKind::Dir;
            out.insert(
                rel.clone(),
                SyncEntry { size: child.size, mtime: child.mtime, is_dir },
            );
            if is_dir {
                let path = dir.join(&child.name);
                walk_into(fs, &path, rel, out).await?;
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(size: u64, secs: u64) -> SyncEntry {
        SyncEntry {
            size,
            mtime: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs)),
            is_dir: false,
        }
    }

    fn dir() -> SyncEntry {
        SyncEntry { size: 0, mtime: None, is_dir: true }
    }

    fn tree(items: &[(&str, SyncEntry)]) -> Tree {
        items.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    fn roots() -> (VfsPath, VfsPath) {
        (VfsPath::local("/a"), VfsPath::local("/b"))
    }

    fn run(a: &Tree, b: &Tree, mode: SyncMode) -> Vec<SyncStep> {
        let (ra, rb) = roots();
        plan(a, b, [&ra, &rb], mode)
    }

    /// The `rel`s of every Copy step, with the side it comes from.
    fn copies(steps: &[SyncStep]) -> Vec<(usize, String)> {
        steps
            .iter()
            .filter_map(|s| match s {
                SyncStep::Copy { from, rel, .. } => Some((*from, rel.clone())),
                _ => None,
            })
            .collect()
    }

    fn deletes(steps: &[SyncStep]) -> Vec<String> {
        steps
            .iter()
            .filter_map(|s| match s {
                SyncStep::Delete { rel, .. } => Some(rel.clone()),
                _ => None,
            })
            .collect()
    }

    const MIRROR: SyncMode = SyncMode::OneWay { delete_extraneous: true };
    const ADDITIVE: SyncMode = SyncMode::OneWay { delete_extraneous: false };

    #[test]
    fn identical_trees_need_no_work() {
        let a = tree(&[("x.txt", file(10, 100)), ("sub", dir()), ("sub/y", file(5, 100))]);
        let b = a.clone();
        for mode in [MIRROR, ADDITIVE, SyncMode::TwoWay] {
            assert!(run(&a, &b, mode).is_empty(), "{mode:?} should plan nothing");
        }
    }

    #[test]
    fn one_way_copies_new_and_changed_files() {
        let a = tree(&[
            ("new.txt", file(10, 100)),
            ("same.txt", file(10, 100)),
            ("bigger.txt", file(20, 100)),
            ("newer.txt", file(10, 500)),
        ]);
        let b = tree(&[
            ("same.txt", file(10, 100)),
            ("bigger.txt", file(10, 100)),
            ("newer.txt", file(10, 100)),
        ]);
        let steps = run(&a, &b, ADDITIVE);
        let mut got = copies(&steps);
        got.sort();
        assert_eq!(
            got,
            vec![
                (0, "bigger.txt".to_string()),
                (0, "new.txt".to_string()),
                (0, "newer.txt".to_string())
            ],
            "identical files are skipped; size or time differences copy source→dest"
        );
        assert!(deletes(&steps).is_empty(), "no deletes without delete-extraneous");
    }

    #[test]
    fn one_way_source_wins_even_when_the_destination_is_newer() {
        // A mirror makes the destination match the source, so a newer file on the
        // destination is still overwritten (that's the difference from two-way).
        let a = tree(&[("f", file(10, 100))]);
        let b = tree(&[("f", file(10, 900))]);
        assert_eq!(copies(&run(&a, &b, ADDITIVE)), vec![(0, "f".to_string())]);
    }

    #[test]
    fn delete_extraneous_removes_only_what_the_source_lacks() {
        let a = tree(&[("keep.txt", file(1, 100))]);
        let b = tree(&[("keep.txt", file(1, 100)), ("gone.txt", file(1, 100))]);
        let steps = run(&a, &b, MIRROR);
        assert_eq!(deletes(&steps), vec!["gone.txt"]);
        assert!(copies(&steps).is_empty(), "the identical file is not re-copied");
        // Without the flag the extraneous file is simply left alone.
        assert!(run(&a, &b, ADDITIVE).is_empty());
    }

    #[test]
    fn an_extraneous_directory_is_deleted_once_not_per_file() {
        let a = tree(&[("keep", file(1, 100))]);
        let b = tree(&[
            ("keep", file(1, 100)),
            ("old", dir()),
            ("old/a", file(3, 100)),
            ("old/deep", dir()),
            ("old/deep/b", file(4, 100)),
        ]);
        let steps = run(&a, &b, MIRROR);
        // One recursive delete for the top directory — not its children too.
        assert_eq!(deletes(&steps), vec!["old"], "nested entries are pruned");
        // Its file count feeds the progress totals.
        let counts = SyncPlan { steps, mode: MIRROR, roots: ["a".into(), "b".into()] }.counts();
        assert_eq!(counts.deletes, 2, "both files under old/ are counted");
    }

    #[test]
    fn two_way_lets_the_newer_side_win_and_never_deletes() {
        let a = tree(&[
            ("a_only", file(1, 100)),
            ("a_newer", file(5, 900)),
            ("b_newer", file(5, 100)),
        ]);
        let b = tree(&[
            ("b_only", file(1, 100)),
            ("a_newer", file(5, 100)),
            ("b_newer", file(5, 900)),
        ]);
        let steps = run(&a, &b, SyncMode::TwoWay);
        let mut got = copies(&steps);
        got.sort();
        assert_eq!(
            got,
            vec![
                (0, "a_newer".to_string()),
                (0, "a_only".to_string()),
                (1, "b_newer".to_string()),
                (1, "b_only".to_string()),
            ]
        );
        assert!(deletes(&steps).is_empty(), "two-way never deletes");
    }

    #[test]
    fn two_way_breaks_a_timestamp_tie_in_the_sources_favour() {
        // Same mtime but different sizes: the active (source) panel wins, rather
        // than picking arbitrarily.
        let a = tree(&[("f", file(10, 100))]);
        let b = tree(&[("f", file(99, 100))]);
        assert_eq!(copies(&run(&a, &b, SyncMode::TwoWay)), vec![(0, "f".to_string())]);
    }

    #[test]
    fn close_timestamps_count_as_unchanged() {
        // Within the tolerance (coarse filesystem clocks) → no work at all.
        let a = tree(&[("f", file(10, 100))]);
        let b = tree(&[("f", file(10, 101))]);
        assert!(run(&a, &b, MIRROR).is_empty(), "1s apart is within tolerance");
        // Beyond it, the file is reconciled.
        let b = tree(&[("f", file(10, 110))]);
        assert_eq!(copies(&run(&a, &b, ADDITIVE)).len(), 1);
    }

    #[test]
    fn a_missing_mtime_falls_back_to_size() {
        let no_time = SyncEntry { size: 10, mtime: None, is_dir: false };
        let a = tree(&[("f", no_time.clone())]);
        let b = tree(&[("f", no_time)]);
        assert!(run(&a, &b, MIRROR).is_empty(), "same size, unknown times → unchanged");
        let b = tree(&[("f", SyncEntry { size: 11, mtime: None, is_dir: false })]);
        assert_eq!(copies(&run(&a, &b, MIRROR)).len(), 1, "size still decides");
    }

    #[test]
    fn empty_directories_are_mirrored() {
        let a = tree(&[("empty", dir())]);
        let b = tree(&[]);
        let steps = run(&a, &b, ADDITIVE);
        assert!(
            matches!(steps.as_slice(), [SyncStep::MkDir { side: 1, rel, .. }] if rel == "empty"),
            "an empty source directory still gets created: {steps:?}"
        );
    }

    #[test]
    fn steps_are_ordered_mkdirs_then_copies_then_deletes() {
        let a = tree(&[("d", dir()), ("d/f", file(1, 100))]);
        let b = tree(&[("stale", file(1, 100))]);
        let steps = run(&a, &b, MIRROR);
        let rank = |s: &SyncStep| match s {
            SyncStep::MkDir { .. } => 0,
            SyncStep::Copy { .. } => 1,
            SyncStep::Delete { .. } => 2,
        };
        let ranks: Vec<u8> = steps.iter().map(rank).collect();
        assert!(ranks.windows(2).all(|w| w[0] <= w[1]), "ordered by phase: {ranks:?}");
        // The directory is created before the file that lands inside it.
        assert!(matches!(steps.first(), Some(SyncStep::MkDir { rel, .. }) if rel == "d"));
        // An extraneous delete is left until after the transfers, so a failed
        // copy never costs data.
        assert!(matches!(steps.last(), Some(SyncStep::Delete { rel, .. }) if rel == "stale"));
    }

    #[test]
    fn a_clash_delete_is_ordered_before_the_write_that_replaces_it() {
        // The destination holds a directory where the source has a file: the
        // delete must land *first*, or the copy would be writing onto a directory.
        let a = tree(&[("x", file(5, 100)), ("z", dir())]);
        let b = tree(&[("x", dir()), ("x/inner", file(1, 100)), ("z", file(9, 100))]);
        let steps = run(&a, &b, MIRROR);
        let pos = |pred: fn(&SyncStep) -> bool| steps.iter().position(pred).expect("step present");
        let del_x = pos(|s| matches!(s, SyncStep::Delete { rel, .. } if rel == "x"));
        let copy_x = pos(|s| matches!(s, SyncStep::Copy { rel, .. } if rel == "x"));
        assert!(del_x < copy_x, "delete x/ before writing the file x: {steps:?}");
        // Mirrored the other way round: the destination's file `z` goes before the
        // directory is created in its place.
        let del_z = pos(|s| matches!(s, SyncStep::Delete { rel, .. } if rel == "z"));
        let mkdir_z = pos(|s| matches!(s, SyncStep::MkDir { rel, .. } if rel == "z"));
        assert!(del_z < mkdir_z, "delete the file z before mkdir z/: {steps:?}");
    }

    #[test]
    fn paths_are_resolved_against_each_root() {
        let a = tree(&[("sub", dir()), ("sub/f.txt", file(1, 100))]);
        let b = tree(&[]);
        let steps = run(&a, &b, ADDITIVE);
        let copy = steps
            .iter()
            .find_map(|s| match s {
                SyncStep::Copy { src, dst, .. } => Some((src.clone(), dst.clone())),
                _ => None,
            })
            .expect("a copy step");
        assert_eq!(copy.0, VfsPath::local("/a/sub/f.txt"));
        assert_eq!(copy.1, VfsPath::local("/b/sub/f.txt"));
    }

    #[test]
    fn counts_summarise_the_plan() {
        let a = tree(&[("new", file(100, 100)), ("d", dir())]);
        let b = tree(&[("stale", file(7, 100))]);
        let steps = run(&a, &b, MIRROR);
        let c = SyncPlan { steps, mode: MIRROR, roots: ["a".into(), "b".into()] }.counts();
        assert_eq!((c.copies, c.deletes, c.mkdirs, c.bytes), (1, 1, 1, 100));
    }

    #[test]
    fn lacks_times_spots_a_backend_that_reports_none() {
        let timeless = SyncEntry { size: 1, mtime: None, is_dir: false };
        // FTP/SCP-style: files, but not a timestamp among them.
        assert!(lacks_times(&tree(&[("a", timeless.clone()), ("b", timeless.clone())])));
        // A normal tree, and one where only some entries lack times, are fine.
        assert!(!lacks_times(&tree(&[("a", file(1, 100))])));
        assert!(!lacks_times(&tree(&[("a", timeless), ("b", file(1, 100))])));
        // An empty tree (or one holding only directories) isn't "timeless" — there
        // is simply nothing to compare, so it must not block a two-way sync.
        assert!(!lacks_times(&tree(&[])));
        assert!(!lacks_times(&tree(&[("d", dir())])));
    }

    #[test]
    fn without_timestamps_comparison_falls_back_to_size() {
        // A one-way sync over a timeless backend still converges: same size ⇒ no
        // work, different size ⇒ copy. (Same-size edits are missed — documented.)
        let t = |size| SyncEntry { size, mtime: None, is_dir: false };
        let a = tree(&[("same", t(10)), ("grew", t(20))]);
        let b = tree(&[("same", t(10)), ("grew", t(5))]);
        assert_eq!(copies(&run(&a, &b, ADDITIVE)), vec![(0, "grew".to_string())]);
    }

    #[test]
    fn under_any_matches_only_real_children() {
        let dirs = vec!["old".to_string()];
        assert!(under_any("old/a", &dirs));
        assert!(under_any("old/deep/b", &dirs));
        assert!(!under_any("old", &dirs), "the directory itself is not under itself");
        assert!(!under_any("older/a", &dirs), "a name prefix is not a path prefix");
        assert!(!under_any("new/a", &dirs));
    }

    #[test]
    fn a_kind_clash_is_replaced_only_when_deleting_is_allowed() {
        // The source has a file where the destination has a directory.
        let a = tree(&[("x", file(5, 100))]);
        let b = tree(&[("x", dir()), ("x/inner", file(1, 100))]);
        // Mirror: remove the directory, then write the file.
        let steps = run(&a, &b, MIRROR);
        assert_eq!(deletes(&steps), vec!["x"]);
        assert_eq!(copies(&steps), vec![(0, "x".to_string())]);
        // Without deletes we refuse to guess and leave both sides untouched.
        assert!(run(&a, &b, ADDITIVE).is_empty());
        assert!(run(&a, &b, SyncMode::TwoWay).is_empty(), "two-way never destroys a side");
    }
}
