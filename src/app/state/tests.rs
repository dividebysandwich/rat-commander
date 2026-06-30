use super::*;
use crate::util::async_bridge;

#[tokio::test]
async fn enters_zip_archive_and_lists_contents() {
    // Build a temp dir with a zip to browse.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_nav_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("sub/file.txt"), b"hi").unwrap();
    std::fs::write(root.join("top.txt"), b"top").unwrap();
    let zip = root.join("test.zip");
    archive::create_archive(
        ArchiveFormat::Zip,
        &zip,
        &[root.join("sub"), root.join("top.txt")],
    )
    .unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();

    // Put the cursor on the zip and "enter" it.
    let idx = st.panels[0]
        .entries
        .iter()
        .position(|e| e.name == "test.zip")
        .unwrap();
    st.panels[0].cursor = idx;
    st.active = 0;
    st.enter_dir().await;

    assert!(st.panels[0].cwd.is_archive(), "should be inside the archive");
    let names: Vec<String> = st.panels[0].entries.iter().map(|e| e.name.clone()).collect();
    assert!(names.contains(&"sub".to_string()), "names: {names:?}");
    assert!(names.contains(&"top.txt".to_string()), "names: {names:?}");
    assert!(names.contains(&"..".to_string()), "archive has parent link");

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(unix)]
#[tokio::test]
async fn cannot_enter_unreadable_directory() {
    use std::os::unix::fs::PermissionsExt;

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_perm_{}_{nanos}", std::process::id()));
    let secret = root.join("secret");
    std::fs::create_dir_all(&secret).unwrap();
    std::fs::write(root.join("visible.txt"), b"hi").unwrap();
    // Remove all permissions on the subdirectory.
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o000)).unwrap();

    // If we can still read it (e.g. running as root), the scenario doesn't
    // apply — skip rather than assert a false negative.
    let denied = std::fs::read_dir(&secret).is_err();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    st.active = 0;

    let idx = st.panels[0]
        .entries
        .iter()
        .position(|e| e.name == "secret")
        .unwrap();
    st.panels[0].cursor = idx;
    st.enter_dir().await;

    if denied {
        assert_eq!(
            st.panels[0].cwd.path, root,
            "should not have entered the unreadable directory"
        );
        assert!(st.panels[0].error.is_none(), "no error should be left behind");
        // The listing is intact so the user can keep navigating.
        assert!(
            st.panels[0].entries.iter().any(|e| e.name == "visible.txt"),
            "panel listing should be preserved"
        );
    }

    // Restore permissions so cleanup can remove the tree.
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o755)).ok();
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn resolve_dest_preserves_remote_backend() {
    use std::path::PathBuf;
    let remote = VfsPath {
        scheme: "scp-0".to_string(),
        path: PathBuf::from("/home/user"),
        container: None,
    };
    // The unchanged (absolute) remote path stays on the remote backend.
    let d = resolve_dest_on("/home/user", &remote);
    assert_eq!(d.scheme, "scp-0");
    assert_eq!(d.path, PathBuf::from("/home/user"));
    // A relative entry joins the remote cwd (still remote).
    let d = resolve_dest_on("uploads", &remote);
    assert_eq!(d.scheme, "scp-0");
    assert_eq!(d.path, PathBuf::from("/home/user/uploads"));
    // A local base resolves to a local path.
    let local = VfsPath::local("/a/b");
    assert_eq!(resolve_dest_on("/c", &local).scheme, "file");
    assert_eq!(resolve_dest_on("sub", &local).path, PathBuf::from("/a/b/sub"));
}

#[test]
fn split_scheme_recognizes_only_real_schemes() {
    assert_eq!(split_scheme("scp-0:///srv/x"), Some(("scp-0", "/srv/x")));
    assert_eq!(split_scheme("sftp-2://rel"), Some(("sftp-2", "rel")));
    assert_eq!(split_scheme("/home/user"), None);
    assert_eq!(split_scheme("relative/path"), None);
    assert_eq!(split_scheme("://nope"), None);
}

#[test]
fn dest_override_remote_to_local() {
    use std::path::PathBuf;
    let remote = VfsPath { scheme: "scp-0".into(), path: PathBuf::from("/home/user"), container: None };
    let local_src = VfsPath::local("/data");

    // Keeping the scheme prefix stays on the remote backend.
    let d = dest_vfspath("scp-0:///srv/up", &remote, &local_src);
    assert_eq!(d.scheme, "scp-0");
    assert_eq!(d.path, PathBuf::from("/srv/up"));
    // A relative remote path joins the matching panel's cwd.
    let d = dest_vfspath("scp-0://uploads", &remote, &local_src);
    assert_eq!((d.scheme.as_str(), d.path), ("scp-0", PathBuf::from("/home/user/uploads")));

    // Dropping the scheme on a remote dest → local (absolute kept as-is).
    let d = dest_vfspath("/tmp/out", &remote, &local_src);
    assert_eq!((d.scheme.as_str(), d.path), ("file", PathBuf::from("/tmp/out")));
    // …and a relative one joins the (local) source panel's directory.
    let d = dest_vfspath("out", &remote, &local_src);
    assert_eq!((d.scheme.as_str(), d.path), ("file", PathBuf::from("/data/out")));

    // A bare path to a *local* destination panel behaves exactly as before.
    let local_dest = VfsPath::local("/a/b");
    let d = dest_vfspath("sub", &local_dest, &remote);
    assert_eq!((d.scheme.as_str(), d.path), ("file", PathBuf::from("/a/b/sub")));
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_dialog_prefilled_from_cursor_and_other_panel() {
    use crate::ui::dialog::{DialogResult, Submit};
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_sym_{}_{nanos}", std::process::id()));
    let src = root.join("src");
    let dest = root.join("dest");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(src.join("doc.txt"), b"x").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&src);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    st.panels[1].cwd = VfsPath::local(&dest);
    st.panels[1].backend = st.registry.local();
    let idx = st.panels[0].entries.iter().position(|e| e.name == "doc.txt").unwrap();
    st.panels[0].cursor = idx;

    st.open_symlink();
    let dlg = st.dialog.as_mut().expect("symlink dialog");
    match dlg.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) {
        DialogResult::Submit(Submit::Symlink { dir, target, name }) => {
            assert_eq!(name, "doc.txt", "link name defaults to the file");
            assert_eq!(target, src.join("doc.txt").to_string_lossy(), "target = file path");
            assert_eq!(dir.path, dest, "link is created in the other panel");
        }
        _ => panic!("expected a Symlink submit"),
    }
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn compare_dirs_marks_by_mode() {
    use std::collections::HashSet;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_cmp_{}_{nanos}", std::process::id()));
    let da = root.join("a");
    let db = root.join("b");
    std::fs::create_dir_all(&da).unwrap();
    std::fs::create_dir_all(&db).unwrap();
    std::fs::write(da.join("same.txt"), b"hello").unwrap();
    std::fs::write(db.join("same.txt"), b"hello").unwrap();
    std::fs::write(da.join("big.txt"), b"AAAA").unwrap(); // larger in A
    std::fs::write(db.join("big.txt"), b"AA").unwrap();
    std::fs::write(da.join("onlyA.txt"), b"x").unwrap();
    std::fs::write(db.join("onlyB.txt"), b"y").unwrap();
    std::fs::write(da.join("diff.txt"), b"abc").unwrap(); // same size, diff content
    std::fs::write(db.join("diff.txt"), b"xyz").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.panels[0].cwd = VfsPath::local(&da);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    st.panels[1].cwd = VfsPath::local(&db);
    st.panels[1].backend = st.registry.local();
    st.panels[1].reload().await.unwrap();

    let marked = |p: &Panel| -> HashSet<String> {
        p.entries
            .iter()
            .filter(|e| p.selection.is_marked(&e.name))
            .map(|e| e.name.clone())
            .collect()
    };
    let set = |names: &[&str]| -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    };

    st.compare_dirs(CompareMode::Quick).await;
    assert_eq!(marked(&st.panels[0]), set(&["onlyA.txt"]));
    assert_eq!(marked(&st.panels[1]), set(&["onlyB.txt"]));

    st.compare_dirs(CompareMode::Size).await;
    assert_eq!(marked(&st.panels[0]), set(&["onlyA.txt", "big.txt"]));
    assert_eq!(marked(&st.panels[1]), set(&["onlyB.txt"]));

    st.compare_dirs(CompareMode::Content).await;
    assert_eq!(marked(&st.panels[0]), set(&["onlyA.txt", "big.txt", "diff.txt"]));
    assert_eq!(marked(&st.panels[1]), set(&["onlyB.txt", "big.txt", "diff.txt"]));

    std::fs::remove_dir_all(&root).ok();
}

/// Run a spawned background task to completion, applying its events (and the
/// final `DuplicatesFound`) to `st`.
async fn drain_duplicates(st: &mut AppState, rx: &mut crate::util::async_bridge::AppReceiver) {
    loop {
        let ev = rx.recv().await.unwrap();
        let done = matches!(ev, AppEvent::DuplicatesFound { .. });
        st.apply_event(ev).await;
        if done {
            break;
        }
    }
}

/// Run a details size-scan to completion, applying its tally events to `st`.
async fn drain_details(st: &mut AppState, rx: &mut crate::util::async_bridge::AppReceiver) {
    loop {
        let ev = rx.recv().await.unwrap();
        let done = matches!(ev, AppEvent::DetailsTally { done: true, .. });
        st.apply_event(ev).await;
        if done {
            break;
        }
    }
}

#[tokio::test]
async fn details_view_files_dirs_and_selection() {
    use crate::details::DetailsKind;
    use crate::panel::ViewFormat;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_details_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(root.join("sub/deep")).unwrap();
    std::fs::write(root.join("a.txt"), vec![0u8; 100]).unwrap();
    std::fs::write(root.join("b.txt"), vec![0u8; 200]).unwrap();
    std::fs::write(root.join("sub/c.bin"), vec![0u8; 1000]).unwrap();
    std::fs::write(root.join("sub/deep/d.bin"), vec![0u8; 4000]).unwrap();

    let (tx, mut rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    st.panels[1].format = ViewFormat::Details; // right panel shows details of left

    let cursor_on = |st: &mut AppState, name: &str| {
        let i = st.panels[0].entries.iter().position(|e| e.name == name).unwrap();
        st.panels[0].cursor = i;
        st.panels[0].selection.clear();
    };

    // Cursor on a file → a File overview (no scan).
    cursor_on(&mut st, "a.txt");
    st.update_details();
    match &st.details[1].kind {
        DetailsKind::File(fi) => {
            assert_eq!(fi.name, "a.txt");
            assert_eq!(fi.size, 100);
        }
        _ => panic!("expected a file overview"),
    }

    // Cursor on a directory → recursive tally (2 dirs: sub + deep; 2 files; 5000 bytes).
    cursor_on(&mut st, "sub");
    st.update_details();
    drain_details(&mut st, &mut rx).await;
    match &st.details[1].kind {
        DetailsKind::Tally(t) => {
            assert!(!t.scanning, "scan finished");
            assert_eq!(t.total, 5000);
            assert_eq!(t.files, 2);
            assert_eq!(t.dirs, 2);
        }
        _ => panic!("expected a tally"),
    }

    // Selection of a file + a directory → combined tally.
    cursor_on(&mut st, "sub");
    st.panels[0].selection.mark("a.txt");
    st.panels[0].selection.mark("sub");
    st.update_details();
    drain_details(&mut st, &mut rx).await;
    match &st.details[1].kind {
        DetailsKind::Tally(t) => {
            assert_eq!(t.total, 5100, "a.txt (100) + sub tree (5000)");
            assert_eq!(t.files, 3, "a.txt + c.bin + d.bin");
            assert_eq!(t.dirs, 2, "sub + deep");
        }
        _ => panic!("expected a tally"),
    }

    // Leaving Details mode cancels and clears the state.
    st.panels[1].format = ViewFormat::Full;
    st.update_details();
    assert!(matches!(st.details[1].kind, DetailsKind::Empty));

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn find_duplicates_marks_by_criteria() {
    use crate::ui::dialog::DupCriteria;
    use std::collections::HashSet;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_dups_{}_{nanos}", std::process::id()));
    let da = root.join("a");
    let db = root.join("b");
    std::fs::create_dir_all(&da).unwrap();
    std::fs::create_dir_all(&db).unwrap();
    std::fs::write(da.join("same.txt"), b"hello").unwrap(); // identical
    std::fs::write(db.join("same.txt"), b"hello").unwrap();
    std::fs::write(da.join("diff.txt"), b"abc").unwrap(); // same size, different bytes
    std::fs::write(db.join("diff.txt"), b"xyz").unwrap();
    std::fs::write(da.join("big.txt"), b"AAAA").unwrap(); // different size
    std::fs::write(db.join("big.txt"), b"AA").unwrap();
    std::fs::write(da.join("onlyA.txt"), b"x").unwrap(); // present on one side only
    std::fs::write(db.join("onlyB.txt"), b"y").unwrap();
    std::fs::write(da.join("Case.txt"), b"z").unwrap(); // name differs only by case
    std::fs::write(db.join("case.txt"), b"z").unwrap();

    let (tx, mut rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.panels[0].cwd = VfsPath::local(&da);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    st.panels[1].cwd = VfsPath::local(&db);
    st.panels[1].backend = st.registry.local();
    st.panels[1].reload().await.unwrap();

    let marked = |p: &Panel| -> HashSet<String> {
        p.entries.iter().filter(|e| p.selection.is_marked(&e.name)).map(|e| e.name.clone()).collect()
    };
    let set = |names: &[&str]| -> HashSet<String> { names.iter().map(|s| s.to_string()).collect() };
    let crit = |size, date, content, cs| DupCriteria { size, date, content, case_sensitive: cs };

    // Names only (case-sensitive): every same-named file is a duplicate, but
    // Case.txt / case.txt differ in case so they don't match.
    st.start_find_duplicates(crit(false, false, false, true));
    drain_duplicates(&mut st, &mut rx).await;
    assert_eq!(marked(&st.panels[0]), set(&["same.txt", "diff.txt", "big.txt"]));
    assert_eq!(marked(&st.panels[1]), set(&["same.txt", "diff.txt", "big.txt"]));

    // By content: only files with identical bytes count (diff differs; big's
    // size already rules it out without a read).
    st.start_find_duplicates(crit(false, false, true, true));
    drain_duplicates(&mut st, &mut rx).await;
    assert_eq!(marked(&st.panels[0]), set(&["same.txt"]));
    assert_eq!(marked(&st.panels[1]), set(&["same.txt"]));

    // By size: same and diff share a size; big does not.
    st.start_find_duplicates(crit(true, false, false, true));
    drain_duplicates(&mut st, &mut rx).await;
    assert_eq!(marked(&st.panels[0]), set(&["same.txt", "diff.txt"]));

    // Case-insensitive names: now Case.txt and case.txt match as well.
    st.start_find_duplicates(crit(false, false, false, false));
    drain_duplicates(&mut st, &mut rx).await;
    assert!(marked(&st.panels[0]).contains("Case.txt"), "case-insensitive name match (left)");
    assert!(marked(&st.panels[1]).contains("case.txt"), "case-insensitive name match (right)");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn parse_cd_recognizes_the_builtin() {
    assert_eq!(parse_cd("cd"), Some(""));
    assert_eq!(parse_cd("cd /tmp"), Some("/tmp"));
    assert_eq!(parse_cd("  cd   foo  "), Some("foo"));
    assert_eq!(parse_cd("cdfoo"), None);
    assert_eq!(parse_cd("ls"), None);
}

#[test]
fn normalize_path_resolves_dotdot() {
    assert_eq!(normalize_path(Path::new("/a/b/../c")), PathBuf::from("/a/c"));
    assert_eq!(normalize_path(Path::new("/a/./b")), PathBuf::from("/a/b"));
}

#[tokio::test]
async fn cd_changes_active_panel_directory() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_cd_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(root.join("child")).unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();

    // Relative cd descends.
    st.change_dir("child").await;
    assert_eq!(st.panels[0].cwd.path, root.join("child"));
    // `cd ..` ascends back.
    st.change_dir("..").await;
    assert_eq!(st.panels[0].cwd.path, root);
    // cd to a non-existent directory leaves the panel where it is.
    st.change_dir("nope").await;
    assert_eq!(st.panels[0].cwd.path, root);

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn mouse_clicks_move_cursor_and_mark_in_panel() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_mouse_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for n in ["a.txt", "b.txt", "c.txt", "d.txt"] {
        std::fs::write(root.join(n), b"x").unwrap();
    }

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 1; // start on the other panel to prove activation switches
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();

    // Render once to populate the panel hit geometry.
    let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
    term.draw(|f| crate::ui::draw(f, &mut st)).unwrap();

    let hit = st.panels[0].hit.expect("panel hit recorded");
    // Aim at the third visible row.
    let col = hit.body.x + 1;
    let row = hit.body.y + 2;
    let target = hit.index_at(col, row, st.panels[0].entries.len()).unwrap();

    // Left-click moves the cursor there and activates the left panel.
    st.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: col,
        row,
        modifiers: KeyModifiers::NONE,
    })
    .await;
    assert_eq!(st.active, 0, "left panel should become active");
    assert_eq!(st.panels[0].cursor, target, "cursor should jump to clicked row");

    // Right-click marks the entry under the pointer.
    let name = st.panels[0].entries[target].name.clone();
    st.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right),
        column: col,
        row,
        modifiers: KeyModifiers::NONE,
    })
    .await;
    assert!(st.panels[0].selection.is_marked(&name), "right-click marks the file");

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn double_click_enters_directory() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_dblclick_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("sub/inside.txt"), b"x").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();

    // Render once so the panel records its click geometry.
    let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
    term.draw(|f| crate::ui::draw(f, &mut st)).unwrap();

    // Find the screen row of the "sub" directory entry.
    let sub_idx = st.panels[0].entries.iter().position(|e| e.name == "sub").unwrap();
    let hit = st.panels[0].hit.expect("panel hit recorded");
    let col = hit.body.x + 1;
    let len = st.panels[0].entries.len();
    let row = (hit.body.y..hit.body.y + hit.body.height)
        .find(|&r| hit.index_at(col, r, len) == Some(sub_idx))
        .expect("a visible row maps to the subdirectory");
    let click = || MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: col,
        row,
        modifiers: KeyModifiers::NONE,
    };

    // First click just moves the cursor — it does not navigate.
    st.handle_mouse(click()).await;
    assert_eq!(st.panels[0].cursor, sub_idx, "single click moves the cursor");
    assert_eq!(st.panels[0].cwd.path, root, "single click does not navigate");

    // A second click on the same entry (within the window) enters the directory.
    st.handle_mouse(click()).await;
    assert_eq!(st.panels[0].cwd.path, root.join("sub"), "double click enters the directory");
    assert!(
        st.panels[0].entries.iter().any(|e| e.name == "inside.txt"),
        "now listing the subdirectory's contents"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(unix)]
#[tokio::test]
async fn chmod_recursive_applies_into_directories() {
    use std::os::unix::fs::PermissionsExt;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_chmodrec_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("top.txt"), b"a").unwrap();
    std::fs::write(root.join("sub/deep.txt"), b"b").unwrap();
    let all = [
        root.clone(),
        root.join("top.txt"),
        root.join("sub"),
        root.join("sub/deep.txt"),
    ];
    // Dirs keep the execute bit so the recursive walk can traverse them; files
    // start at 0o644. Everything differs from the 0o700 we'll apply.
    for p in &all {
        let m = if p.is_dir() { 0o755 } else { 0o644 };
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap();
    }
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();

    // Recursive: every file and directory in the tree gets the new mode.
    st.handle_submit(Submit::Chmod(vec![VfsPath::local(&root)], 0o700, true)).await;
    for p in &all {
        assert_eq!(mode(p), 0o700, "recursive chmod reached {}", p.display());
    }

    // Non-recursive: only the named target changes; descendants are untouched.
    st.handle_submit(Submit::Chmod(vec![VfsPath::local(&root)], 0o755, false)).await;
    assert_eq!(mode(&root), 0o755, "root changed");
    assert_eq!(mode(&root.join("sub/deep.txt")), 0o700, "descendant untouched");

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn multi_rename_swaps_and_renumbers_safely() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_mrename_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.txt"), b"A").unwrap();
    std::fs::write(root.join("b.txt"), b"B").unwrap();
    std::fs::write(root.join("c.txt"), b"C").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();

    let p = |n: &str| VfsPath::local(&root).join(n);
    // Swap a <-> b (the hard case: each target is another live source) and
    // rename c -> d.
    let plan = vec![
        (p("a.txt"), "b.txt".to_string()),
        (p("b.txt"), "a.txt".to_string()),
        (p("c.txt"), "d.txt".to_string()),
    ];
    st.do_multi_rename(plan).await;

    assert!(st.dialog.is_none(), "no error dialog on success");
    assert_eq!(std::fs::read(root.join("a.txt")).unwrap(), b"B", "a.txt got b's content");
    assert_eq!(std::fs::read(root.join("b.txt")).unwrap(), b"A", "b.txt got a's content");
    assert!(!root.join("c.txt").exists(), "c.txt was renamed away");
    assert_eq!(std::fs::read(root.join("d.txt")).unwrap(), b"C", "d.txt got c's content");
    let leftover: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".rc-rename-tmp"))
        .collect();
    assert!(leftover.is_empty(), "temporary names cleaned up: {leftover:?}");

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn shift_f6_opens_multi_rename_for_selection() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_mrshortcut_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.txt"), b"a").unwrap();
    std::fs::write(root.join("b.txt"), b"b").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();

    // With nothing selected, the shortcut shows an error instead of the tool.
    st.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::SHIFT)).await;
    assert!(
        matches!(st.dialog, Some(Dialog::Message(_))),
        "no selection → error message"
    );
    st.dialog = None;

    // Select a file; now Shift-F6 (and Ctrl-F6) open the multi-rename dialog.
    st.panels[0].selection.mark("a.txt");
    st.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::SHIFT)).await;
    assert!(matches!(st.dialog, Some(Dialog::MultiRename(_))), "Shift-F6 opens multi rename");
    st.dialog = None;
    st.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::CONTROL)).await;
    assert!(matches!(st.dialog, Some(Dialog::MultiRename(_))), "Ctrl-F6 opens multi rename");

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn multi_rename_refuses_to_clobber_existing_file() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_mrclobber_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.txt"), b"A").unwrap();
    std::fs::write(root.join("keep.txt"), b"KEEP").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();

    // Renaming a.txt onto the unrelated existing keep.txt must be refused.
    let plan = vec![(VfsPath::local(&root).join("a.txt"), "keep.txt".to_string())];
    st.do_multi_rename(plan).await;

    assert!(st.dialog.is_some(), "an error dialog is shown");
    assert!(root.join("a.txt").exists(), "source left in place");
    assert_eq!(std::fs::read(root.join("keep.txt")).unwrap(), b"KEEP", "existing target untouched");

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn delete_anchor_targets_next_file() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_del_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for n in ["a.txt", "b.txt", "c.txt", "d.txt"] {
        std::fs::write(root.join(n), b"x").unwrap();
    }
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    let at = |st: &AppState, name: &str| {
        st.panels[0].entries.iter().position(|e| e.name == name).unwrap()
    };

    // Cursor on c.txt; deleting it should anchor the cursor on the *next* file.
    st.panels[0].cursor = at(&st, "c.txt");
    let anchor = st.delete_anchor(&[VfsPath::local(root.join("c.txt"))]);
    assert_eq!(anchor.as_deref(), Some("d.txt"), "cursor follows down to the next file");

    // Deleting the last file falls back to the entry above it.
    st.panels[0].cursor = at(&st, "d.txt");
    let anchor = st.delete_anchor(&[VfsPath::local(root.join("d.txt"))]);
    assert_eq!(anchor.as_deref(), Some("c.txt"), "no file below ⇒ anchor above");

    // Deleting a block running to the end also falls back above the block.
    st.panels[0].cursor = at(&st, "c.txt");
    let anchor = st.delete_anchor(&[
        VfsPath::local(root.join("c.txt")),
        VfsPath::local(root.join("d.txt")),
    ]);
    assert_eq!(anchor.as_deref(), Some("b.txt"));

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn right_drag_inverts_selection_across_files() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_drag_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for n in ["a.txt", "b.txt", "c.txt", "d.txt"] {
        std::fs::write(root.join(n), b"x").unwrap();
    }

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    // Pre-select a.txt and b.txt.
    st.panels[0].selection.mark("a.txt");
    st.panels[0].selection.mark("b.txt");

    let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
    term.draw(|f| crate::ui::draw(f, &mut st)).unwrap();
    let hit = st.panels[0].hit.expect("hit");

    let col = hit.body.x + 1;
    // Press on a.txt, then drag across a (again), b, then c.
    for (kind, name) in [
        (MouseEventKind::Down(MouseButton::Right), "a.txt"),
        (MouseEventKind::Drag(MouseButton::Right), "a.txt"), // same cell: no double-flip
        (MouseEventKind::Drag(MouseButton::Right), "b.txt"),
        (MouseEventKind::Drag(MouseButton::Right), "c.txt"),
    ] {
        let idx = st.panels[0].entries.iter().position(|e| e.name == name).unwrap();
        let row = hit.body.y + (idx - hit.offset) as u16;
        st.handle_mouse(MouseEvent { kind, column: col, row, modifiers: KeyModifiers::NONE })
            .await;
    }

    let sel = &st.panels[0].selection;
    assert!(!sel.is_marked("a.txt"), "a was selected → inverted off");
    assert!(!sel.is_marked("b.txt"), "b was selected → inverted off");
    assert!(sel.is_marked("c.txt"), "c was unselected → inverted on");
    assert!(!sel.is_marked("d.txt"), "d untouched");

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn page_keys_move_by_visible_page() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_page_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for i in 0..100 {
        std::fs::write(root.join(format!("f{i:03}.txt")), b"x").unwrap();
    }
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();

    // Render so the panel records its visible page size from the area.
    let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
    term.draw(|f| crate::ui::draw(f, &mut st)).unwrap();
    let page = st.panels[0].page;
    assert!(page > 1, "page size should reflect the terminal height");

    st.panels[0].cursor = 0;
    st.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)).await;
    assert_eq!(st.panels[0].cursor, page, "PageDown moves one whole page");
    st.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE)).await;
    assert_eq!(st.panels[0].cursor, 0, "PageUp moves back a whole page");

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn mouse_click_on_menu_bar_opens_menu() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.last_area = Rect::new(0, 0, 120, 30);
    assert!(st.menu.is_none());
    // The "File" title sits a few columns in on the top row.
    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 8,
        row: 0,
        modifiers: KeyModifiers::NONE,
    };
    st.handle_mouse(click).await;
    assert!(st.menu.is_some(), "clicking the menu bar should open a menu");
}

#[test]
fn find_files_by_name_and_content() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_find_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("a.txt"), b"hello there").unwrap();
    std::fs::write(root.join("sub/b.txt"), b"world").unwrap();
    std::fs::write(root.join("c.log"), b"hello again").unwrap();

    let run = |p: &FindParams| {
        let m = crate::panel::selection::NameMatcher::build(&p.file_name, p.case_sensitive, p.shell)
            .unwrap();
        let c = crate::ops::CancelToken::new();
        find_files(&root, p, &m, &c, |_, _| {})
    };

    let by_name = FindParams {
        start_at: String::new(),
        file_name: "*.txt".into(),
        content: String::new(),
        recursive: true,
        case_sensitive: false,
        skip_hidden: true,
        shell: true,
    };
    assert_eq!(run(&by_name).len(), 2, "two .txt files");

    let by_content = FindParams {
        file_name: "*".into(),
        content: "HELLO".into(),
        ..by_name
    };
    assert_eq!(run(&by_content).len(), 2, "two files contain 'hello'");

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn find_files_vfs_matches_names_recursively() {
    use std::sync::Arc;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_vfind_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("a.txt"), b"x").unwrap();
    std::fs::write(root.join("sub/b.txt"), b"yy").unwrap();
    std::fs::write(root.join("sub/c.log"), b"z").unwrap();

    // Exercise the VFS walker through the local backend (stands in for remote).
    let backend: Arc<dyn Vfs> = Arc::new(crate::vfs::local::LocalFs::new());
    let matcher = crate::panel::selection::NameMatcher::build("*.txt", false, true).unwrap();
    let cancel = crate::ops::CancelToken::new();
    let results =
        find_files_vfs(&backend, VfsPath::local(&root), &matcher, true, true, &cancel, |_, _| {})
            .await;

    let mut names: Vec<String> = results.iter().map(|(p, _)| p.file_name()).collect();
    names.sort();
    assert_eq!(names, vec!["a.txt", "b.txt"], "name-only, recursive, .log excluded");
    // Sizes come from the directory listing, not a second stat.
    assert!(results.iter().any(|(p, s)| p.file_name() == "b.txt" && *s == 2));

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn theme_preview_applies_and_reverts_on_cancel() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    let original = st.theme.name.clone();
    st.open_settings();
    // The Theme choice is the first (focused) field; Space cycles it.
    st.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)).await;
    assert_ne!(st.theme.name, original, "theme preview should apply live");
    // Esc cancels → revert to the original theme.
    st.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).await;
    assert_eq!(st.theme.name, original, "cancel should revert the preview");
}

#[tokio::test]
async fn f1_opens_help_in_viewer() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    assert!(st.viewer.is_none());
    st.open_help();
    let v = st.viewer.as_ref().expect("F1 should open the help viewer");
    assert!(
        v.markdown_active(),
        "help should open in rendered Markdown mode (tags hidden), not raw"
    );
}

#[tokio::test]
async fn disk_mounter_opens_and_prompts_for_path() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.init().await;
    // The Command-menu action opens the mounter view.
    st.run_menu_action(crate::ui::menu::MenuAction::DiskManager).await;
    assert!(st.mountview.is_some(), "disk mounter should open");

    // Enter on a device requests a mount → the app raises a path-input dialog.
    let mv = st.mountview.as_mut().unwrap();
    mv.devices = vec![crate::mount::BlockDevice {
        name: "sdb1".into(),
        dev: "/dev/sdb1".into(),
        size: 0,
        fstype: String::new(),
        mountpoint: None,
        ..Default::default()
    }];
    mv.dev_cursor = 0;
    // Enter opens the device action menu (Mount/Format/Cancel).
    st.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
    assert!(
        matches!(st.dialog, Some(crate::ui::dialog::Dialog::Confirm(_))),
        "Enter on a device opens its action menu"
    );
    // Activating the focused "Mount" button prompts for the target path.
    st.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
    assert!(
        matches!(st.dialog, Some(crate::ui::dialog::Dialog::Input(_))),
        "the Mount action prompts for the target path"
    );

    // Esc cancels the (open) dialog immediately, returning to the mounter.
    st.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).await;
    assert!(st.dialog.is_none());
    assert!(st.mountview.is_some(), "still on the mounter after cancel");
    // With no dialog, a lone Esc is held (function-key prefix); the next key
    // flushes it through to the mounter, which closes.
    st.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).await;
    st.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).await;
    assert!(st.mountview.is_none(), "Esc closes the mounter");
}

#[tokio::test]
async fn flash_selection_warns_for_non_removable_then_confirms() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    let spec = |removable: bool| crate::flash::FlashSpec {
        image_path: "/x.iso".into(),
        image_name: "x.iso".into(),
        image_size: 10,
        target: crate::flash::FlashTarget {
            dev: "/dev/sdb".into(),
            size: 1000,
            removable,
            ..Default::default()
        },
    };
    // A removable target goes straight to the destructive confirm.
    st.handle_submit(Submit::FlashSelected(spec(true))).await;
    assert!(matches!(st.dialog, Some(Dialog::Confirm(_))));
    // A fixed disk first raises the red danger warning.
    st.handle_submit(Submit::FlashSelected(spec(false))).await;
    assert!(matches!(st.dialog, Some(Dialog::Confirm(_))));
}

#[tokio::test]
async fn flash_abort_prompts_then_resumes_or_aborts() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    let id = 42;
    let tok = crate::ops::CancelToken::new();
    st.flash_tasks.insert(id, tok.clone());
    st.dialog = Some(Dialog::Progress(ProgressDialog::new(id, "Flashing")));

    // Abort stashes the progress and raises the resume/abort prompt.
    st.handle_dialog_result(DialogResult::Abort(id)).await;
    assert!(matches!(st.dialog, Some(Dialog::Confirm(_))), "abort confirm shown");
    assert!(st.stashed_progress.is_some());
    assert!(!tok.is_cancelled(), "flashing keeps running until really aborted");

    // Resume restores the progress dialog.
    st.handle_submit(Submit::FlashResume).await;
    assert!(matches!(st.dialog, Some(Dialog::Progress(_))));

    // Really aborting trips the cancel token.
    st.handle_dialog_result(DialogResult::Abort(id)).await;
    st.handle_submit(Submit::FlashAbort(id)).await;
    assert!(tok.is_cancelled(), "abort cancels the flash task");
    assert!(st.stashed_progress.is_none());
}

#[tokio::test]
async fn create_image_browse_overwrite_and_done() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    let target = crate::flash::FlashTarget { dev: "/dev/sdb".into(), size: 1000, ..Default::default() };

    // "Create image" opens the save browser.
    st.handle_submit(Submit::ImageBrowse(target.clone())).await;
    assert!(matches!(st.dialog, Some(Dialog::ImageSave(_))), "image save browser opens");

    // Saving onto an existing file raises an overwrite confirmation.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dest = std::env::temp_dir().join(format!("rc_imgov_{}_{nanos}.img", std::process::id()));
    std::fs::write(&dest, b"old").unwrap();
    let spec = crate::flash::ImageSpec {
        source: target,
        dest_path: dest.clone(),
        dest_name: "x.img".into(),
    };
    st.handle_submit(Submit::ImageSave(spec)).await;
    assert!(matches!(st.dialog, Some(Dialog::Confirm(_))), "overwrite confirm shown");
    std::fs::remove_file(&dest).ok();

    // ImageDone clears the task and reports success.
    let id = 99;
    st.flash_tasks.insert(id, crate::ops::CancelToken::new());
    st.apply_event(AppEvent::ImageDone { id, outcome: TaskOutcome::Done }).await;
    assert!(!st.flash_tasks.contains_key(&id));
    assert!(matches!(st.dialog, Some(Dialog::Message(_))));
}

#[tokio::test]
async fn formatting_shows_busy_dialog_until_done() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.mountview = Some(MountView::new());

    // A privileged op in flight raises the non-dismissible busy spinner, and
    // input can't close it.
    st.dialog = Some(Dialog::Busy(BusyDialog::new("Please wait", "Formatting /dev/sdb1...")));
    st.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
    st.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).await;
    assert!(matches!(st.dialog, Some(Dialog::Busy(_))), "busy spinner ignores input");

    // Completion dismisses the spinner and reports success on the status line.
    st.apply_event(AppEvent::PrivilegedDone {
        ok_msg: "Formatted /dev/sdb1 as EXT4".into(),
        result: Ok(()),
    })
    .await;
    assert!(st.dialog.is_none(), "busy dialog dismissed on completion");
    assert_eq!(st.mountview.as_ref().unwrap().status, "Formatted /dev/sdb1 as EXT4");

    // A failure surfaces as an error on the status line.
    st.dialog = Some(Dialog::Busy(BusyDialog::new("Please wait", "Formatting...")));
    st.apply_event(AppEvent::PrivilegedDone {
        ok_msg: "ok".into(),
        result: Err("mkfs failed".into()),
    })
    .await;
    assert!(st.dialog.is_none());
    assert!(st.mountview.as_ref().unwrap().status.contains("mkfs failed"));
}

#[test]
fn menu_title_index_maps_first_letters() {
    assert_eq!(menu_title_index('l'), Some(0));
    assert_eq!(menu_title_index('F'), Some(1));
    assert_eq!(menu_title_index('c'), Some(2));
    assert_eq!(menu_title_index('O'), Some(3));
    assert_eq!(menu_title_index('r'), Some(4));
    assert_eq!(menu_title_index('x'), None);
}

#[tokio::test]
async fn alt_arms_hint_and_alt_letter_opens_menu() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.init().await;
    assert!(!st.alt_hint && st.menu.is_none());

    // Alt+F opens the File menu directly (no hint left dangling).
    st.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT)).await;
    assert!(st.menu.is_some(), "Alt+F opens a menu");
    assert!(!st.alt_hint);
    // Once open, a plain letter drives it (Alt no longer needed): 'q' = Quit.
    // Close it back first to keep the test focused on arming below.
    st.menu = None;

    // A non-menu Alt key just arms the accelerator hint (menu stays closed).
    st.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::ALT)).await;
    assert!(st.alt_hint, "Alt arms the accelerator hint");
    assert!(st.menu.is_none());
    // The next ordinary key clears the hint.
    st.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)).await;
    assert!(!st.alt_hint, "a non-Alt key disarms the hint");
}

#[tokio::test]
async fn alt_fkeys_open_the_drive_connection_picker() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.init().await;

    // Alt-F1 / Alt-F2 open the picker for the left / right panel.
    st.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::ALT)).await;
    assert!(matches!(st.dialog, Some(Dialog::Drive(_))), "Alt-F1 opens the picker");
    st.dialog = None;
    st.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::ALT)).await;
    assert!(matches!(st.dialog, Some(Dialog::Drive(_))), "Alt-F2 opens the picker");

    // Choosing a connection opens the connect form for that side/protocol.
    st.handle_submit(Submit::OpenConnect(1, crate::vfs::remote::Protocol::Sftp)).await;
    assert!(matches!(st.dialog, Some(Dialog::Form(_))), "SFTP button opens the connect form");
}

#[tokio::test]
async fn fkey_bar_click_runs_panel_functions() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.init().await;
    let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
    term.draw(|f| crate::ui::draw(f, &mut st)).unwrap();

    // The bar is the bottom row (29); 10 labels over 120 cols → 12 each.
    // F9 ("PullDn", index 8) spans cols 96-107 → opens the pulldown menu.
    assert!(st.menu.is_none());
    let click = |c| MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: c,
        row: 29,
        modifiers: KeyModifiers::NONE,
    };
    st.handle_mouse(click(100)).await;
    assert!(st.menu.is_some(), "clicking the F9 segment opens the menu");
    st.menu = None;

    // F10 ("Quit", index 9) spans cols 108-119 → quits (confirmation off).
    st.config.confirm_exit = false;
    let flow = st.handle_mouse(click(112)).await;
    assert!(matches!(flow, Flow::Quit), "clicking the F10 segment quits");
}

#[tokio::test]
async fn confirm_exit_gates_the_quit_prompt() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    // With confirmation off, F10 quits immediately.
    st.config.confirm_exit = false;
    let flow = st.handle_key(KeyEvent::new(KeyCode::F(10), KeyModifiers::NONE)).await;
    assert!(matches!(flow, Flow::Quit));
    assert!(st.dialog.is_none(), "no prompt when confirmation is off");
    // With confirmation on, F10 raises the quit dialog instead.
    st.config.confirm_exit = true;
    let flow = st.handle_key(KeyEvent::new(KeyCode::F(10), KeyModifiers::NONE)).await;
    assert!(matches!(flow, Flow::Continue));
    assert!(st.dialog.is_some(), "prompt shown when confirmation is on");
}

#[test]
fn esc_prefix_maps_digits_to_function_keys() {
    assert_eq!(fkey_for_code(KeyCode::Char('1')), Some(1));
    assert_eq!(fkey_for_code(KeyCode::Char('9')), Some(9));
    assert_eq!(fkey_for_code(KeyCode::Char('0')), Some(10));
    assert_eq!(fkey_for_code(KeyCode::Char('a')), None);
    assert_eq!(fkey_for_code(KeyCode::Esc), None);
}

#[tokio::test]
async fn esc_then_digit_acts_as_function_key() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    assert!(st.viewer.is_none());
    // A lone Esc (no dialog/menu) is held, not acted on immediately.
    st.handle_key(esc_key()).await;
    assert!(st.pending_esc.is_some(), "lone Esc should be held");
    // The following '1' completes Esc-1 => F1 => help viewer.
    st.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE))
        .await;
    assert!(st.viewer.is_some(), "Esc-1 should act as F1 (help)");
    assert!(st.pending_esc.is_none(), "the sequence is resolved");
}

#[tokio::test]
async fn alt_digit_acts_as_function_key() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    assert!(st.viewer.is_none());
    // Terminals deliver a fast Esc+digit as Alt+digit; that is an F-key too.
    st.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT))
        .await;
    assert!(st.viewer.is_some(), "Alt-1 should act as F1 (help)");
    assert!(st.pending_esc.is_none());
}

#[tokio::test]
async fn esc_then_nondigit_delivers_plain_esc() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    for c in "abc".chars() {
        st.cmd.insert(c);
    }
    st.handle_key(esc_key()).await;
    assert!(st.pending_esc.is_some());
    // A non-digit resolves the held Esc as a plain Esc (clears the cmd line)
    // and then delivers the key itself.
    st.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
        .await;
    assert!(st.pending_esc.is_none());
    assert_eq!(st.cmd.buffer, "x", "Esc cleared the line, then 'x' was typed");
}
