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

/// Full dispatch: pressing Enter on a file matched by an rc.ext `Open=%cd
/// …/uzip://` rule mounts it through the real MC `uzip` extfs script. Uses a
/// `.pk3` (a zip the native archive backend does not recognise) so the path
/// goes through rc.ext, not `ArchiveFs`. Skipped when uzip/zip are absent.
#[cfg(unix)]
#[tokio::test]
async fn enter_dir_mounts_extfs_via_rc_ext() {
    let have_script = crate::vfs::extfs::find_extfs_script("uzip").is_some();
    let have_zip = std::process::Command::new("zip")
        .arg("-v")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !have_script || !have_zip {
        eprintln!("skipping enter_dir_mounts_extfs test: uzip script or zip not available");
        return;
    }

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_ext_nav_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("sub/file.txt"), b"hi").unwrap();
    std::fs::write(root.join("top.txt"), b"top").unwrap();
    assert!(std::process::Command::new("zip")
        .current_dir(&root)
        .arg("-r")
        .arg("bundle.pk3")
        .arg("sub")
        .arg("top.txt")
        .output()
        .unwrap()
        .status
        .success());

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    // Map .pk3 → the uzip extfs script (native ArchiveFs ignores .pk3).
    st.ext_rules = crate::ext::parse("shell/.pk3\n    Open=%cd %p/uzip://\n");
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();

    let idx = st.panels[0]
        .entries
        .iter()
        .position(|e| e.name == "bundle.pk3")
        .unwrap();
    st.panels[0].cursor = idx;
    st.active = 0;
    st.enter_dir().await;

    assert_eq!(st.panels[0].cwd.scheme, "uzip", "panel should be in the uzip mount");
    assert!(st.panels[0].cwd.is_archive(), "extfs path is container-backed");
    assert!(!st.panels[0].cwd.is_remote(), "extfs counts as local");
    let names: Vec<String> = st.panels[0].entries.iter().map(|e| e.name.clone()).collect();
    assert!(names.contains(&"sub".to_string()), "names: {names:?}");
    assert!(names.contains(&"top.txt".to_string()), "names: {names:?}");
    assert!(names.contains(&"..".to_string()), "mount has a parent link");

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

// A minimal in-memory VFS used to stand in for a remote backend: it lists an
// empty directory (so navigation/reload succeeds) and refuses everything else.
struct StubVfs;

#[async_trait::async_trait]
impl crate::vfs::Vfs for StubVfs {
    fn scheme(&self) -> &str {
        "sftp"
    }
    fn capabilities(&self) -> crate::vfs::Capabilities {
        crate::vfs::Capabilities::local()
    }
    async fn read_dir(&self, _dir: &VfsPath) -> crate::util::Result<Vec<VfsEntry>> {
        Ok(vec![])
    }
    async fn stat(&self, path: &VfsPath) -> crate::util::Result<VfsEntry> {
        Ok(VfsEntry {
            name: path.file_name(),
            kind: VfsKind::Dir,
            size: 0,
            mtime: None,
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
    async fn open_read(&self, _p: &VfsPath) -> crate::util::Result<crate::vfs::BoxRead> {
        Err(crate::util::Error::Unsupported)
    }
    async fn open_write(
        &self,
        _p: &VfsPath,
        _m: crate::vfs::WriteMeta,
    ) -> crate::util::Result<crate::vfs::BoxWrite> {
        Err(crate::util::Error::Unsupported)
    }
    async fn mkdir(&self, _p: &VfsPath) -> crate::util::Result<()> {
        Err(crate::util::Error::Unsupported)
    }
    async fn remove_file(&self, _p: &VfsPath) -> crate::util::Result<()> {
        Err(crate::util::Error::Unsupported)
    }
    async fn remove_dir(&self, _p: &VfsPath) -> crate::util::Result<()> {
        Err(crate::util::Error::Unsupported)
    }
    async fn rename(&self, _f: &VfsPath, _t: &VfsPath) -> crate::util::Result<()> {
        Err(crate::util::Error::Unsupported)
    }
}

/// Register a stub remote backend under `scheme` and record a session for it,
/// placing panel `side` on `path` within that session. Returns the session id.
fn setup_remote_panel(st: &mut AppState, side: usize, scheme: &str, path: &str) -> usize {
    let id = st.next_session_id;
    st.next_session_id += 1;
    st.registry.register(scheme.to_string(), std::sync::Arc::new(StubVfs));
    let root = VfsPath { scheme: scheme.to_string(), path: "/".into(), container: None };
    st.sessions.push(RemoteSession {
        id,
        scheme: scheme.to_string(),
        label: format!("sftp://u@{scheme}"),
        cwd: root,
        creds: creds(),
    });
    let cwd = VfsPath { scheme: scheme.to_string(), path: path.into(), container: None };
    let backend = st.registry.resolve(&cwd).unwrap();
    st.panels[side].cwd = cwd;
    st.panels[side].backend = backend;
    id
}

fn creds() -> RemoteCreds {
    RemoteCreds {
        protocol: crate::vfs::remote::Protocol::Sftp,
        host: "example.invalid".into(),
        port: 22,
        user: "u".into(),
        password: "p".into(),
        path: String::new(),
        passive: true,
    }
}

#[tokio::test]
async fn session_persists_and_switch_restores_last_dir() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    let local = VfsPath::local_cwd();
    st.last_local_cwd[0] = local.clone();
    let id = setup_remote_panel(&mut st, 0, "sftp-0", "/home/user/work");

    // Return to Local: the session must survive and remember /home/user/work.
    st.go_local(0).await;
    assert!(!st.panels[0].cwd.is_remote(), "panel is local again");
    assert_eq!(st.panels[0].cwd, local, "restored the last local dir");
    assert_eq!(st.sessions.len(), 1, "session stays open after go_local");
    assert_eq!(st.sessions[0].cwd.path, std::path::PathBuf::from("/home/user/work"));

    // Switch back: land on the remembered directory, not the session root.
    st.switch_to_session(0, id).await;
    assert_eq!(st.panels[0].cwd.scheme, "sftp-0");
    assert_eq!(st.panels[0].cwd.path, std::path::PathBuf::from("/home/user/work"));
}

#[tokio::test]
async fn one_remote_guard_blocks_second_remote_panel() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    let id = setup_remote_panel(&mut st, 0, "sftp-0", "/srv");

    // Panel 1 is local; trying to connect it while panel 0 is remote is refused
    // (guard runs before any network I/O).
    st.connect_remote(1, creds()).await;
    assert!(!st.panels[1].cwd.is_remote(), "panel 1 stays local");
    assert!(matches!(st.dialog, Some(Dialog::Message(_))), "an error was shown");

    // Switching panel 1 to the existing session is likewise refused.
    st.dialog = None;
    st.switch_to_session(1, id).await;
    assert!(!st.panels[1].cwd.is_remote(), "panel 1 still local");
    assert!(matches!(st.dialog, Some(Dialog::Message(_))));
}

#[tokio::test]
async fn disconnect_session_tears_down_and_frees_panel() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    let remote_path =
        VfsPath { scheme: "sftp-0".into(), path: "/srv".into(), container: None };
    let id = setup_remote_panel(&mut st, 0, "sftp-0", "/srv");

    st.disconnect_session(id).await;
    assert!(st.sessions.is_empty(), "session record dropped");
    assert!(st.registry.resolve(&remote_path).is_err(), "backend unregistered");
    assert!(!st.panels[0].cwd.is_remote(), "the panel on it went local");
}

#[tokio::test]
async fn go_local_restores_remembered_dir_not_process_cwd() {
    // Create a real, readable directory distinct from the process cwd.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rc_local_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.last_local_cwd[0] = VfsPath::local(&dir);
    setup_remote_panel(&mut st, 0, "sftp-0", "/srv");

    st.go_local(0).await;
    assert_eq!(st.panels[0].cwd, VfsPath::local(&dir), "landed on the remembered dir");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn other_panel_is_remote_treats_archive_as_local() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.panels[1].cwd = VfsPath::archive("/tmp/some.zip", "/");
    assert!(!st.panels[1].cwd.is_remote(), "archive counts as local");
    assert!(!st.other_panel_is_remote(0), "archive on the other panel is not remote");
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

    // A bare name resolves against the *source* (active) panel — mc-style, so a
    // typed new name renames in place instead of moving to the opposite panel.
    let local_dest = VfsPath::local("/a/b");
    let d = dest_vfspath("sub", &local_dest, &remote);
    assert_eq!((d.scheme.as_str(), d.path), ("scp-0", PathBuf::from("/home/user/sub")));
    // An absolute path still lands on the destination (other) panel's backend.
    let d = dest_vfspath("/a/b/sub", &local_dest, &remote);
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

/// Run a spawned copy/move/delete task to completion, applying its events.
async fn drain_taskdone(st: &mut AppState, rx: &mut crate::util::async_bridge::AppReceiver) {
    loop {
        let ev = rx.recv().await.unwrap();
        let done = matches!(ev, AppEvent::TaskDone { .. });
        st.apply_event(ev).await;
        if done {
            break;
        }
    }
}

/// F6 on several *selected* files moves them into the other panel: the copies
/// appear there and the originals are gone from the source.
#[tokio::test]
async fn f6_moves_selected_files_and_removes_originals() {
    use crate::ui::dialog::Submit;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_moveN_{}_{nanos}", std::process::id()));
    let left = root.join("left");
    let right = root.join("right");
    std::fs::create_dir_all(&left).unwrap();
    std::fs::create_dir_all(&right).unwrap();
    for n in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(left.join(n), b"data").unwrap();
    }

    let (tx, mut rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&left);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    st.panels[1].cwd = VfsPath::local(&right);
    st.panels[1].backend = st.registry.local();
    st.panels[1].reload().await.unwrap();

    // Select a.txt and b.txt (leave c.txt behind), then Move to the right panel.
    st.panels[0].selection.mark("a.txt");
    st.panels[0].selection.mark("b.txt");
    let sources = st.panels[0].operation_targets();
    assert_eq!(sources.len(), 2, "two files selected");
    let dest = right.to_string_lossy().into_owned();
    st.handle_submit(Submit::Move(sources, dest)).await;
    drain_taskdone(&mut st, &mut rx).await;

    // Moved: present on the right, gone from the left.
    assert!(right.join("a.txt").is_file() && right.join("b.txt").is_file(), "copies land in dest");
    assert!(!left.join("a.txt").exists(), "a.txt removed from source (moved, not copied)");
    assert!(!left.join("b.txt").exists(), "b.txt removed from source (moved, not copied)");
    assert!(left.join("c.txt").is_file(), "unselected file stays put");
    // The source panel's listing is refreshed so the moved files no longer show.
    let left_names: Vec<&str> = st.panels[0].entries.iter().map(|e| e.name.as_str()).collect();
    assert!(!left_names.contains(&"a.txt"), "source panel refreshed (a.txt gone from listing)");
    assert!(!left_names.contains(&"b.txt"), "source panel refreshed (b.txt gone from listing)");

    std::fs::remove_dir_all(&root).ok();
}

/// Moving onto an existing destination file (which skips the intra-backend
/// rename fast path and uses copy-then-delete) still removes the source.
#[tokio::test]
async fn move_over_existing_file_still_removes_source() {
    use crate::ui::dialog::Submit;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_moveover_{}_{nanos}", std::process::id()));
    let left = root.join("left");
    let right = root.join("right");
    std::fs::create_dir_all(&left).unwrap();
    std::fs::create_dir_all(&right).unwrap();
    std::fs::write(left.join("a.txt"), b"new").unwrap();
    std::fs::write(right.join("a.txt"), b"old").unwrap(); // conflict at the destination

    let (tx, mut rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.config.confirm_overwrite = false; // overwrite silently (no conflict prompt)
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&left);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    st.panels[1].cwd = VfsPath::local(&right);
    st.panels[1].backend = st.registry.local();
    st.panels[1].reload().await.unwrap();

    let src = VfsPath::local(left.join("a.txt"));
    st.handle_submit(Submit::Move(vec![src], right.to_string_lossy().into_owned())).await;
    drain_taskdone(&mut st, &mut rx).await;

    assert_eq!(std::fs::read(right.join("a.txt")).unwrap(), b"new", "destination overwritten");
    assert!(!left.join("a.txt").exists(), "source removed even when the destination existed");

    std::fs::remove_dir_all(&root).ok();
}

/// A Move where the user answers "Skip" at an overwrite conflict must NOT delete
/// the source — previously the skipped file was removed without being copied
/// (silent data loss).
#[tokio::test]
async fn move_skipping_overwrite_conflict_keeps_source() {
    use crate::ops::progress::OverwriteDecision;
    use crate::ui::dialog::Submit;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_moveskip_{}_{nanos}", std::process::id()));
    let left = root.join("left");
    let right = root.join("right");
    std::fs::create_dir_all(&left).unwrap();
    std::fs::create_dir_all(&right).unwrap();
    std::fs::write(left.join("a.txt"), b"new").unwrap();
    std::fs::write(right.join("a.txt"), b"old").unwrap(); // conflict at the destination

    let (tx, mut rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.config.confirm_overwrite = true; // force the conflict prompt (regardless of config)
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&left);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    st.panels[1].cwd = VfsPath::local(&right);
    st.panels[1].backend = st.registry.local();
    st.panels[1].reload().await.unwrap();

    let src = VfsPath::local(left.join("a.txt"));
    st.handle_submit(Submit::Move(vec![src], right.to_string_lossy().into_owned())).await;
    // Drain events; answer the overwrite conflict with "Skip", finish on TaskDone.
    loop {
        let ev = rx.recv().await.unwrap();
        match ev {
            AppEvent::Conflict(info) => {
                let h = st.tasks.get(&info.id).expect("running move task");
                let _ = h.reply.try_send(OverwriteDecision::SkipOnce);
            }
            AppEvent::TaskDone { id, outcome } => {
                st.apply_event(AppEvent::TaskDone { id, outcome }).await;
                break;
            }
            other => st.apply_event(other).await,
        }
    }

    assert!(left.join("a.txt").exists(), "skipped file's source is kept (not deleted)");
    assert_eq!(std::fs::read(right.join("a.txt")).unwrap(), b"old", "destination left untouched");

    std::fs::remove_dir_all(&root).ok();
}

/// The full keyboard flow — F6 to open the Move dialog, Enter to accept the
/// prefilled other-panel destination — moves the selected files (not copies).
#[tokio::test]
async fn f6_key_flow_moves_selected_files() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_f6key_{}_{nanos}", std::process::id()));
    let left = root.join("left");
    let right = root.join("right");
    std::fs::create_dir_all(&left).unwrap();
    std::fs::create_dir_all(&right).unwrap();
    for n in ["a.txt", "b.txt"] {
        std::fs::write(left.join(n), b"data").unwrap();
    }

    let (tx, mut rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].format = ViewFormat::Full;
    st.panels[0].cwd = VfsPath::local(&left);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    st.panels[1].cwd = VfsPath::local(&right);
    st.panels[1].backend = st.registry.local();
    st.panels[1].reload().await.unwrap();
    st.panels[0].selection.mark("a.txt");
    st.panels[0].selection.mark("b.txt");

    let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
    st.handle_key(key(KeyCode::F(6))).await; // open the Move dialog
    assert!(matches!(st.dialog, Some(Dialog::Input(_))), "F6 opens the transfer dialog");
    st.handle_key(key(KeyCode::Enter)).await; // accept the prefilled destination
    drain_taskdone(&mut st, &mut rx).await;

    assert!(right.join("a.txt").is_file() && right.join("b.txt").is_file(), "files copied to dest");
    assert!(!left.join("a.txt").exists() && !left.join("b.txt").exists(), "originals removed (moved)");

    std::fs::remove_dir_all(&root).ok();
}

/// F6 on a single directory with a bare new name renames it *in place* (in the
/// source panel), not into the opposite panel, and lands the cursor on it.
#[tokio::test]
async fn f6_bare_name_renames_in_place_and_focuses() {
    use crate::ui::dialog::Submit;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_rename_{}_{nanos}", std::process::id()));
    let left = root.join("left");
    let right = root.join("right");
    std::fs::create_dir_all(left.join("a")).unwrap();
    std::fs::create_dir_all(&right).unwrap();
    std::fs::write(left.join("a/file.txt"), b"hi").unwrap();

    let (tx, mut rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&left);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    st.panels[1].cwd = VfsPath::local(&right);
    st.panels[1].backend = st.registry.local();
    st.panels[1].reload().await.unwrap();

    let src = VfsPath::local(left.join("a"));
    st.handle_submit(Submit::Move(vec![src], "b".into())).await;
    drain_taskdone(&mut st, &mut rx).await;

    // "a" was renamed to "b" in place — not nested, and the other panel is untouched.
    assert!(left.join("b").is_dir(), "renamed dir should exist");
    assert!(left.join("b/file.txt").is_file(), "contents move with it");
    assert!(!left.join("a").exists(), "old name is gone");
    assert!(!left.join("b/a").exists(), "must not create b/a (the old bug)");
    assert!(!right.join("b").exists(), "opposite panel is not involved");

    // The cursor lands on the freshly renamed entry.
    let p = &st.panels[0];
    assert_eq!(p.entries[p.cursor].name, "b");

    std::fs::remove_dir_all(&root).ok();
}

/// Renaming when both panels show the same directory still lands the *active*
/// panel's cursor on the new name (not the other panel showing the same dir).
#[tokio::test]
async fn rename_focuses_active_panel_when_both_show_same_dir() {
    use crate::ui::dialog::Submit;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_rn_same_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(root.join("a")).unwrap();

    let (tx, mut rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    for i in 0..2 {
        st.panels[i].cwd = VfsPath::local(&root);
        st.panels[i].backend = st.registry.local();
        st.panels[i].reload().await.unwrap();
    }
    st.active = 1; // the non-first panel is active

    st.handle_submit(Submit::Move(vec![VfsPath::local(root.join("a"))], "b".into())).await;
    drain_taskdone(&mut st, &mut rx).await;

    assert!(root.join("b").is_dir());
    let p = &st.panels[1];
    assert_eq!(p.entries[p.cursor].name, "b", "active panel cursor should be on the renamed entry");

    std::fs::remove_dir_all(&root).ok();
}

/// In Brief (multi-column, column-major) view the arrow keys navigate the grid
/// like Midnight Commander: Down/Up walk down/up a column and roll over to the
/// next/previous column at a column edge; Left/Right move sideways by a whole
/// column, clamping the first column's Left to the top-left and the last
/// column's Right to the very bottom.
#[tokio::test]
async fn brief_view_column_major_arrow_navigation() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_brief_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    // 8 files + ".." = 9 entries → columns of height 3: col0=0,1,2  col1=3,4,5
    // col2=6,7,8.
    for i in 0..8 {
        std::fs::write(root.join(format!("f{i}.txt")), b"x").unwrap();
    }

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    assert_eq!(st.panels[0].entries.len(), 9, "8 files plus the parent entry");
    // Brief grid geometry as the renderer would record it: 2 visible columns,
    // each 3 entries tall.
    st.panels[0].format = ViewFormat::Brief;
    st.panels[0].cols = 2;
    st.panels[0].brief_rows = 3;

    let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
    macro_rules! at {
        ($start:expr, $code:expr) => {{
            st.panels[0].cursor = $start;
            st.handle_key(key($code)).await;
            st.panels[0].cursor
        }};
    }
    // Down at a column bottom rolls to the next column's top.
    assert_eq!(at!(2, KeyCode::Down), 3, "Down wraps col bottom → next col top");
    // Up at a column top rolls to the previous column's bottom.
    assert_eq!(at!(3, KeyCode::Up), 2, "Up wraps col top → prev col bottom");
    // Right/Left move sideways by a whole column (same row).
    assert_eq!(at!(4, KeyCode::Right), 7, "Right → same row, next column");
    assert_eq!(at!(7, KeyCode::Left), 4, "Left → same row, previous column");
    // Left inside the first column lands on the top-left.
    assert_eq!(at!(1, KeyCode::Left), 0, "Left in first column → top-left");
    // Right from the last column lands on the very bottom (clamped).
    assert_eq!(at!(7, KeyCode::Right), 8, "Right in last column → bottom");

    std::fs::remove_dir_all(&root).ok();
}

/// Editor search/replace terms are remembered on `AppState`, so they survive
/// across editor sessions (and different files) to prefill the next dialog.
#[tokio::test]
async fn editor_search_terms_persist_app_wide() {
    use crate::ui::dialog::{SearchReplaceParams, Submit};
    let params = |search: &str, replacement: &str, hex: bool, replace: bool| SearchReplaceParams {
        replace,
        search: search.into(),
        replacement: replacement.into(),
        regex: false,
        case_sensitive: false,
        whole_words: false,
        backwards: false,
        hex,
    };

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);

    // A text replace records both terms (no live editor required for the memory).
    st.handle_submit(Submit::SearchReplace(params("foo", "bar", false, true))).await;
    assert_eq!(st.search_memory.search, "foo");
    assert_eq!(st.search_memory.replacement, "bar");

    // A later hex search fills its own slot without disturbing the text terms.
    st.handle_submit(Submit::SearchReplace(params("48 65", "", true, false))).await;
    assert_eq!(st.search_memory.hex_search, "48 65");
    assert_eq!(st.search_memory.search, "foo", "text search preserved");
    assert_eq!(st.search_memory.replacement, "bar", "replacement preserved");
}

/// Creating a directory refreshes the *other* panel too when it shows the same
/// location, so the new entry appears there without a manual reload.
#[tokio::test]
async fn mkdir_mirrors_into_other_panel_showing_same_dir() {
    use crate::ui::dialog::Submit;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_mkdir_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    for i in 0..2 {
        st.panels[i].cwd = VfsPath::local(&root);
        st.panels[i].backend = st.registry.local();
        st.panels[i].reload().await.unwrap();
    }
    st.active = 0;

    st.handle_submit(Submit::MkDir("fresh".into())).await;

    let has_fresh = |p: &Panel| p.entries.iter().any(|e| e.name == "fresh");
    assert!(has_fresh(&st.panels[0]), "active panel shows the new dir");
    assert!(has_fresh(&st.panels[1]), "other panel on the same dir is refreshed too");

    std::fs::remove_dir_all(&root).ok();
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

/// Ctrl-W cycles into Tree view (building the tree), and Enter on a tree node
/// points the *inactive* panel at that directory while opening the branch.
#[tokio::test]
async fn tree_view_enter_navigates_inactive_panel() {
    use crate::panel::ViewFormat;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_tree_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(root.join("alpha/inner")).unwrap();
    std::fs::create_dir_all(root.join("beta")).unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].format = ViewFormat::Full; // start from a known view (not an ambient config)
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    // The inactive panel starts somewhere else (the process cwd).
    let start_right = st.panels[1].cwd.clone();

    // Ctrl-W: Full → Brief → Details → Tree.
    let ctrl_w = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL);
    for _ in 0..3 {
        st.handle_key(ctrl_w).await;
    }
    assert_eq!(st.panels[0].format, ViewFormat::Tree, "Ctrl-W reaches Tree view");
    assert!(st.panels[0].tree.is_some(), "the tree is built on entering Tree view");
    // Entering Tree view doesn't move the console line off the panel's directory.
    assert_eq!(st.console_cwd().path, root, "console starts at the panel's directory");

    // Move the cursor onto the `alpha` child. Merely browsing must NOT change the
    // console line or the other panel — only Enter commits.
    let tree = st.panels[0].tree.as_ref().unwrap();
    let alpha_row = tree
        .rows
        .iter()
        .position(|n| n.label == "alpha")
        .expect("alpha listed under root");
    st.panels[0].tree.as_mut().unwrap().cursor = alpha_row;
    assert_eq!(st.console_cwd().path, root, "moving the cursor alone doesn't change the console");
    assert_eq!(st.panels[1].cwd, start_right, "moving the cursor alone doesn't move the panel");

    // Enter opens alpha's branch and commits: right panel + console both follow.
    st.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;

    // The inactive (right) panel moved to alpha…
    assert_eq!(st.panels[1].cwd.path, root.join("alpha"), "inactive panel follows the tree");
    // …and the command-line/console path now reflects the committed directory.
    assert_eq!(st.console_cwd().path, root.join("alpha"), "Enter updates the console path");
    assert_ne!(st.panels[1].cwd, start_right, "right panel actually moved");
    // …and alpha's branch opened, revealing its subdirectory.
    let tree = st.panels[0].tree.as_ref().unwrap();
    assert!(tree.rows[alpha_row].expanded, "alpha's branch is open");
    assert!(
        tree.rows.iter().any(|n| n.label == "inner"),
        "alpha's subdirectory is now visible"
    );
    // The active (tree) panel did not itself navigate.
    assert_eq!(st.panels[0].cwd, VfsPath::local(&root), "tree panel stays put");

    // Ctrl-W once more leaves Tree view and drops the tree.
    st.handle_key(ctrl_w).await;
    assert_eq!(st.panels[0].format, ViewFormat::Full, "Tree → Full completes the cycle");
    assert!(st.panels[0].tree.is_none(), "leaving Tree view drops the tree");
    // Back in a normal view the console line tracks the active panel again.
    assert_eq!(st.console_cwd(), VfsPath::local(&root), "console follows the active panel");

    std::fs::remove_dir_all(&root).ok();
}

/// The tree panel's title shows the committed directory (updated on Enter), and
/// mouse clicks drive it: one click positions the cursor, a double-click enters.
#[tokio::test]
async fn tree_view_title_and_mouse() {
    use crate::panel::ViewFormat;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_treemouse_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(root.join("alpha/inner")).unwrap();
    std::fs::create_dir_all(root.join("beta")).unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].format = ViewFormat::Full;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    st.set_format(0, ViewFormat::Tree).await;

    // Read the rendered title row (top border of the left panel).
    let title_text = |st: &mut AppState| -> String {
        let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
        term.draw(|f| crate::ui::draw(f, st)).unwrap();
        let b = term.backend().buffer();
        (0..b.area.width / 2).map(|x| b[(x, 1)].symbol().to_string()).collect()
    };

    // Initially the title shows the panel's own directory (the root).
    assert!(
        title_text(&mut st).contains(&root.file_name().unwrap().to_string_lossy().into_owned()),
        "title starts at the tree's directory"
    );

    // Render to record hit geometry, then find the row showing `alpha`.
    let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
    term.draw(|f| crate::ui::draw(f, &mut st)).unwrap();
    let hit = st.panels[0].hit.expect("tree records hit geometry");
    let alpha_idx = st
        .panels[0]
        .tree
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .position(|n| n.label == "alpha")
        .unwrap();
    // Map that tree index back to a screen row within the body.
    let arow = hit.body.y + (alpha_idx - hit.offset) as u16;
    let acol = hit.body.x + 1;
    let start_right = st.panels[1].cwd.clone();

    // One click positions the cursor on `alpha` without navigating anything.
    let click = |col, row| MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: col,
        row,
        modifiers: KeyModifiers::NONE,
    };
    st.handle_mouse(click(acol, arow)).await;
    assert_eq!(st.panels[0].tree.as_ref().unwrap().cursor, alpha_idx, "single click moves cursor");
    assert_eq!(st.panels[1].cwd, start_right, "single click doesn't navigate the other panel");

    // A second click on the same row enters: the other panel + title follow.
    st.handle_mouse(click(acol, arow)).await;
    assert_eq!(st.panels[1].cwd.path, root.join("alpha"), "double click enters the directory");
    assert!(
        title_text(&mut st).contains("alpha"),
        "the title now shows the committed directory"
    );

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
    st.panels[0].format = ViewFormat::Full; // don't inherit an ambient Tree/Brief config
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
    st.panels[0].format = ViewFormat::Full;
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
    st.panels[0].format = ViewFormat::Full;
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
    st.panels[0].format = ViewFormat::Full; // don't inherit an ambient Tree/Brief config
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
    let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
    // Theme lives in the "Visual" group (field index 6); Tab down to it, then
    // Enter opens its dropdown and moving the highlight previews the theme live.
    for _ in 0..6 {
        st.handle_key(key(KeyCode::Tab)).await;
    }
    st.handle_key(key(KeyCode::Enter)).await;
    st.handle_key(key(KeyCode::Down)).await;
    let previewed = st.theme.name.clone();
    assert_ne!(previewed, original, "scrolling the dropdown previews the theme live");
    // Enter confirms the highlighted option; the preview persists.
    st.handle_key(key(KeyCode::Enter)).await;
    assert_eq!(st.theme.name, previewed, "confirming keeps the previewed theme");
    // Esc cancels the settings dialog → revert to the original theme.
    st.handle_key(key(KeyCode::Esc)).await;
    assert_eq!(st.theme.name, original, "cancel should revert to the original theme");
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
    assert!(v.is_outline_open(), "help opens with the document outline shown");
}

#[tokio::test]
async fn edit_startup_opens_file_in_editor() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rc_edit_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("hello.txt");
    std::fs::write(&file, b"hello world").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    assert!(st.editor.is_none());
    st.open_path_in_editor(file.clone()).await;
    let ed = st.editor.as_ref().expect("/edit should open the editor");
    assert!(ed.contents().contains("hello world"), "the file's text is loaded");

    // A non-existent path opens an empty buffer (so it can be created on save).
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.open_path_in_editor(dir.join("brand-new.txt")).await;
    assert!(st.editor.is_some(), "a new file still opens the editor");
    assert!(st.editor.as_ref().unwrap().contents().is_empty(), "new file starts empty");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn editor_save_as_writes_and_retargets() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rc_sa_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let orig = dir.join("orig.txt");
    std::fs::write(&orig, b"data").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.open_path_in_editor(orig.clone()).await;
    assert!(st.editor.is_some());

    // Save As to a new path: the file is written and the editor retargets.
    let dest = dir.join("renamed.md");
    st.do_save_as(dest.clone()).await;
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "data", "buffer written to the new path");
    let ed = st.editor.as_ref().unwrap();
    assert_eq!(ed.name, "renamed.md", "editor name retargeted");
    assert_eq!(ed.path, VfsPath::local(&dest), "editor path retargeted");
    assert!(!ed.dirty, "saved buffer is no longer dirty");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn edit_with_no_file_opens_unnamed_buffer_that_saves_via_save_as() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rc_new_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // `rc /edit` with no file → a fresh, unnamed buffer.
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.open_new_editor();
    let ed = st.editor.as_ref().expect("a blank editor should open");
    assert!(ed.is_unnamed(), "a no-file editor buffer starts unnamed");
    assert!(ed.contents().is_empty(), "the blank buffer starts empty");

    // Pressing Save (F2) must route to the "Save as" browser rather than write
    // silently (there is no filename to write to yet).
    st.apply_editor_signal(crate::editor::EditorSignal::Save { close_after: false }).await;
    assert!(
        matches!(st.dialog, Some(Dialog::SaveAs(_))),
        "saving an unnamed buffer opens the Save-as dialog"
    );

    // The quit-time save path is guarded too (Save changes? → yes).
    st.dialog = None;
    st.save_editor(true).await;
    assert!(
        matches!(st.dialog, Some(Dialog::SaveAs(_))),
        "save-and-close also redirects an unnamed buffer to Save as"
    );

    // Completing "Save as" writes the file and the buffer is no longer unnamed.
    let dest = dir.join("chosen.txt");
    st.do_save_as(dest.clone()).await;
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "", "buffer written to the chosen path");
    let ed = st.editor.as_ref().unwrap();
    assert!(!ed.is_unnamed(), "the buffer is named after Save as");
    assert_eq!(ed.name, "chosen.txt", "editor name set from the chosen path");
    assert_eq!(ed.path, VfsPath::local(&dest), "editor path set from the chosen path");

    std::fs::remove_dir_all(&dir).ok();
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

fn mk_entry(name: &str) -> VfsEntry {
    VfsEntry {
        name: name.to_string(),
        kind: VfsKind::File,
        size: 0,
        mtime: None,
        atime: None,
        ctime: None,
        inode: None,
        mode: Some(0o644),
        uid: None,
        gid: None,
        symlink_target: None,
        symlink_broken: false,
    }
}

#[tokio::test]
async fn alt_s_or_ctrl_s_starts_empty_quick_search_then_letters_extend() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.init().await;
    st.panels[0].entries = vec![mk_entry("yo"), mk_entry("hello"), mk_entry("hi"), mk_entry("high")];
    st.panels[0].resort(); // stable: hello, hi, high, yo
    st.panels[0].cursor = 3; // start off the 'h' entries (on "yo")

    // Alt-S opens an empty search box; the cursor hasn't moved yet.
    st.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT)).await;
    assert_eq!(st.quick_search.as_ref().unwrap().query, "");
    assert_eq!(st.panels[0].entries[st.panels[0].cursor].name, "yo");

    // Every letter typed afterward is added to the box and jumps the cursor.
    st.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE)).await;
    assert_eq!(st.quick_search.as_ref().unwrap().query, "h");
    assert_eq!(st.panels[0].entries[st.panels[0].cursor].name, "hello");
    st.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)).await;
    assert_eq!(st.quick_search.as_ref().unwrap().query, "hi");
    assert_eq!(st.panels[0].entries[st.panels[0].cursor].name, "hi");
    st.handle_key(esc_key()).await;
    assert!(st.quick_search.is_none());

    // Ctrl-S starts it just the same.
    st.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)).await;
    assert_eq!(st.quick_search.as_ref().unwrap().query, "");
}

#[tokio::test]
async fn alt_menu_letter_opens_menu_but_alt_s_does_not() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.init().await;
    // Alt + a menu letter (F/O/C/L/R) opens that top menu now, even with quick
    // search enabled — Alt no longer starts a search on its own.
    for c in ['f', 'o', 'c', 'l', 'r'] {
        st.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)).await;
        assert!(st.menu.is_some(), "Alt+{c} opens a menu");
        assert!(st.quick_search.is_none(), "Alt+{c} does not start a search");
        st.menu = None;
    }
    // Alt-S starts a search, not a menu ('s' isn't a menu letter anyway).
    st.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT)).await;
    assert!(st.menu.is_none());
    assert!(st.quick_search.is_some(), "Alt-S starts a quick search");
}

#[tokio::test]
async fn quick_search_extends_while_alt_is_held() {
    // Once the box is open, holding Alt across letters (Alt-H, Alt-I, Alt-G)
    // must extend the query, not re-trigger anything.
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.init().await;
    st.panels[0].entries = vec![mk_entry("yo"), mk_entry("hello"), mk_entry("hi"), mk_entry("high")];
    st.panels[0].resort();
    st.panels[0].cursor = 3;

    let alt = KeyModifiers::ALT;
    st.handle_key(KeyEvent::new(KeyCode::Char('s'), alt)).await; // open the box
    st.handle_key(KeyEvent::new(KeyCode::Char('h'), alt)).await;
    assert_eq!(st.quick_search.as_ref().unwrap().query, "h");
    assert_eq!(st.panels[0].entries[st.panels[0].cursor].name, "hello");
    st.handle_key(KeyEvent::new(KeyCode::Char('i'), alt)).await;
    assert_eq!(st.quick_search.as_ref().unwrap().query, "hi");
    st.handle_key(KeyEvent::new(KeyCode::Char('g'), alt)).await;
    assert_eq!(st.quick_search.as_ref().unwrap().query, "hig");
    assert_eq!(st.panels[0].entries[st.panels[0].cursor].name, "high");
}

#[tokio::test]
async fn quick_search_survives_shift_and_empty_backspace() {
    use ratatui::crossterm::event::ModifierKeyCode;
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.init().await;
    st.panels[0].entries = vec![mk_entry("Alpha"), mk_entry("beta")];
    st.panels[0].resort();

    // Quick search is always available (no config toggle).
    st.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT)).await;
    assert!(st.quick_search.is_some());

    // A lone Shift key (reported on its own by the enhanced protocol) must NOT
    // dismiss the search — otherwise uppercase letters can't be typed.
    st.handle_key(KeyEvent::new(
        KeyCode::Modifier(ModifierKeyCode::LeftShift),
        KeyModifiers::SHIFT,
    ))
    .await;
    assert!(st.quick_search.is_some(), "Shift alone does not close the search");
    st.handle_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT)).await;
    assert_eq!(st.quick_search.as_ref().unwrap().query, "A");

    // Backspacing to empty keeps the (now empty) box open; pressing it again on
    // an empty box still doesn't close it.
    st.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)).await;
    assert_eq!(st.quick_search.as_ref().unwrap().query, "");
    st.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)).await;
    assert!(st.quick_search.is_some(), "an empty box stays open");

    // Esc dismisses.
    st.handle_key(esc_key()).await;
    assert!(st.quick_search.is_none());

    // Re-open with Ctrl-S; an arrow key dismisses it (and moves the cursor).
    st.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)).await;
    assert!(st.quick_search.is_some());
    st.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)).await;
    assert!(st.quick_search.is_none(), "an arrow key dismisses the search");
}

#[tokio::test]
async fn alt_arms_and_disarms_the_hotkey_hint() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.init().await;
    // A non-menu Alt letter just arms the menu-accelerator hint.
    st.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::ALT)).await;
    assert!(st.alt_hint, "Alt arms the accelerator hint");
    assert!(st.menu.is_none());
    // The next ordinary key clears it.
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

// -- Background operations ---------------------------------------------------

fn bg_key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn progress_update(id: TaskId, verb: &'static str, done: u64, total: u64) -> ProgressUpdate {
    ProgressUpdate {
        id,
        verb,
        current_name: "file.bin".into(),
        file_done: done,
        file_total: total,
        total_done: done,
        total_total: total,
        files_done: 0,
        files_total: 1,
    }
}

/// The progress dialog's "To background" button/keys map to the right results.
#[test]
fn progress_dialog_background_keys() {
    let mut d = ProgressDialog::new(9, "Copying");
    d.backgroundable = true;
    // 'b' backgrounds; Esc/q/a abort; Enter activates the focused button
    // (default focus = To background).
    assert!(matches!(d.handle_key(bg_key(KeyCode::Char('b'))), DialogResult::Background(9)));
    assert!(matches!(d.handle_key(bg_key(KeyCode::Enter)), DialogResult::Background(9)));
    assert!(matches!(d.handle_key(bg_key(KeyCode::Char('a'))), DialogResult::Abort(9)));
    assert!(matches!(d.handle_key(bg_key(KeyCode::Esc)), DialogResult::Abort(9)));
    // Tab moves focus to Abort → Enter now aborts.
    d.handle_key(bg_key(KeyCode::Tab));
    assert!(matches!(d.handle_key(bg_key(KeyCode::Enter)), DialogResult::Abort(9)));

    // A modal (non-backgroundable) dialog keeps the old behaviour: Enter aborts.
    let mut m = ProgressDialog::new(1, "Searching");
    assert!(matches!(m.handle_key(bg_key(KeyCode::Enter)), DialogResult::Abort(1)));
}

/// The Background operations list: Enter foregrounds, Delete aborts, Esc closes.
#[test]
fn background_ops_list_keys() {
    use crate::ui::dialog::{BackgroundOpsDialog, BgRow};
    let mut d = BackgroundOpsDialog::new(vec![
        BgRow { id: 3, label: "Copying a".into(), ratio: 0.5 },
        BgRow { id: 4, label: "Moving b".into(), ratio: 0.1 },
    ]);
    // Enter on the first row foregrounds it.
    assert!(matches!(
        d.handle_key(bg_key(KeyCode::Enter)),
        DialogResult::Submit(Submit::ForegroundTask(3))
    ));
    // Move down, Delete aborts the second row.
    d.handle_key(bg_key(KeyCode::Down));
    assert!(matches!(d.handle_key(bg_key(KeyCode::Delete)), DialogResult::Abort(4)));
    assert!(matches!(d.handle_key(bg_key(KeyCode::Esc)), DialogResult::Cancel));
}

/// Sending a running transfer to the background dismisses the dialog but keeps
/// the task alive (and tracked for the mini bar / list).
#[tokio::test]
async fn to_background_keeps_task_and_lists_it() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_bg_{}_{nanos}", std::process::id()));
    let left = root.join("left");
    let right = root.join("right");
    std::fs::create_dir_all(&left).unwrap();
    std::fs::create_dir_all(&right).unwrap();
    std::fs::write(left.join("big.bin"), vec![0u8; 4096]).unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&left);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    st.panels[1].cwd = VfsPath::local(&right);
    st.panels[1].backend = st.registry.local();
    st.panels[1].reload().await.unwrap();

    st.handle_submit(Submit::Copy(vec![VfsPath::local(left.join("big.bin"))], right.to_string_lossy().into_owned())).await;
    let id = match &st.dialog {
        Some(Dialog::Progress(p)) => p.id,
        _ => panic!("a transfer progress dialog should be showing"),
    };
    assert!(st.tasks.contains_key(&id) && st.task_progress.contains_key(&id));

    // Send to background: dialog closes, task stays.
    st.handle_dialog_result(DialogResult::Background(id)).await;
    assert!(st.dialog.is_none(), "progress dialog dismissed");
    assert!(st.tasks.contains_key(&id), "task keeps running");
    assert!(st.task_progress.contains_key(&id), "still tracked for the mini bar");
    let (_, _, count) = st.background_summary().expect("one background op");
    assert_eq!(count, 1);

    // The Background operations list opens and shows it.
    st.open_background_ops();
    assert!(matches!(st.dialog, Some(Dialog::BackgroundOps(_))));

    std::fs::remove_dir_all(&root).ok();
}

/// A conflict on a backgrounded transfer foregrounds it under the overwrite
/// prompt (rebuilt from its snapshot), ready to restore on answer.
#[tokio::test]
async fn conflict_foregrounds_a_background_transfer() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    // A backgrounded transfer with no foreground dialog.
    st.task_progress.insert(
        7,
        BgTransfer { verb: "Copying", update: Some(progress_update(7, "Copying", 10, 100)), schemes: vec![] },
    );
    assert!(st.dialog.is_none());

    let info = crate::ops::progress::ConflictInfo {
        id: 7,
        name: "dup.txt".into(),
        new_path: "/a/dup.txt".into(),
        new_size: 5,
        new_mtime: None,
        old_path: "/b/dup.txt".into(),
        old_size: 3,
        old_mtime: None,
    };
    st.apply_event(AppEvent::Conflict(info)).await;
    assert!(matches!(st.dialog, Some(Dialog::Overwrite(_))), "overwrite prompt shown");
    match &st.stashed_progress {
        Some(p) => assert_eq!(p.id, 7, "the conflicting transfer is stashed to restore on answer"),
        None => panic!("progress dialog should be stashed under the overwrite prompt"),
    }
}

/// Foregrounding a background task via `Submit::ForegroundTask` re-opens its
/// progress dialog seeded from the snapshot.
#[tokio::test]
async fn foreground_task_reopens_progress_dialog() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    let (reply, _r) = tokio::sync::mpsc::channel(1);
    st.tasks.insert(
        5,
        crate::ops::TaskHandle { id: 5, cancel: crate::ops::CancelToken::new(), reply },
    );
    st.task_progress.insert(
        5,
        BgTransfer { verb: "Moving", update: Some(progress_update(5, "Moving", 40, 80)), schemes: vec![] },
    );

    st.handle_submit(Submit::ForegroundTask(5)).await;
    match &st.dialog {
        Some(Dialog::Progress(p)) => {
            assert_eq!(p.id, 5);
            assert_eq!(p.total_done, 40, "seeded from the latest snapshot");
        }
        _ => panic!("progress dialog should re-open for the foregrounded task"),
    }
}

/// The menu-bar aggregate sums bytes across all background transfers.
#[test]
fn background_summary_aggregates() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    assert!(st.background_summary().is_none(), "nothing running");
    st.task_progress.insert(1, BgTransfer { verb: "Copying", update: Some(progress_update(1, "Copying", 30, 100)), schemes: vec![] });
    st.task_progress.insert(2, BgTransfer { verb: "Moving", update: Some(progress_update(2, "Moving", 20, 100)), schemes: vec![] });
    let (done, total, count) = st.background_summary().unwrap();
    assert_eq!((done, total, count), (50, 200, 2));
}


/// The open Background operations list advances live as progress arrives, and
/// closes once the last transfer finishes.
#[tokio::test]
async fn background_ops_list_updates_live() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    let (reply, _r) = tokio::sync::mpsc::channel(1);
    st.tasks.insert(1, crate::ops::TaskHandle { id: 1, cancel: crate::ops::CancelToken::new(), reply });
    st.task_progress.insert(1, BgTransfer { verb: "Copying", update: Some(progress_update(1, "Copying", 10, 100)), schemes: vec![] });

    st.open_background_ops();
    let ratio0 = match &st.dialog {
        Some(Dialog::BackgroundOps(d)) => d.row_snapshot()[0].1,
        _ => panic!("list open"),
    };
    assert!((ratio0 - 0.10).abs() < 1e-9);

    // A progress update advances the row live.
    st.apply_event(AppEvent::Progress(progress_update(1, "Copying", 70, 100))).await;
    let ratio1 = match &st.dialog {
        Some(Dialog::BackgroundOps(d)) => d.row_snapshot()[0].1,
        _ => panic!("list still open"),
    };
    assert!((ratio1 - 0.70).abs() < 1e-9, "row advanced to 70%");

    // Completing the last transfer closes the (now empty) list.
    st.apply_event(AppEvent::TaskDone { id: 1, outcome: crate::ops::progress::TaskOutcome::Done }).await;
    assert!(st.dialog.is_none(), "list closes when the last op finishes");
}

/// `run_program_cmd` builds a shell-safe command from an executable's path.
#[test]
fn run_program_cmd_quotes_the_path() {
    let cmd = run_program_cmd(std::path::Path::new("/home/u/my tool"));
    // The space must be quoted/escaped so the shell treats it as one argument.
    assert!(cmd.contains("my tool") && cmd != "/home/u/my tool", "path is shell-quoted: {cmd}");
}

/// Confirming the "Execute file" dialog runs the program in the foreground
/// (the dialog result becomes a `RunCommand`).
#[tokio::test]
async fn run_program_submit_executes_in_foreground() {
    use crate::ui::dialog::{DialogResult, Submit};
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    let path = std::path::PathBuf::from("/usr/local/bin/tool");
    let flow = st
        .handle_dialog_result(DialogResult::Submit(Submit::RunProgram(path.clone())))
        .await;
    match flow {
        Flow::RunCommand(cmd) => assert!(cmd.contains("tool"), "runs the program: {cmd}"),
        _ => panic!("RunProgram should run the executable in the foreground"),
    }
    assert!(st.dialog.is_none(), "the confirm dialog is dismissed");
}

/// Pressing Enter on an executable file with no MIME handler runs it directly
/// (ELF binaries, scripts) rather than trying to open it with an application.
#[cfg(all(unix, target_os = "linux"))]
#[tokio::test]
async fn enter_on_executable_binary_runs_it() {
    use std::os::unix::fs::PermissionsExt;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_exec_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    // A tiny ELF-looking binary with no extension: no desktop MIME handler.
    let bin = root.join("runme");
    std::fs::write(&bin, b"\x7fELF\x02\x01\x01\x00rest").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].format = ViewFormat::Full;
    st.config.confirm_execute = false;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    let i = st.panels[0].entries.iter().position(|e| e.name == "runme").unwrap();
    st.panels[0].cursor = i;

    let flow = st.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
    match flow {
        Flow::RunCommand(cmd) => assert!(cmd.contains("runme"), "executes the binary: {cmd}"),
        _ => panic!("Enter on an executable with no handler should run it"),
    }
    // A non-executable file with no handler must NOT be executed.
    let doc = root.join("notes");
    std::fs::write(&doc, b"plain text").unwrap();
    st.panels[0].reload().await.unwrap();
    let j = st.panels[0].entries.iter().position(|e| e.name == "notes").unwrap();
    st.panels[0].cursor = j;
    let flow = st.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
    assert!(matches!(flow, Flow::Continue), "a non-executable is not run");

    std::fs::remove_dir_all(&root).ok();
}

/// With "confirm execute" enabled, Enter on an executable asks first (via the
/// "Execute file" dialog) instead of running immediately.
#[cfg(all(unix, target_os = "linux"))]
#[tokio::test]
async fn enter_on_executable_asks_when_confirm_enabled() {
    use std::os::unix::fs::PermissionsExt;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_execc_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let bin = root.join("runme");
    std::fs::write(&bin, b"\x7fELF\x02\x01\x01\x00rest").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].format = ViewFormat::Full;
    st.config.confirm_execute = true;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    let i = st.panels[0].entries.iter().position(|e| e.name == "runme").unwrap();
    st.panels[0].cursor = i;

    let flow = st.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
    assert!(matches!(flow, Flow::Continue), "confirm defers the run");
    assert!(matches!(st.dialog, Some(Dialog::Confirm(_))), "an execute-confirm dialog opens");

    std::fs::remove_dir_all(&root).ok();
}

/// Ctrl-O opens the subshell normally, but a nested instance (started from
/// inside another Rat Commander's subshell) has it disabled and explains why.
#[tokio::test]
async fn ctrl_o_is_disabled_for_a_nested_instance() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    let ctrl_o = KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL);

    // Normal instance: Ctrl-O drops to the subshell.
    st.subshell_disabled = false;
    assert!(matches!(st.handle_key(ctrl_o).await, Flow::SubShell), "Ctrl-O opens the subshell");

    // Nested instance: Ctrl-O is inert and shows an explanatory dialog.
    st.subshell_disabled = true;
    let flow = st.handle_key(ctrl_o).await;
    assert!(matches!(flow, Flow::Continue), "Ctrl-O must not open a subshell when nested");
    assert!(matches!(st.dialog, Some(Dialog::Message(_))), "it explains that the subshell is off");
}

/// Alt-Enter copies the name under the cursor to the command line; Alt-P recalls
/// history; Alt-H opens the Shell History window and selecting recalls a command.
#[tokio::test]
async fn command_line_history_and_alt_enter() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_hist_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("report.txt"), b"x").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.cmd.history.clear(); // ignore any real persisted history on this machine
    st.panels[0].format = ViewFormat::Full;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    let i = st.panels[0].entries.iter().position(|e| e.name == "report.txt").unwrap();
    st.panels[0].cursor = i;

    let alt = |c| KeyEvent::new(c, KeyModifiers::ALT);
    let alt_c = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT);
    let plain = |c| KeyEvent::new(c, KeyModifiers::NONE);

    // Alt-Enter appends the filename (with a trailing space).
    st.handle_key(alt(KeyCode::Enter)).await;
    assert_eq!(st.cmd.buffer, "report.txt ", "Alt-Enter copies the filename");
    st.cmd.clear();

    // Build some history by "running" commands (Enter records via take()).
    for c in ["ls", "pwd"] {
        st.cmd.set(c.to_string());
        st.handle_key(plain(KeyCode::Enter)).await; // returns RunCommand; records history
    }
    assert_eq!(st.cmd.history, vec!["ls".to_string(), "pwd".to_string()]);

    // Alt-P recalls the most recent, then the one before it.
    st.handle_key(alt_c('p')).await;
    assert_eq!(st.cmd.buffer, "pwd");
    st.handle_key(alt_c('p')).await;
    assert_eq!(st.cmd.buffer, "ls");
    st.cmd.clear();

    // Alt-H opens the Shell History window.
    st.handle_key(alt_c('h')).await;
    assert!(matches!(st.dialog, Some(Dialog::ShellHistory(_))), "Alt-H opens the history window");
    // Up selects the older entry ("ls"); Enter recalls it without running.
    st.handle_key(plain(KeyCode::Up)).await;
    let flow = st.handle_key(plain(KeyCode::Enter)).await;
    assert!(matches!(flow, Flow::Continue), "recall does not run the command");
    assert!(st.dialog.is_none(), "the window closes");
    assert_eq!(st.cmd.buffer, "ls", "the chosen command is on the command line");

    std::fs::remove_dir_all(&root).ok();
}

/// The command line supports Emacs/readline editing: cursor motions, word
/// motions, kill-to-end and yank.
#[tokio::test]
async fn command_line_readline_editing() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.init().await;
    st.cmd.history.clear();
    let plain = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
    let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
    let alt = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT);

    for c in "echo hello".chars() {
        st.handle_key(plain(c)).await;
    }
    assert_eq!(st.cmd.buffer, "echo hello");
    assert_eq!(st.cmd.cursor, 10);

    // C-a to start, C-e to end.
    st.handle_key(ctrl('a')).await;
    assert_eq!(st.cmd.cursor, 0);
    st.handle_key(ctrl('e')).await;
    assert_eq!(st.cmd.cursor, 10);

    // Alt-b twice walks back over the two words; Alt-f walks forward one word.
    st.handle_key(alt('b')).await;
    assert_eq!(st.cmd.cursor, 5, "start of 'hello'");
    st.handle_key(alt('b')).await;
    assert_eq!(st.cmd.cursor, 0, "start of 'echo'");
    st.handle_key(alt('f')).await;
    assert_eq!(st.cmd.cursor, 4, "end of 'echo'");

    // C-k kills " hello" to the end; C-y yanks it back.
    st.handle_key(ctrl('k')).await;
    assert_eq!(st.cmd.buffer, "echo");
    st.handle_key(ctrl('e')).await;
    st.handle_key(ctrl('y')).await;
    assert_eq!(st.cmd.buffer, "echo hello");

    // C-b (char left), C-d (delete at point), C-h (delete previous).
    st.handle_key(ctrl('a')).await;
    st.handle_key(ctrl('d')).await; // delete 'e'
    assert_eq!(st.cmd.buffer, "cho hello");
    st.handle_key(ctrl('f')).await; // cursor after 'c'
    st.handle_key(ctrl('h')).await; // delete 'c'
    assert_eq!(st.cmd.buffer, "ho hello");
}

/// C-E, C-W and Alt-F keep their panel meaning while the command line is empty,
/// but edit the text once the line has content.
#[tokio::test]
async fn command_line_readline_conflicts_respect_empty_line() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.init().await;
    st.cmd.history.clear();
    st.panels[st.active].format = ViewFormat::Full;
    let plain = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
    let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
    let alt = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT);

    // -- Empty line: the keys keep their panel/menu meaning. --
    let fmt = st.panels[st.active].format;
    st.handle_key(ctrl('w')).await;
    assert_ne!(st.panels[st.active].format, fmt, "C-W cycles the view when empty");
    let rev = st.panels[st.active].sort.reverse;
    st.handle_key(ctrl('e')).await;
    assert_ne!(st.panels[st.active].sort.reverse, rev, "C-E reverses sort when empty");
    st.handle_key(alt('f')).await;
    assert!(st.menu.is_some(), "Alt-F opens the File menu when empty");
    st.menu = None;

    // -- With text, the same keys edit the line. --
    for c in "ab cd".chars() {
        st.handle_key(plain(c)).await;
    }
    let fmt = st.panels[st.active].format;
    let rev = st.panels[st.active].sort.reverse;
    st.handle_key(ctrl('a')).await;
    st.handle_key(ctrl('w')).await; // no mark → no-op, but NOT a view cycle
    assert_eq!(st.panels[st.active].format, fmt, "C-W does not cycle the view with text");
    st.handle_key(ctrl('e')).await;
    assert_eq!(st.cmd.cursor, 5, "C-E goes to end of line");
    assert_eq!(st.panels[st.active].sort.reverse, rev, "C-E does not reverse sort with text");
    st.handle_key(ctrl('a')).await;
    st.handle_key(alt('f')).await;
    assert!(st.menu.is_none(), "Alt-F edits (word forward) with text, no menu");
    assert_eq!(st.cmd.cursor, 2, "Alt-F moved to end of 'ab'");
}

#[tokio::test]
async fn ctrl_p_opens_command_palette_and_runs_a_command() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.init().await;
    // Give the two panels distinct locations so a swap is observable. (Swap does
    // not reload, so the paths need not exist.)
    st.panels[0].cwd = VfsPath::local("/marker/left");
    st.panels[1].cwd = VfsPath::local("/marker/right");
    let left = st.panels[0].cwd.clone();
    let right = st.panels[1].cwd.clone();

    // Ctrl-P opens the fuzzy palette.
    st.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL)).await;
    assert!(
        matches!(st.dialog, Some(Dialog::CommandPalette(_))),
        "Ctrl-P opens the command palette"
    );

    // Type to filter to the unique "Swap panels" command, then run it. This
    // exercises the whole path: query editing → Submit(Palette) → run_menu_action.
    for c in "swap panels".chars() {
        st.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)).await;
    }
    st.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;

    assert!(st.dialog.is_none(), "running an entry closes the palette");
    assert_eq!(st.panels[0].cwd, right, "Swap panels ran: left shows the old right");
    assert_eq!(st.panels[1].cwd, left, "Swap panels ran: right shows the old left");

    // Esc closes the palette without acting.
    st.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL)).await;
    assert!(matches!(st.dialog, Some(Dialog::CommandPalette(_))));
    st.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).await;
    assert!(st.dialog.is_none(), "Esc closes the palette");
}

#[tokio::test]
async fn directory_history_filter_and_hotlist() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rc_hist_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("a.txt"), b"a").unwrap();
    std::fs::write(root.join("b.rs"), b"b").unwrap();
    std::fs::write(root.join("c.rs"), b"c").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();

    let alt = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT);

    // -- Back / forward history --
    let sub_idx = st.panels[0].entries.iter().position(|e| e.name == "sub").unwrap();
    st.panels[0].cursor = sub_idx;
    st.enter_dir().await;
    assert!(st.panels[0].cwd.path.ends_with("sub"), "entered sub");
    assert!(st.panels[0].can_back(), "entering records history");
    assert!(!st.panels[0].can_forward());

    st.handle_key(alt('y')).await; // Alt-y = back
    assert_eq!(st.panels[0].cwd.path, root, "Alt-y returns to root");
    assert!(st.panels[0].can_forward(), "back enables forward");

    st.handle_key(alt('u')).await; // Alt-u = forward
    assert!(st.panels[0].cwd.path.ends_with("sub"), "Alt-u goes forward to sub");

    st.handle_key(alt('y')).await; // back to root; forward now holds sub
    assert_eq!(st.panels[0].cwd.path, root);
    assert!(st.panels[0].can_forward());

    // -- Clicking a panel's ▶ arrow steps forward (as the renderer places it) --
    st.panels[0].fwd_arrow = Some(Rect::new(2, 1, 1, 1));
    st.last_area = Rect::new(0, 0, 80, 24);
    st.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 2,
        row: 1,
        modifiers: KeyModifiers::NONE,
    })
    .await;
    assert!(st.panels[0].cwd.path.ends_with("sub"), "clicking ▶ steps forward");
    st.handle_key(alt('y')).await; // back to root for the filter test
    assert_eq!(st.panels[0].cwd.path, root);

    // -- Persistent listing filter --
    st.apply_panel_filter(0, "*.rs".to_string()).await;
    let names: Vec<String> = st.panels[0].entries.iter().map(|e| e.name.clone()).collect();
    assert!(names.contains(&"b.rs".to_string()) && names.contains(&"c.rs".to_string()));
    assert!(!names.contains(&"a.txt".to_string()), "filter hides a.txt");
    assert!(!names.contains(&"sub".to_string()), "filter hides the sub dir");
    // Clearing the filter restores everything.
    st.apply_panel_filter(0, String::new()).await;
    assert!(st.panels[0].entries.iter().any(|e| e.name == "a.txt"), "cleared filter shows a.txt");

    // -- Hotlist opens on Ctrl-\ --
    st.handle_key(KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::CONTROL)).await;
    assert!(matches!(st.dialog, Some(Dialog::Hotlist(_))), "Ctrl-\\ opens the hotlist");
    st.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).await;
    assert!(st.dialog.is_none());

    // -- Alt-I opens the filter prompt --
    st.handle_key(alt('i')).await;
    assert!(
        matches!(st.dialog, Some(Dialog::Input(_))),
        "Alt-I opens the filter input"
    );

    let _ = std::fs::remove_dir_all(&root);
}
