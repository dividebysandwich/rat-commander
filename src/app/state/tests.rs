use super::*;
use crate::app::state::watch::{FsEvent, FsKind};
use crate::util::async_bridge;

#[tokio::test]
async fn enters_zip_archive_and_lists_contents() {
    // Build a temp dir with a zip to browse.
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rc_nav_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("sub/file.txt"), b"hi").unwrap();
    std::fs::write(root.join("top.txt"), b"top").unwrap();
    let zip = root.join("test.zip");
    archive::create_archive(ArchiveFormat::Zip, &zip, &[root.join("sub"), root.join("top.txt")])
        .unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();

    // Put the cursor on the zip and "enter" it.
    let idx = st.panels[0].entries.iter().position(|e| e.name == "test.zip").unwrap();
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

    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rc_ext_nav_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("sub/file.txt"), b"hi").unwrap();
    std::fs::write(root.join("top.txt"), b"top").unwrap();
    assert!(
        std::process::Command::new("zip")
            .current_dir(&root)
            .arg("-r")
            .arg("bundle.pk3")
            .arg("sub")
            .arg("top.txt")
            .output()
            .unwrap()
            .status
            .success()
    );

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    // Map .pk3 → the uzip extfs script (native ArchiveFs ignores .pk3).
    st.ext_rules = crate::ext::parse("shell/.pk3\n    Open=%cd %p/uzip://\n");
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();

    let idx = st.panels[0].entries.iter().position(|e| e.name == "bundle.pk3").unwrap();
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

    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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

    let idx = st.panels[0].entries.iter().position(|e| e.name == "secret").unwrap();
    st.panels[0].cursor = idx;
    st.enter_dir().await;

    if denied {
        assert_eq!(st.panels[0].cwd.path, root, "should not have entered the unreadable directory");
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
    let remote =
        VfsPath { scheme: "scp-0".to_string(), path: PathBuf::from("/home/user"), container: None };
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
        key_file: String::new(),
        key_passphrase: String::new(),
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
    let remote_path = VfsPath { scheme: "sftp-0".into(), path: "/srv".into(), container: None };
    let id = setup_remote_panel(&mut st, 0, "sftp-0", "/srv");

    st.disconnect_session(id).await;
    assert!(st.sessions.is_empty(), "session record dropped");
    assert!(st.registry.resolve(&remote_path).is_err(), "backend unregistered");
    assert!(!st.panels[0].cwd.is_remote(), "the panel on it went local");
}

#[tokio::test]
async fn go_local_restores_remembered_dir_not_process_cwd() {
    // Create a real, readable directory distinct from the process cwd.
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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

/// On startup the initially-active panel opens at the current directory (where
/// rc was launched), not its last-saved directory.
#[tokio::test]
async fn startup_active_panel_opens_at_current_directory() {
    let (tx, _rx) = async_bridge::channel();
    let st = AppState::new(tx);
    let cwd = std::env::current_dir().unwrap();
    assert_eq!(
        st.panels[st.active].cwd,
        VfsPath::local(&cwd),
        "the active panel starts at the process cwd regardless of the saved session"
    );
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
    let remote =
        VfsPath { scheme: "scp-0".into(), path: PathBuf::from("/home/user"), container: None };
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
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
    let set = |names: &[&str]| -> HashSet<String> { names.iter().map(|s| s.to_string()).collect() };

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

#[tokio::test]
async fn compare_files_opens_binary_files_byte_by_byte_and_text_files_line_by_line() {
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rc_cmpf_{}_{nanos}", std::process::id()));
    let (da, db) = (root.join("a"), root.join("b"));
    std::fs::create_dir_all(&da).unwrap();
    std::fs::create_dir_all(&db).unwrap();
    std::fs::write(da.join("fw.bin"), b"\x7fELF\x00\x01\x02\x03").unwrap();
    std::fs::write(db.join("fw.bin"), b"\x7fELF\x00\x01\xff\x03").unwrap();
    std::fs::write(da.join("notes.txt"), b"one\ntwo\n").unwrap();
    std::fs::write(db.join("notes.txt"), b"one\nthree\n").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    for (side, dir) in [(0, &da), (1, &db)] {
        st.panels[side].cwd = VfsPath::local(dir);
        st.panels[side].backend = st.registry.local();
        st.panels[side].reload().await.unwrap();
    }
    let point = |st: &mut AppState, name: &str| {
        for p in &mut st.panels {
            p.cursor = p.entries.iter().position(|e| e.name == name).unwrap();
        }
    };

    point(&mut st, "fw.bin");
    st.open_compare_files().await;
    assert!(st.diffview.is_none(), "a binary file isn't split into lines");
    let hd = st.hexdiff.as_mut().expect("the byte-by-byte view");
    while hd.poll() {
        std::thread::yield_now();
    }
    assert_eq!(hd.runs_status().0, 1);
    // F5 asks for an offset, which moves the cursor.
    let f5 = KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE);
    st.handle_key(f5).await;
    assert!(matches!(st.dialog, Some(Dialog::Input(_))));
    st.dialog = None;
    st.handle_submit(Submit::HexDiffGoto("0x6".into())).await;
    assert_eq!(st.hexdiff.as_ref().unwrap().cursor, 6);
    st.handle_key(KeyEvent::new(KeyCode::F(10), KeyModifiers::NONE)).await;
    assert!(st.hexdiff.is_none());

    point(&mut st, "notes.txt");
    st.open_compare_files().await;
    assert!(st.hexdiff.is_none() && st.diffview.is_some(), "text still diffs by line");
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
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
                let _ = h.reply.try_send(crate::ops::progress::TaskReply::Overwrite(
                    OverwriteDecision::SkipOnce,
                ));
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
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
    assert!(
        !left.join("a.txt").exists() && !left.join("b.txt").exists(),
        "originals removed (moved)"
    );

    std::fs::remove_dir_all(&root).ok();
}

/// F6 on a single directory with a bare new name renames it *in place* (in the
/// source panel), not into the opposite panel, and lands the cursor on it.
#[tokio::test]
async fn f6_bare_name_renames_in_place_and_focuses() {
    use crate::ui::dialog::Submit;
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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

/// An F6 target carrying a `*` is a name mask, not a literal name: the wildcard
/// stands for the file's own name, so `*.new` appends a suffix rather than
/// creating a file actually called `*.new` (which is what used to happen).
#[tokio::test]
async fn rename_expands_a_wildcard_to_the_source_name() {
    use crate::ui::dialog::Submit;
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rc_rn_glob_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(root.join("adir")).unwrap();
    std::fs::write(root.join("test.txt"), b"hi").unwrap();

    let (tx, mut rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    for i in 0..2 {
        st.panels[i].cwd = VfsPath::local(&root);
        st.panels[i].backend = st.registry.local();
        st.panels[i].reload().await.unwrap();
    }
    st.active = 0;

    st.handle_submit(Submit::Move(vec![VfsPath::local(root.join("test.txt"))], "*.new".into()))
        .await;
    drain_taskdone(&mut st, &mut rx).await;

    assert!(root.join("test.txt.new").is_file(), "the wildcard took the file's own name");
    assert!(!root.join("test.txt").exists(), "and the original was moved, not copied");
    assert!(!root.join("*.new").exists(), "nothing literally named `*.new` was created");
    // The cursor follows the expanded name, not the mask.
    let p = &st.panels[0];
    assert_eq!(p.entries[p.cursor].name, "test.txt.new");

    // Directories go through the same path.
    st.handle_submit(Submit::Move(vec![VfsPath::local(root.join("adir"))], "*.bak".into())).await;
    drain_taskdone(&mut st, &mut rx).await;
    assert!(root.join("adir.bak").is_dir(), "a directory renames through the mask too");

    std::fs::remove_dir_all(&root).ok();
}

/// A wildcard target renames a whole marked set, each file through its own name.
/// Without the mask this took the `*` for a directory name and moved everything
/// into one directory called `*.bak`.
#[tokio::test]
async fn a_wildcard_target_renames_every_marked_file() {
    use crate::ui::dialog::Submit;
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rc_rn_many_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for n in ["one.txt", "two.txt", "three.txt"] {
        std::fs::write(root.join(n), b"x").unwrap();
    }

    let (tx, mut rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    for i in 0..2 {
        st.panels[i].cwd = VfsPath::local(&root);
        st.panels[i].backend = st.registry.local();
        st.panels[i].reload().await.unwrap();
    }
    st.active = 0;

    let sources: Vec<VfsPath> =
        ["one.txt", "two.txt", "three.txt"].iter().map(|n| VfsPath::local(root.join(n))).collect();
    st.handle_submit(Submit::Move(sources, "*.bak".into())).await;
    drain_taskdone(&mut st, &mut rx).await;

    for n in ["one.txt.bak", "two.txt.bak", "three.txt.bak"] {
        assert!(root.join(n).is_file(), "{n} should exist");
    }
    assert!(!root.join("*.bak").exists(), "no directory was made out of the mask");

    std::fs::remove_dir_all(&root).ok();
}

/// A target with no wildcard is still a literal name, and one that names an
/// existing directory still means "move into it" — the mask must not change
/// either.
#[tokio::test]
async fn a_target_without_a_wildcard_is_unchanged() {
    use crate::ui::dialog::Submit;
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rc_rn_plain_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(root.join("into")).unwrap();
    std::fs::write(root.join("a.txt"), b"x").unwrap();
    std::fs::write(root.join("b.txt"), b"y").unwrap();

    let (tx, mut rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    for i in 0..2 {
        st.panels[i].cwd = VfsPath::local(&root);
        st.panels[i].backend = st.registry.local();
        st.panels[i].reload().await.unwrap();
    }
    st.active = 0;

    st.handle_submit(Submit::Move(vec![VfsPath::local(root.join("a.txt"))], "plain.txt".into()))
        .await;
    drain_taskdone(&mut st, &mut rx).await;
    assert!(root.join("plain.txt").is_file(), "a literal name still renames");

    st.handle_submit(Submit::Move(vec![VfsPath::local(root.join("b.txt"))], "into".into())).await;
    drain_taskdone(&mut st, &mut rx).await;
    assert!(root.join("into/b.txt").is_file(), "an existing directory still means 'move into it'");

    std::fs::remove_dir_all(&root).ok();
}

/// Renaming when both panels show the same directory still lands the *active*
/// panel's cursor on the new name (not the other panel showing the same dir).
#[tokio::test]
async fn rename_focuses_active_panel_when_both_show_same_dir() {
    use crate::ui::dialog::Submit;
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
        find_all: false,
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
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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

    // Alt-T: Full → Brief → Details → Tree.
    let alt_t = KeyEvent::new(KeyCode::Char('t'), KeyModifiers::ALT);
    for _ in 0..3 {
        st.handle_key(alt_t).await;
    }
    assert_eq!(st.panels[0].format, ViewFormat::Tree, "Alt-T reaches Tree view");
    assert!(st.panels[0].tree.is_some(), "the tree is built on entering Tree view");
    // Entering Tree view doesn't move the console line off the panel's directory.
    assert_eq!(st.console_cwd().path, root, "console starts at the panel's directory");

    // Move the cursor onto the `alpha` child. Merely browsing must NOT change the
    // console line or the other panel — only Enter commits.
    let tree = st.panels[0].tree.as_ref().unwrap();
    let alpha_row =
        tree.rows.iter().position(|n| n.label == "alpha").expect("alpha listed under root");
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
    assert!(tree.rows.iter().any(|n| n.label == "inner"), "alpha's subdirectory is now visible");
    // The active (tree) panel did not itself navigate.
    assert_eq!(st.panels[0].cwd, VfsPath::local(&root), "tree panel stays put");

    // Alt-T once more leaves Tree view for the 3D view, dropping the tree…
    st.handle_key(alt_t).await;
    assert_eq!(st.panels[0].format, ViewFormat::Space3d, "Tree → 3D");
    assert!(st.panels[0].tree.is_none(), "leaving Tree view drops the tree");
    assert!(st.panels[0].space3d.is_some(), "the 3D view builds its state");
    // …then the thumbnail grid, and once more completes the cycle back to Full.
    st.handle_key(alt_t).await;
    assert_eq!(st.panels[0].format, ViewFormat::Thumbs, "3D → Thumbnails");
    assert!(st.panels[0].space3d.is_none(), "leaving the 3D view drops its state");
    st.handle_key(alt_t).await;
    assert_eq!(st.panels[0].format, ViewFormat::Full, "Thumbnails → Full completes the cycle");
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

    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
    let alpha_idx =
        st.panels[0].tree.as_ref().unwrap().rows.iter().position(|n| n.label == "alpha").unwrap();
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
    assert!(title_text(&mut st).contains("alpha"), "the title now shows the committed directory");

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn find_duplicates_marks_by_criteria() {
    use crate::ui::dialog::DupCriteria;
    use std::collections::HashSet;
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
        p.entries
            .iter()
            .filter(|e| p.selection.is_marked(&e.name))
            .map(|e| e.name.clone())
            .collect()
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
    let some = |s: &str| Some(s.to_string());
    assert_eq!(parse_cd("cd"), some(""));
    assert_eq!(parse_cd("cd /tmp"), some("/tmp"));
    assert_eq!(parse_cd("  cd   foo  "), some("foo"));
    assert_eq!(parse_cd("cd\tfoo"), some("foo"));
    assert_eq!(parse_cd("cd -"), some("-"));
    assert_eq!(parse_cd("cd ~/src"), some("~/src"));
    assert_eq!(parse_cd("cdfoo"), None);
    assert_eq!(parse_cd("ls"), None);
}

#[test]
fn parse_cd_unquotes_and_leaves_shell_syntax_to_the_shell() {
    let some = |s: &str| Some(s.to_string());
    assert_eq!(parse_cd("cd 'My Files'"), some("My Files"));
    assert_eq!(parse_cd("cd \"a \\\"b\\\" c\""), some("a \"b\" c"));
    assert_eq!(parse_cd("cd My\\ Files"), some("My Files"));
    // Compound lines, expansions and extra words are the shell's to run.
    assert_eq!(parse_cd("cd /tmp && ls"), None);
    assert_eq!(parse_cd("cd /tmp; ls"), None);
    assert_eq!(parse_cd("cd $HOME"), None);
    assert_eq!(parse_cd("cd \"$HOME\""), None);
    assert_eq!(parse_cd("cd /us*"), None);
    assert_eq!(parse_cd("cd ~root"), None);
    assert_eq!(parse_cd("cd a b"), None);
    assert_eq!(parse_cd("cd 'unterminated"), None);
}

#[test]
fn normalize_path_resolves_dotdot() {
    assert_eq!(normalize_path(Path::new("/a/b/../c")), PathBuf::from("/a/c"));
    assert_eq!(normalize_path(Path::new("/a/./b")), PathBuf::from("/a/b"));
}

#[tokio::test]
async fn cd_changes_active_panel_directory() {
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
    // cd to a non-existent directory leaves the panel where it is, and says so.
    st.change_dir("nope").await;
    assert_eq!(st.panels[0].cwd.path, root);
    assert!(st.dialog.is_some(), "a failed cd reports it");
    st.dialog = None;
    // `cd -` returns to the directory before.
    st.change_dir("child").await;
    st.change_dir("-").await;
    assert_eq!(st.panels[0].cwd.path, root);

    // Inside an archive, an absolute path leaves for the local disk and `..`
    // at the archive's root steps out beside the archive.
    let zip = root.join("a.zip");
    st.panels[0].cwd = VfsPath::archive(&zip, "/");
    st.change_dir(&root.join("child").to_string_lossy()).await;
    assert_eq!(st.panels[0].cwd, VfsPath::local(root.join("child")));
    assert_eq!(
        walk_path(VfsPath::archive(&zip, "/"), "../child"),
        VfsPath::local(root.join("child"))
    );
    assert_eq!(walk_path(VfsPath::archive(&zip, "/d/e"), "../f"), VfsPath::archive(&zip, "/d/f"));

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn mouse_clicks_move_cursor_and_mark_in_panel() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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

    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rc_chmodrec_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("top.txt"), b"a").unwrap();
    std::fs::write(root.join("sub/deep.txt"), b"b").unwrap();
    let all = [root.clone(), root.join("top.txt"), root.join("sub"), root.join("sub/deep.txt")];
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
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
    assert!(matches!(st.dialog, Some(Dialog::Message(_))), "no selection → error message");
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
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
    let anchor =
        st.delete_anchor(&[VfsPath::local(root.join("c.txt")), VfsPath::local(root.join("d.txt"))]);
    assert_eq!(anchor.as_deref(), Some("b.txt"));

    std::fs::remove_dir_all(&root).ok();
}

/// Trashing the file under the cursor (F8's default) leaves the cursor on the
/// same row, now showing the entry that followed — in a listing sorted by
/// modification time, where that is not the next name. A second panel showing
/// the same directory with its cursor on the trashed file keeps its row too.
#[tokio::test]
async fn trash_keeps_cursor_row_in_mtime_sorted_panel() {
    use crate::panel::sort::SortKey;
    use crate::ui::dialog::Submit;
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rc_trashcur_{}_{nanos}", std::process::id()));
    let dir = root.join("dir");
    let home = root.join("home");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    // Never touch the real trash; the temp home shares the files' filesystem.
    let _guard = crate::trash::test_home::TempHome::set(&home);

    // Oldest to newest, deliberately out of name order.
    let base = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    for (i, n) in ["c.txt", "a.txt", "d.txt", "b.txt"].iter().enumerate() {
        let path = dir.join(n);
        std::fs::write(&path, b"x").unwrap();
        let stamp = base + std::time::Duration::from_secs(60 * i as u64);
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(stamp).unwrap();
    }

    let (tx, mut rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    for side in 0..2 {
        st.panels[side].cwd = VfsPath::local(&dir);
        st.panels[side].backend = st.registry.local();
        st.panels[side].sort.key = SortKey::ModifyTime;
        st.panels[side].reload().await.unwrap();
    }
    let names = |st: &AppState, side: usize| -> Vec<String> {
        st.panels[side].entries.iter().map(|e| e.name.clone()).collect()
    };
    assert_eq!(names(&st, 0), ["..", "c.txt", "a.txt", "d.txt", "b.txt"], "sorted by mtime");

    for side in 0..2 {
        st.panels[side].cursor = 2; // a.txt
    }
    let targets = st.panels[0].operation_targets();
    st.handle_submit(Submit::Trash(targets)).await;
    drain_taskdone(&mut st, &mut rx).await;

    assert!(!dir.join("a.txt").exists(), "a.txt went to the trash");
    for side in 0..2 {
        assert_eq!(st.panels[side].cursor, 2, "panel {side}: cursor keeps its row");
        assert_eq!(
            st.panels[side].current_entry().map(|e| e.name.as_str()),
            Some("d.txt"),
            "panel {side}: cursor lands on the file that followed",
        );
    }

    // Trashing the last entry falls back to the one above it.
    st.panels[0].cursor = 3; // b.txt
    let targets = st.panels[0].operation_targets();
    st.handle_submit(Submit::Trash(targets)).await;
    drain_taskdone(&mut st, &mut rx).await;
    assert_eq!(st.panels[0].current_entry().map(|e| e.name.as_str()), Some("d.txt"));

    std::fs::remove_dir_all(&root).ok();
}

/// A file operation on the active panel must not disturb an unrelated selection
/// (or the cursor) sitting on the *inactive* panel: only the panel whose files
/// the op consumed has its marks cleared.
#[tokio::test]
async fn delete_on_active_panel_leaves_inactive_selection_and_cursor() {
    use crate::ui::dialog::Submit;
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rc_delkeep_{}_{nanos}", std::process::id()));
    let left = root.join("left");
    let right = root.join("right");
    std::fs::create_dir_all(&left).unwrap();
    std::fs::create_dir_all(&right).unwrap();
    for n in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(left.join(n), b"data").unwrap();
    }
    for n in ["x.txt", "y.txt", "z.txt"] {
        std::fs::write(right.join(n), b"data").unwrap();
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

    // The inactive panel (1) carries an unrelated selection and a cursor parked
    // on a specific entry.
    st.panels[1].selection.mark("y.txt");
    st.panels[1].selection.mark("z.txt");
    let cursor_name = "y.txt";
    st.panels[1].cursor = st.panels[1].entries.iter().position(|e| e.name == cursor_name).unwrap();

    // The active panel (0) marks its own files and deletes them.
    st.panels[0].selection.mark("a.txt");
    st.panels[0].selection.mark("b.txt");
    let targets = st.panels[0].operation_targets();
    assert_eq!(targets.len(), 2, "two files marked on the active panel");
    st.handle_submit(Submit::Delete(targets)).await;
    drain_taskdone(&mut st, &mut rx).await;

    // The deleted files are gone, and the active panel's marks were consumed.
    assert!(!left.join("a.txt").exists() && !left.join("b.txt").exists());
    assert!(st.panels[0].selection.is_empty(), "active panel's selection cleared");

    // The inactive panel is untouched: its selection survives...
    assert!(st.panels[1].selection.is_marked("y.txt"), "inactive selection kept (y)");
    assert!(st.panels[1].selection.is_marked("z.txt"), "inactive selection kept (z)");
    // ...and its cursor still sits on the same entry.
    assert_eq!(
        st.panels[1].current_entry().map(|e| e.name.as_str()),
        Some(cursor_name),
        "inactive panel's cursor left in place",
    );

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn right_drag_inverts_selection_across_files() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
        st.handle_mouse(MouseEvent { kind, column: col, row, modifiers: KeyModifiers::NONE }).await;
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

    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
async fn wheel_pages_then_steps_at_the_listing_ends() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rc_wheel_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for i in 0..100 {
        std::fs::write(root.join(format!("f{i:03}.txt")), b"x").unwrap();
    }
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    // Don't inherit an ambient Tree/Brief config.
    st.panels[0].format = ViewFormat::Full;
    st.panels[1].format = ViewFormat::Full;
    for side in 0..2 {
        st.panels[side].cwd = VfsPath::local(&root);
        st.panels[side].backend = st.registry.local();
        st.panels[side].reload().await.unwrap();
    }

    // Render so both panels record their page size and click geometry.
    let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
    term.draw(|f| crate::ui::draw(f, &mut st)).unwrap();
    let page = st.panels[0].page;
    assert!(page > 1, "page size should reflect the terminal height");
    let len = st.panels[0].entries.len();
    let hit = st.panels[0].hit.expect("hit");
    let wheel = |down: bool| MouseEvent {
        kind: if down { MouseEventKind::ScrollDown } else { MouseEventKind::ScrollUp },
        column: hit.body.x + 1,
        row: hit.body.y,
        modifiers: KeyModifiers::NONE,
    };

    // A notch pages like PgDn...
    st.panels[0].cursor = 0;
    st.handle_mouse(wheel(true)).await;
    assert_eq!(st.panels[0].cursor, page, "wheel down moves a whole page");

    // ...and keeps paging near the end: a page that would run past the listing
    // lands on the last entry rather than degrading into single steps.
    st.panels[0].cursor = len - 2;
    st.handle_mouse(wheel(true)).await;
    assert_eq!(st.panels[0].cursor, len - 1, "a page past the end lands on the last entry");
    st.handle_mouse(wheel(true)).await;
    assert_eq!(st.panels[0].cursor, len - 1, "the wheel stops at the last entry");

    // Same going up: a whole page, clamped onto the first entry at the top.
    st.panels[0].cursor = page;
    st.handle_mouse(wheel(false)).await;
    assert_eq!(st.panels[0].cursor, 0, "wheel up moves a whole page");
    st.panels[0].cursor = page - 1;
    st.handle_mouse(wheel(false)).await;
    assert_eq!(st.panels[0].cursor, 0, "a page past the start lands on the first entry");
    st.handle_mouse(wheel(false)).await;
    assert_eq!(st.panels[0].cursor, 0, "the wheel stops at the first entry");

    // The wheel scrolls the panel under the pointer without moving the focus.
    let other = st.panels[1].hit.expect("hit");
    st.panels[1].cursor = 0;
    st.handle_mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: other.body.x + 1,
        row: other.body.y,
        modifiers: KeyModifiers::NONE,
    })
    .await;
    assert_eq!(st.panels[1].cursor, page, "the hovered panel scrolls");
    assert_eq!(st.active, 0, "scrolling doesn't change which panel is active");

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

/// The thumbnail grid loads pictures for what is on screen, in the background,
/// and its arrows move by whole rows and single cells.
#[tokio::test]
async fn the_thumbnail_grid_loads_its_pictures_and_moves_by_row_and_cell() {
    use crate::panel::ViewFormat;
    let root = temp_dir("thumb_grid");
    for i in 0..6 {
        image::RgbaImage::from_pixel(64, 48, image::Rgba([0, 90, 200, 255]))
            .save(root.join(format!("p{i}.png")))
            .unwrap();
    }
    let (tx, mut rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.config.command_prompt = false;
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    st.set_format(0, ViewFormat::Thumbs).await;
    st.panels[0].page = 6;
    st.panels[0].cols = 3;
    st.update_thumbs();

    let mut ready = 0;
    while ready < 6 {
        let ev = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
        if let AppEvent::Thumbnail { thumb, .. } = &ev {
            assert!(thumb.is_some(), "every PNG decodes");
            ready += 1;
        }
        st.apply_event(ev).await;
    }
    let entries = st.panels[0].entries.clone();
    let cache = st.panels[0].thumbs.as_mut().unwrap();
    let bg = crate::ui::graphics::raster::rgb(st.theme.panel_bg);
    let e = entries.iter().find(|e| e.name == "p0.png").unwrap();
    let key = crate::thumbs::ThumbKey::new(&st.panels[0].cwd, e, cache.size, bg);
    assert!(matches!(cache.get(&key), Some(crate::thumbs::ThumbState::Ready(_))));

    st.panels[0].cursor = 1; // p0.png, after `..`
    st.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)).await;
    assert_eq!(st.panels[0].cursor, 4, "Down moves a whole row of three");
    st.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)).await;
    assert_eq!(st.panels[0].cursor, 5, "Right moves one cell");
    st.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)).await;
    assert_eq!(st.panels[0].cursor, 2);

    st.set_format(0, ViewFormat::Full).await;
    st.update_thumbs();
    assert!(st.panels[0].thumbs.is_none(), "leaving the grid frees its pictures");
    let _ = std::fs::remove_dir_all(&root);
}

/// An Activity log asks for a recursive watch on the other panel's tree — even
/// with auto-refresh off, which then still does not reload any listing — and
/// Enter on a row points the other panel at the file.
#[tokio::test]
async fn an_activity_log_watches_the_other_tree_and_enter_shows_the_file() {
    use crate::panel::ViewFormat;
    let root = temp_dir("activity_log");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.config.auto_refresh = false;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    st.active = 1;
    st.set_format(1, ViewFormat::Activity).await;
    st.update_activity_logs();
    st.update_watches();
    assert_eq!(st.watch_armed.get(&root), Some(&true), "the tree is watched recursively");
    assert_eq!(st.panels[1].activity.as_ref().unwrap().root.as_deref(), Some(root.as_path()));

    st.note_fs_event(&FsEvent::new(FsKind::Modify, root.join("src/main.rs")));
    assert!(!st.watch_pending(), "auto-refresh is off: no reload is stamped");
    assert_eq!(st.panels[1].activity.as_ref().unwrap().visible_len(), 1);

    st.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
    assert_eq!(st.active, 0, "the other panel takes over");
    assert_eq!(st.panels[0].cwd.path, root.join("src"));
    assert_eq!(st.panels[0].current_entry().map(|e| e.name.as_str()), Some("main.rs"));
    // The log follows the other panel into the directory it now shows.
    st.update_activity_logs();
    assert_eq!(
        st.panels[1].activity.as_ref().unwrap().root.as_deref(),
        Some(root.join("src").as_path())
    );

    // Alt-T leaves the log for the listing cycle, and never lands on it.
    st.active = 1;
    for _ in 0..8 {
        st.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::ALT)).await;
        assert_ne!(st.panels[1].format, ViewFormat::Activity);
    }
    st.update_activity_logs();
    assert!(st.panels[1].activity.is_none(), "and the log is dropped with it");
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn a_watched_directory_change_reloads_the_panel_after_the_debounce() {
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rc_watch_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.txt"), b"x").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    st.update_watches();
    assert_eq!(st.watch_key[0], root.to_string_lossy(), "the local panel is watched");
    // The events below are fed in by hand; the real watcher's own reports of
    // the same write would race them (the live tests in `watch` cover it).
    st.watcher = None;

    // A file appears behind our back, and the watcher reports it.
    std::fs::write(root.join("b.txt"), b"y").unwrap();
    st.note_fs_event(&FsEvent::new(FsKind::Create, root.join("b.txt")));
    assert!(st.watch_pending(), "the change is stamped, not acted on yet");
    assert!(st.wants_ticks(), "and the loop keeps ticking so it can fire");
    assert!(!st.panels[0].entries.iter().any(|e| e.name == "b.txt"), "not reloaded yet");

    // Before the debounce expires nothing happens; after it, the panel re-reads.
    st.flush_dir_changes().await;
    assert!(st.watch_pending(), "still within the quiet window");
    st.watch_dirty[0] = Some(Instant::now() - crate::app::state::watch::DEBOUNCE);
    st.flush_dir_changes().await;
    assert!(!st.watch_pending(), "the pending reload was consumed");
    assert!(st.panels[0].entries.iter().any(|e| e.name == "b.txt"), "listing picked it up");

    // A panelized listing is left alone: reloading would discard the results.
    st.panels[0].set_results(Vec::new(), Vec::new());
    st.update_watches();
    assert_eq!(st.watch_key[0], "", "a panelized panel is not watched");
    st.note_fs_event(&FsEvent::new(FsKind::Modify, root.join("b.txt")));
    assert!(!st.watch_pending(), "and an event for it is ignored");

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn clipboard_text_covers_name_path_and_selection() {
    use crate::ui::menu::ClipTarget;
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rc_clip_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for n in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(root.join(n), b"x").unwrap();
    }

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    // Park the cursor on a.txt (the listing is sorted, `..` is absent at a root
    // only, so find it by name rather than assuming an index).
    st.panels[0].cursor =
        st.panels[0].entries.iter().position(|e| e.name == "a.txt").expect("a.txt listed");

    // With nothing marked, both single-item forms describe the cursor entry.
    assert_eq!(st.clipboard_text(ClipTarget::Name), "a.txt");
    assert_eq!(st.clipboard_text(ClipTarget::FullPath), root.join("a.txt").display().to_string());
    assert_eq!(
        st.clipboard_text(ClipTarget::Selection),
        root.join("a.txt").display().to_string(),
        "the cursor entry stands in for an empty selection"
    );

    // Marking makes Selection a newline-separated list, in listing order.
    st.panels[0].selection.mark("a.txt");
    st.panels[0].selection.mark("c.txt");
    let lines: Vec<String> =
        st.clipboard_text(ClipTarget::Selection).lines().map(str::to_string).collect();
    assert_eq!(lines.len(), 2, "two marked files");
    assert!(lines[0].ends_with("a.txt") && lines[1].ends_with("c.txt"), "got {lines:?}");

    std::fs::remove_dir_all(&root).ok();
}

/// Find file end to end at the app level: the dialog's params go in, the search
/// runs on its background task, and the results come back panelized with the
/// content-hit lines recorded so F3 can open the viewer on the match.
#[tokio::test]
async fn find_file_content_search_panelizes_hits_and_records_their_lines() {
    use crate::ui::dialog::Submit;
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rc_finde2e_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("one.txt"), b"alpha\nbeta needle here\ngamma\n").unwrap();
    std::fs::write(root.join("two.txt"), b"nothing to see\n").unwrap();
    std::fs::write(root.join("sub/three.txt"), b"deep needle\n").unwrap();
    // A binary file that *does* contain the needle: it must be skipped.
    let mut blob = vec![0u8; 32];
    blob.extend_from_slice(b"needle");
    std::fs::write(root.join("blob.bin"), &blob).unwrap();

    let (tx, mut rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();

    st.handle_submit(Submit::Find(FindParams {
        start_at: root.to_string_lossy().into_owned(),
        file_name: "*".into(),
        content: "needle".into(),
        recursive: true,
        case_sensitive: false,
        skip_hidden: true,
        shell: true,
        regex_content: false,
    }))
    .await;
    // Drive the background search to completion.
    loop {
        let ev = rx.recv().await.unwrap();
        let done = matches!(ev, AppEvent::FindDone { .. });
        st.apply_event(ev).await;
        if done {
            break;
        }
    }

    assert!(st.panels[0].is_panelized(), "results replaced the listing");
    let names: Vec<String> = st.panels[0].entries.iter().map(|e| e.name.clone()).collect();
    let has = |n: &str| names.iter().any(|x| x.ends_with(n));
    assert!(has("one.txt"), "matched on line 2: {names:?}");
    assert!(has("three.txt"), "matched in a subdirectory: {names:?}");
    assert!(!has("two.txt"), "does not contain the needle");
    assert!(!has("blob.bin"), "binary files are skipped even when they match");

    // The hit lines came back with the results, keyed for the viewer jump.
    let one = VfsPath::local(root.join("one.txt")).display();
    assert_eq!(st.find_hit_lines.get(&one), Some(&2), "'needle' is on line 2 of one.txt");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn grep_file_streams_and_reports_line_numbers() {
    use crate::viewer::search::Needle;
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rc_grep_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let lit = |s: &str| Needle::build(s, false, true, false, false).unwrap();

    // A hit on the first line reports 1; later lines count the newlines before.
    let p = root.join("lines.txt");
    std::fs::write(&p, b"alpha\nbeta\ngamma\n").unwrap();
    assert_eq!(grep_file(&p, &lit("alpha")), Some(1));
    assert_eq!(grep_file(&p, &lit("gamma")), Some(3));
    assert_eq!(grep_file(&p, &lit("absent")), None);

    // Case sensitivity follows the needle, not the file.
    assert_eq!(grep_file(&p, &lit("ALPHA")), None);
    assert_eq!(
        grep_file(&p, &Needle::build("ALPHA", false, false, false, false).unwrap()),
        Some(1)
    );

    // A NUL byte marks the file binary, even though the needle is present.
    let bin = root.join("blob.bin");
    std::fs::write(&bin, b"\x00\x01alpha").unwrap();
    assert_eq!(grep_file(&bin, &lit("alpha")), None, "binary files are skipped");

    // The whole point of streaming: a match straddling a window seam is still
    // found, and its line number counts every newline before it.
    let seam = root.join("seam.txt");
    let mut data = vec![b'x'; GREP_WINDOW - 3];
    data.extend_from_slice(b"\nNEEDLE\n");
    std::fs::write(&seam, &data).unwrap();
    assert_eq!(grep_file(&seam, &lit("NEEDLE")), Some(2), "found across the seam");

    // A hit well past the first window reports a file-wide line number.
    let big = root.join("big.txt");
    let mut data = b"filler\n".repeat(GREP_WINDOW / 7 + 10);
    let lines_before = data.iter().filter(|b| **b == b'\n').count() as u64;
    data.extend_from_slice(b"TARGET\n");
    std::fs::write(&big, &data).unwrap();
    assert_eq!(grep_file(&big, &lit("TARGET")), Some(lines_before + 1));

    // A regex needle works, and takes the RE_OVERLAP window path.
    assert_eq!(grep_file(&p, &Needle::build(r"g\w+a", true, true, false, false).unwrap()), Some(3));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn find_files_by_name_and_content() {
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rc_find_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("a.txt"), b"hello there").unwrap();
    std::fs::write(root.join("sub/b.txt"), b"world").unwrap();
    std::fs::write(root.join("c.log"), b"hello again").unwrap();

    let run = |p: &FindParams| {
        let m =
            crate::panel::selection::NameMatcher::build(&p.file_name, p.case_sensitive, p.shell)
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
        regex_content: false,
    };
    assert_eq!(run(&by_name).len(), 2, "two .txt files");
    assert!(run(&by_name).iter().all(|(_, l)| l.is_none()), "a name match has no hit line");

    let by_content =
        FindParams { file_name: "*".into(), content: "HELLO".into(), ..by_name.clone() };
    assert_eq!(run(&by_content).len(), 2, "two files contain 'hello'");
    assert!(run(&by_content).iter().all(|(_, l)| *l == Some(1)), "both match on line 1");

    // The content field is a regular expression when asked, and the reported
    // line is the one the match is actually on.
    std::fs::write(root.join("d.txt"), b"one\ntwo\nthr33\n").unwrap();
    let by_regex = FindParams {
        file_name: "d.txt".into(),
        content: r"thr\d+".into(),
        regex_content: true,
        ..by_name.clone()
    };
    let hits = run(&by_regex);
    assert_eq!(hits.len(), 1, "the regex matches only d.txt");
    assert_eq!(hits[0].1, Some(3), "'thr33' is on line 3");

    // The same pattern read literally matches nothing.
    let literal = FindParams { regex_content: false, ..by_regex };
    assert!(run(&literal).is_empty(), "literal mode does not treat \\d as a class");

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn find_files_vfs_matches_names_recursively() {
    use std::sync::Arc;
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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

    let mut names: Vec<String> = results.iter().map(|h| h.path.file_name()).collect();
    names.sort();
    assert_eq!(names, vec!["a.txt", "b.txt"], "name-only, recursive, .log excluded");
    // Sizes come from the directory listing, not a second stat.
    assert!(results.iter().any(|h| h.path.file_name() == "b.txt" && h.size == 2));
    // A remote/archive walk is name-only, so it never carries a content line.
    assert!(results.iter().all(|h| h.line.is_none()), "the VFS walker stays name-only");

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn theme_preview_applies_and_reverts_on_cancel() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    let original = st.theme.name.clone();
    st.open_settings();
    let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
    // Theme is the first field of the first tab, so it has the focus; Enter
    // opens its dropdown and moving the highlight previews the theme live.
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
async fn nerd_font_symbols_preview_live_and_persist_on_ok() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.config.nerd_font = false;
    st.open_settings();
    let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
    // "Nerd Font symbols" is the Appearance tab's third field; Tab down to it
    // and toggle it with Space.
    for _ in 0..2 {
        st.handle_key(key(KeyCode::Tab)).await;
    }
    st.handle_key(key(KeyCode::Char(' '))).await;
    assert!(st.nerd_font_active(), "the listing previews the glyphs while the box is ticked");
    assert!(!st.config.nerd_font, "nothing is stored until the form is submitted");

    // Esc closes the dialog without applying it…
    st.handle_key(key(KeyCode::Esc)).await;
    assert!(!st.nerd_font_active(), "cancel reverts to the plain `ls -F` markers");

    // …and submitting saves it.
    st.open_settings();
    for _ in 0..2 {
        st.handle_key(key(KeyCode::Tab)).await;
    }
    st.handle_key(key(KeyCode::Char(' '))).await;
    st.handle_key(key(KeyCode::Enter)).await;
    assert!(st.config.nerd_font, "OK stores the setting");
    assert!(st.nerd_font_active(), "and the listing keeps drawing the glyphs");
}

#[tokio::test]
async fn the_confirmations_command_opens_settings_on_its_tab() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
    assert!(st.config.confirm_delete);
    st.run_menu_action(MenuAction::Confirmations).await;
    let tab = |st: &AppState| match &st.dialog {
        Some(Dialog::Form(fd)) => fd.settings_tab(),
        _ => None,
    };
    assert_eq!(tab(&st), Some(SettingsTab::Confirmations), "the command opens its tab");
    // The tab opens on Confirm delete: Space unticks it, Enter saves.
    st.handle_key(key(KeyCode::Char(' '))).await;
    st.handle_key(key(KeyCode::Enter)).await;
    assert!(st.dialog.is_none());
    assert!(!st.config.confirm_delete, "the Confirmations tab saves through Settings");
    // Plain Options → Settings… now comes back to the tab it was closed on.
    st.open_settings();
    assert_eq!(tab(&st), Some(SettingsTab::Confirmations), "the last tab is remembered");
    // Switch tabs and cancel: that is remembered too.
    let ctrl_pgdn = KeyEvent::new(KeyCode::PageDown, KeyModifiers::CONTROL);
    st.handle_key(ctrl_pgdn).await;
    st.handle_key(key(KeyCode::Esc)).await;
    st.open_settings();
    assert_eq!(tab(&st), Some(SettingsTab::Language));
}

#[tokio::test]
async fn lowering_the_history_size_in_settings_trims_the_history() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
    st.cmd.history = ["a", "b", "c", "d", "e"].map(String::from).to_vec();
    st.open_settings_at(SettingsTab::Programs);
    // The history size is the Programs tab's seventh and last field.
    for _ in 0..6 {
        st.handle_key(key(KeyCode::Tab)).await;
    }
    for _ in 0..3 {
        st.handle_key(key(KeyCode::Backspace)).await;
    }
    st.handle_key(key(KeyCode::Char('2'))).await;
    st.handle_key(key(KeyCode::Enter)).await;
    assert_eq!(st.config.command_history_max, 2);
    assert_eq!(st.cmd.history_max, 2);
    assert_eq!(
        st.cmd.history,
        ["d", "e"],
        "the oldest entries go at once, not on the next command"
    );
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
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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

/// Shift-F4 asks for a name and opens the editor on it in the active panel's
/// directory: the buffer starts empty and the first save creates the file. A
/// name that is already taken opens that file instead of shadowing it.
#[tokio::test]
async fn shift_f4_edits_a_new_file_in_the_panel_directory() {
    let dir = crate::util::temp::rc_temp_path("test-newfile");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("taken.txt"), b"already here").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.panels[0].cwd = VfsPath::local(&dir);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    st.active = 0;

    // The key opens the name prompt (plain F4 still edits the cursor file).
    st.handle_key(KeyEvent::new(KeyCode::F(4), KeyModifiers::SHIFT)).await;
    match &st.dialog {
        Some(Dialog::Input(d)) => assert!(matches!(d.purpose, InputPurpose::EditNewFile)),
        _ => panic!("Shift-F4 opens the file-name prompt"),
    }
    assert!(st.editor.is_none(), "nothing is opened until a name is entered");

    let flow = st
        .handle_dialog_result(DialogResult::Submit(Submit::EditNewFile("fresh.txt".into())))
        .await;
    assert!(matches!(flow, Flow::Continue));
    assert!(st.dialog.is_none(), "no error dialog");
    let ed = st.editor.as_ref().expect("the editor opens on the new name");
    assert_eq!(ed.name, "fresh.txt");
    assert_eq!(ed.path, VfsPath::local(dir.join("fresh.txt")), "aimed at the panel's directory");
    assert!(ed.contents().is_empty(), "a new file starts empty");
    assert!(!dir.join("fresh.txt").exists(), "the file itself waits for the first save");

    st.save_editor(false).await;
    assert!(dir.join("fresh.txt").exists(), "saving creates it");

    // An existing name opens that file rather than an empty buffer over it.
    st.editor = None;
    st.open_new_file_editor("taken.txt".into()).await;
    let ed = st.editor.as_ref().expect("an existing name still opens the editor");
    assert!(ed.contents().starts_with("already here"), "loaded: {:?}", ed.contents());

    std::fs::remove_dir_all(&dir).ok();
}

/// A directory name is refused (there is nothing to edit), and a name with a
/// subdirectory in it lands in that subdirectory.
#[tokio::test]
async fn shift_f4_refuses_a_directory_and_honours_a_subpath() {
    let dir = crate::util::temp::rc_temp_path("test-newfile-dir");
    std::fs::create_dir_all(dir.join("sub")).unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.panels[0].cwd = VfsPath::local(&dir);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    st.active = 0;

    st.open_new_file_editor("sub".into()).await;
    assert!(st.editor.is_none(), "a directory is not opened in the editor");
    assert!(matches!(&st.dialog, Some(Dialog::Message(m)) if m.is_error), "it says why");
    st.dialog = None;

    st.open_new_file_editor("sub/notes.txt".into()).await;
    let ed = st.editor.as_ref().expect("a sub-path opens too");
    assert_eq!(ed.name, "notes.txt", "named by the last component");
    assert_eq!(ed.path, VfsPath::local(dir.join("sub/notes.txt")));
    st.save_editor(false).await;
    assert!(dir.join("sub/notes.txt").exists(), "saved into the subdirectory");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn editor_save_as_writes_and_retargets() {
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
    let target =
        crate::flash::FlashTarget { dev: "/dev/sdb".into(), size: 1000, ..Default::default() };

    // "Create image" opens the save browser.
    st.handle_submit(Submit::ImageBrowse(target.clone())).await;
    assert!(matches!(st.dialog, Some(Dialog::ImageSave(_))), "image save browser opens");

    // Saving onto an existing file raises an overwrite confirmation.
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
    st.panels[0].entries =
        vec![mk_entry("yo"), mk_entry("hello"), mk_entry("hi"), mk_entry("high")];
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
    // Alt + a menu letter opens that top menu now, even with quick search
    // enabled — Alt no longer starts a search on its own. ('o' is missing here:
    // plain Alt-O shows the cursor's directory on the other panel, so the Options
    // menu keeps the shifted letter — asserted below.)
    for c in ['f', 'c', 'l', 'r'] {
        st.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)).await;
        assert!(st.menu.is_some(), "Alt+{c} opens a menu");
        assert!(st.quick_search.is_none(), "Alt+{c} does not start a search");
        st.menu = None;
    }
    // Plain Alt-O is the panel action, not the Options menu.
    st.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::ALT)).await;
    assert!(st.menu.is_none(), "Alt-O no longer opens the Options menu");
    // Alt-Shift-O still reaches it (the menu lookup lower-cases the letter).
    st.handle_key(KeyEvent::new(KeyCode::Char('O'), KeyModifiers::ALT | KeyModifiers::SHIFT)).await;
    assert!(st.menu.is_some(), "Alt-Shift-O opens the Options menu");
    st.menu = None;
    // Alt-S starts a search, not a menu ('s' isn't a menu letter anyway).
    st.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT)).await;
    assert!(st.menu.is_none());
    assert!(st.quick_search.is_some(), "Alt-S starts a quick search");
}

/// A panel state with the four sortable names used by the quick-search tests,
/// on the *active* side (which `init` may have restored to either panel).
fn seed_quick_search_panel(st: &mut AppState) {
    let side = st.active;
    st.panels[side].entries =
        vec![mk_entry("yo"), mk_entry("hello"), mk_entry("hi"), mk_entry("high")];
    st.panels[side].resort(); // stable: hello, hi, high, yo
    st.panels[side].cursor = 3; // start off the 'h' entries (on "yo")
}

#[tokio::test]
async fn ctrl_f5_toggles_the_command_prompt_and_clears_the_line() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.init().await;
    seed_quick_search_panel(&mut st); // a real entry under the cursor, for F5
    st.config.command_prompt = true;
    st.cmd.set("half typed".to_string());

    st.handle_key(KeyEvent::new(KeyCode::F(5), KeyModifiers::CONTROL)).await;
    assert!(!st.config.command_prompt, "Ctrl-F5 hides the command prompt");
    assert!(st.cmd.is_empty(), "hiding it drops any half-typed line");

    st.handle_key(KeyEvent::new(KeyCode::F(5), KeyModifiers::CONTROL)).await;
    assert!(st.config.command_prompt, "Ctrl-F5 brings it back");

    // Plain F5 is still Copy, not the toggle.
    st.handle_key(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE)).await;
    assert!(st.config.command_prompt, "plain F5 doesn't touch the prompt");
    assert!(st.dialog.is_some(), "plain F5 still opens the copy dialog");
}

#[tokio::test]
async fn typing_starts_a_quick_search_when_the_command_prompt_is_hidden() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.init().await;
    seed_quick_search_panel(&mut st);
    let side = st.active;

    // With the prompt on, a letter is text — no search starts.
    st.config.command_prompt = true;
    st.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE)).await;
    assert!(st.quick_search.is_none(), "the command line takes the character");
    assert_eq!(st.cmd.buffer, "h");
    st.cmd.clear();

    // With it hidden, the same letter seeds a search and jumps the cursor.
    st.config.command_prompt = false;
    st.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE)).await;
    assert_eq!(st.quick_search.as_ref().unwrap().query, "h");
    assert_eq!(st.panels[side].entries[st.panels[side].cursor].name, "hello");
    assert!(st.cmd.is_empty(), "nothing reaches the hidden command line");

    // Further characters extend it through the normal quick-search handler.
    st.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)).await;
    assert_eq!(st.quick_search.as_ref().unwrap().query, "hi");
    assert_eq!(st.panels[side].entries[st.panels[side].cursor].name, "hi");

    // Non-alphanumeric characters search too (Midnight Commander's rule), so
    // dotfiles and names starting with "_" are reachable.
    st.handle_key(esc_key()).await;
    st.panels[side].entries.push(mk_entry(".config"));
    st.panels[side].resort();
    st.handle_key(KeyEvent::new(KeyCode::Char('.'), KeyModifiers::NONE)).await;
    assert_eq!(st.quick_search.as_ref().unwrap().query, ".");
    assert_eq!(st.panels[side].entries[st.panels[side].cursor].name, ".config");
}

#[tokio::test]
async fn hidden_command_prompt_keeps_the_selection_and_enter_keys() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.init().await;
    seed_quick_search_panel(&mut st);
    st.config.command_prompt = false;

    // '+' / '-' / '*' stay selection keys rather than seeding a search.
    for c in ['+', '-'] {
        st.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)).await;
        assert!(st.quick_search.is_none(), "'{c}' is not a search character");
        assert!(st.dialog.is_some(), "'{c}' opens the select-group dialog");
        st.dialog = None;
    }
    st.handle_key(KeyEvent::new(KeyCode::Char('*'), KeyModifiers::NONE)).await;
    assert!(st.quick_search.is_none(), "'*' inverts the selection instead");

    // The command-line-only keys are inert, and must not fall through to
    // something else (Alt-Enter especially must not descend).
    let before = st.panels[st.active].cwd.clone();
    for key in [
        KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT),
        KeyEvent::new(KeyCode::Char('p'), KeyModifiers::ALT),
        KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT),
        KeyEvent::new(KeyCode::Char('H'), KeyModifiers::ALT | KeyModifiers::SHIFT),
    ] {
        st.handle_key(key).await;
        assert!(st.cmd.is_empty(), "{key:?} leaves the hidden command line alone");
        assert!(st.dialog.is_none(), "{key:?} opens nothing");
        assert_eq!(st.panels[st.active].cwd, before, "{key:?} doesn't navigate");
    }
}

#[tokio::test]
async fn quick_search_extends_while_alt_is_held() {
    // Once the box is open, holding Alt across letters (Alt-H, Alt-I, Alt-G)
    // must extend the query, not re-trigger anything.
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.init().await;
    st.panels[0].entries =
        vec![mk_entry("yo"), mk_entry("hello"), mk_entry("hi"), mk_entry("high")];
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
    st.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE)).await;
    assert!(st.viewer.is_some(), "Esc-1 should act as F1 (help)");
    assert!(st.pending_esc.is_none(), "the sequence is resolved");
}

#[tokio::test]
async fn alt_digit_acts_as_function_key() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    assert!(st.viewer.is_none());
    // Terminals deliver a fast Esc+digit as Alt+digit; that is an F-key too.
    st.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT)).await;
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
    st.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)).await;
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
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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

    st.handle_submit(Submit::Copy(
        vec![VfsPath::local(left.join("big.bin"))],
        right.to_string_lossy().into_owned(),
    ))
    .await;
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
        BgTransfer {
            verb: "Copying",
            update: Some(progress_update(7, "Copying", 10, 100)),
            schemes: vec![],
            chart: Default::default(),
        },
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
    st.tasks
        .insert(5, crate::ops::TaskHandle { id: 5, cancel: crate::ops::CancelToken::new(), reply });
    st.task_progress.insert(
        5,
        BgTransfer {
            verb: "Moving",
            update: Some(progress_update(5, "Moving", 40, 80)),
            schemes: vec![],
            chart: Default::default(),
        },
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
    st.task_progress.insert(
        1,
        BgTransfer {
            verb: "Copying",
            update: Some(progress_update(1, "Copying", 30, 100)),
            schemes: vec![],
            chart: Default::default(),
        },
    );
    st.task_progress.insert(
        2,
        BgTransfer {
            verb: "Moving",
            update: Some(progress_update(2, "Moving", 20, 100)),
            schemes: vec![],
            chart: Default::default(),
        },
    );
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
    st.tasks
        .insert(1, crate::ops::TaskHandle { id: 1, cancel: crate::ops::CancelToken::new(), reply });
    st.task_progress.insert(
        1,
        BgTransfer {
            verb: "Copying",
            update: Some(progress_update(1, "Copying", 10, 100)),
            schemes: vec![],
            chart: Default::default(),
        },
    );

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
    st.apply_event(AppEvent::TaskDone { id: 1, outcome: crate::ops::progress::TaskOutcome::Done })
        .await;
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
    let flow =
        st.handle_dialog_result(DialogResult::Submit(Submit::RunProgram(path.clone()))).await;
    match flow {
        Flow::RunCommand(cmd) => assert!(cmd.contains("tool"), "runs the program: {cmd}"),
        _ => panic!("RunProgram should run the executable in the foreground"),
    }
    assert!(st.dialog.is_none(), "the confirm dialog is dismissed");
}

/// An F2 user-menu command on a local panel is routed to the foreground
/// suspend-and-run path (so its output is visible), not the background console.
#[tokio::test]
async fn user_menu_command_runs_in_the_foreground_on_a_local_panel() {
    use crate::ui::dialog::{DialogResult, Submit};
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    let flow = st
        .handle_dialog_result(DialogResult::Submit(Submit::UserCommand("echo hi in %d".into())))
        .await;
    match flow {
        Flow::RunCommandForeground(cmd) => {
            assert!(
                cmd.contains("echo hi in ") && !cmd.contains("%d"),
                "expanded + foreground: {cmd}"
            );
        }
        _ => panic!("a local F2 menu command should run in the foreground"),
    }
    assert!(st.dialog.is_none(), "the menu dialog is dismissed");
}

#[test]
fn menu_prompts_extracts_labels_in_order() {
    use super::keys::menu_prompts;
    assert!(menu_prompts("echo %f").is_empty());
    assert_eq!(menu_prompts("CMD=%{Enter command}"), vec!["Enter command".to_string()]);
    assert_eq!(
        menu_prompts("%{First} then %{Second}"),
        vec!["First".to_string(), "Second".to_string()]
    );
    // `%%` escapes a literal percent, so `%%{x}` is not a prompt.
    assert_eq!(menu_prompts("100%%{x} %{Real}"), vec!["Real".to_string()]);
}

/// A `%{…}` prompt opens an input dialog; answering it substitutes the typed
/// text verbatim and runs the command (foreground, on a local panel).
#[tokio::test]
async fn user_menu_prompt_asks_then_runs_with_the_answer() {
    use crate::ui::dialog::{Dialog, DialogResult, Submit};
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    let flow = st
        .handle_dialog_result(DialogResult::Submit(Submit::UserCommand(
            "CMD=%{Enter command}\necho $CMD".into(),
        )))
        .await;
    assert!(matches!(flow, Flow::Continue), "the command waits for its prompt");
    match &st.dialog {
        Some(Dialog::Input(d)) => assert_eq!(d.prompt, "Enter command", "prompt label shown"),
        _ => panic!("a %{{…}} prompt should open an input dialog"),
    }
    let flow =
        st.handle_dialog_result(DialogResult::Submit(Submit::MenuPrompt("ls -la".into()))).await;
    match flow {
        Flow::RunCommandForeground(cmd) => {
            assert!(cmd.contains("CMD=ls -la"), "answer substituted verbatim: {cmd}");
            assert!(!cmd.contains("%{"), "no prompt macro remains: {cmd}");
        }
        _ => panic!("the completed command should run in the foreground"),
    }
    assert!(st.dialog.is_none() && st.pending_menu.is_none(), "state is cleaned up");
}

/// Multiple `%{…}` prompts are asked one dialog at a time, then all substituted.
#[tokio::test]
async fn user_menu_multiple_prompts_are_asked_in_sequence() {
    use crate::ui::dialog::{Dialog, DialogResult, Submit};
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    let flow = st
        .handle_dialog_result(DialogResult::Submit(Submit::UserCommand(
            "echo %{One} %{Two}".into(),
        )))
        .await;
    assert!(matches!(flow, Flow::Continue));
    match &st.dialog {
        Some(Dialog::Input(d)) => assert_eq!(d.prompt, "One"),
        _ => panic!("first prompt"),
    }
    let flow = st.handle_dialog_result(DialogResult::Submit(Submit::MenuPrompt("a".into()))).await;
    assert!(matches!(flow, Flow::Continue), "still asking the second prompt");
    match &st.dialog {
        Some(Dialog::Input(d)) => assert_eq!(d.prompt, "Two"),
        _ => panic!("second prompt"),
    }
    let flow = st.handle_dialog_result(DialogResult::Submit(Submit::MenuPrompt("b".into()))).await;
    match flow {
        Flow::RunCommandForeground(cmd) => assert_eq!(cmd, "echo a b"),
        _ => panic!("runs once both answers are in"),
    }
}

/// Cancelling a `%{…}` prompt abandons the whole command.
#[tokio::test]
async fn cancelling_a_menu_prompt_abandons_the_command() {
    use crate::ui::dialog::{DialogResult, Submit};
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.handle_dialog_result(DialogResult::Submit(Submit::UserCommand("echo %{Ask}".into()))).await;
    assert!(st.pending_menu.is_some(), "a prompt is pending");
    let flow = st.handle_dialog_result(DialogResult::Cancel).await;
    assert!(matches!(flow, Flow::Continue));
    assert!(st.pending_menu.is_none(), "cancel abandons the pending command");
    assert!(st.pending_run.is_none() && st.pending_run_fg.is_none(), "nothing is queued to run");
}

/// mc `+ <cond>` filters which entries show and `= <cond>` picks the default,
/// evaluated against the panel at F2-press time.
#[tokio::test]
async fn user_menu_conditions_filter_and_default() {
    use crate::ui::dialog::Dialog;
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.user_menu = crate::usermenu::parse(
        "+ ! t t\na Current\n\techo cur\n+ t t\nb Tagged\n\techo tag\nc Always\n\techo always\n= t t\nd Default\n\techo d\n",
    );
    // Nothing tagged: the `+ ! t t` entry shows, the `+ t t` one is hidden, and
    // no `=` condition matches so the default is the first entry.
    st.open_user_menu();
    match &st.dialog {
        Some(Dialog::UserMenu(d)) => {
            assert_eq!(d.hotkeys(), vec!['a', 'c', 'd']);
            assert_eq!(d.selected_hotkey(), Some('a'));
        }
        _ => panic!("menu should open"),
    }
    // With a tagged file the sets flip and `= t t` selects the default.
    st.panels[st.active].selection.mark("x");
    st.open_user_menu();
    match &st.dialog {
        Some(Dialog::UserMenu(d)) => {
            assert_eq!(d.hotkeys(), vec!['b', 'c', 'd']);
            assert_eq!(d.selected_hotkey(), Some('d'));
        }
        _ => panic!("menu should open"),
    }
}

/// Value macros shell-quote by default (matching mc); `%0f` opts out and
/// `%view{…}` is stripped.
#[tokio::test]
async fn user_menu_macros_quote_and_strip_view() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.panels[st.active].entries = vec![mk_entry("my file.txt")];
    st.panels[st.active].cursor = 0;
    // %f is quoted by default, so the space is protected.
    assert!(!st.expand_macros("cat %f").contains("cat my file.txt"), "%f quoted");
    // %0f opts out of quoting.
    assert!(st.expand_macros("cat %0f").contains("cat my file.txt"), "%0f raw");
    // %view{…} is stripped; the rest of the command survives.
    let v = st.expand_macros("%view{ascii,nroff} roff %f");
    assert!(!v.contains("%view") && !v.contains("{ascii"), "view stripped: {v}");
    assert!(v.trim_start().starts_with("roff "), "command runs: {v}");
}

/// Pressing Enter on an executable file with no MIME handler runs it directly
/// (ELF binaries, scripts) rather than trying to open it with an application.
#[cfg(all(unix, target_os = "linux"))]
#[tokio::test]
async fn enter_on_executable_binary_runs_it() {
    use std::os::unix::fs::PermissionsExt;
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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

    // Alt-Shift-H opens the Shell History window (plain Alt-H now lists the
    // *directory* history — the two windows share the letter).
    st.handle_key(alt_c('H')).await;
    assert!(
        matches!(st.dialog, Some(Dialog::ShellHistory(_))),
        "Alt-Shift-H opens the shell history window"
    );
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
    // (C-W is no longer among them: the listing toggle it used to share moved to
    // Alt-T, so C-W is now unconditionally readline's kill-word-back.)
    let fmt = st.panels[st.active].format;
    st.handle_key(alt('t')).await;
    assert_ne!(st.panels[st.active].format, fmt, "Alt-T cycles the view");
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
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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

    // -- Alt-Shift-I opens the filter prompt (plain Alt-I syncs the panels) --
    st.handle_key(alt('I')).await;
    assert!(matches!(st.dialog, Some(Dialog::Input(_))), "Alt-Shift-I opens the filter input");
    st.dialog = None;

    let _ = std::fs::remove_dir_all(&root);
}

/// Drive the background preview loader until the Details view for `viewer` has a
/// loaded (non-Loading) preview, applying every event that arrives.
async fn drain_until_preview(st: &mut AppState, rx: &mut crate::util::async_bridge::AppReceiver) {
    for _ in 0..100 {
        match tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await {
            Ok(Some(ev)) => {
                st.apply_event(ev).await;
                if !matches!(
                    st.details[1].preview,
                    crate::details::Preview::Loading | crate::details::Preview::None
                ) {
                    return;
                }
            }
            _ => return,
        }
    }
}

#[tokio::test]
async fn details_preview_loads_text_head_and_dir_tree() {
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rc_prev_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("sub/inner.rs"), b"pub fn inner() {}\n").unwrap();
    std::fs::write(root.join("notes.rs"), b"fn main() {\n    let x = 1;\n}\n").unwrap();

    let (tx, mut rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    // Source = panel 0; the Details view = panel 1.
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    st.panels[1].format = ViewFormat::Details;

    // -- A text file → syntax-highlighted head --
    let idx = st.panels[0].entries.iter().position(|e| e.name == "notes.rs").unwrap();
    st.panels[0].cursor = idx;
    st.update_details();
    drain_until_preview(&mut st, &mut rx).await;
    match &st.details[1].preview {
        crate::details::Preview::Text(lines) => {
            assert!(lines.iter().any(|l| l.text.contains("fn main")), "text head loaded");
            assert!(lines.iter().any(|l| !l.runs.is_empty()), "a .rs head is syntax-highlighted");
        }
        other => panic!("expected a text preview, got {other:?}"),
    }

    // -- A directory → shallow tree --
    let didx = st.panels[0].entries.iter().position(|e| e.name == "sub").unwrap();
    st.panels[0].cursor = didx;
    st.update_details();
    drain_until_preview(&mut st, &mut rx).await;
    match &st.details[1].preview {
        crate::details::Preview::Tree(rows) => {
            assert!(rows.iter().any(|r| r.name == "inner.rs"), "tree lists the child file");
        }
        other => panic!("expected a tree preview, got {other:?}"),
    }

    // -- An image file → decoded thumbnail (exercises the image decoders) --
    image::RgbaImage::from_pixel(8, 6, image::Rgba([10, 200, 60, 255]))
        .save(root.join("pic.png"))
        .unwrap();
    st.panels[0].reload().await.unwrap();
    let pidx = st.panels[0].entries.iter().position(|e| e.name == "pic.png").unwrap();
    st.panels[0].cursor = pidx;
    st.update_details();
    drain_until_preview(&mut st, &mut rx).await;
    match &st.details[1].preview {
        crate::details::Preview::Image(pi) => {
            assert!(pi.img.width() > 0 && pi.img.height() > 0, "decoded a thumbnail");
        }
        other => panic!("expected an image preview, got {other:?}"),
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn f3_opens_image_viewer_and_falls_back_to_text() {
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rc_imgview_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    image::RgbaImage::from_pixel(12, 8, image::Rgba([200, 30, 60, 255]))
        .save(root.join("pic.png"))
        .unwrap();
    std::fs::write(root.join("notes.txt"), b"hello world\n").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();

    // F3 on the image → the viewer opens showing the decoded image.
    let idx = st.panels[0].entries.iter().position(|e| e.name == "pic.png").unwrap();
    st.panels[0].cursor = idx;
    st.open_view().await;
    let v = st.viewer.as_ref().expect("viewer opened for the image");
    assert!(v.active_image().is_some(), "F3 on an image shows it as an image");
    st.viewer = None;

    // F3 on a text file → the normal text viewer (no image).
    let idx = st.panels[0].entries.iter().position(|e| e.name == "notes.txt").unwrap();
    st.panels[0].cursor = idx;
    st.open_view().await;
    let v = st.viewer.as_ref().expect("viewer opened for the text file");
    assert!(v.active_image().is_none(), "a text file is not shown as an image");

    let _ = std::fs::remove_dir_all(&root);
}

/// F3 on an executable or library opens its analysis; on a file that only looks
/// like one, or on text, the ordinary viewer.
#[tokio::test]
async fn f3_opens_a_binary_in_binary_mode_and_anything_else_as_text() {
    let root = temp_dir("binview");
    std::fs::write(root.join("tool.o"), crate::viewer::binary::tests::sample_elf()).unwrap();
    let mut broken = b"\x7fELF\x02\x01\x01".to_vec();
    broken.resize(256, 0xff);
    std::fs::write(root.join("broken"), broken).unwrap();
    std::fs::write(root.join("notes.txt"), b"hello world\n").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    let view = async |st: &mut AppState, name: &str| {
        st.viewer = None;
        st.panels[0].cursor = st.panels[0].entries.iter().position(|e| e.name == name).unwrap();
        st.open_view().await;
        // A slow machine may still be analysing once F3 returns; the tick
        // collects it, as it would in the running program.
        for _ in 0..500 {
            if !st.viewer.as_ref().unwrap().analyzing() {
                break;
            }
            st.on_tick();
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        st.viewer.as_ref().unwrap().mode
    };

    use crate::viewer::ViewMode;
    assert_eq!(view(&mut st, "tool.o").await, ViewMode::Binary);
    assert!(st.viewer.as_ref().unwrap().active_binary().is_some(), "the analysis landed");
    assert_eq!(view(&mut st, "broken").await, ViewMode::Text, "unparseable: the text view");
    assert_eq!(view(&mut st, "notes.txt").await, ViewMode::Text);

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn f3_on_a_certificate_shows_what_it_holds_and_a_readme_quoting_one_stays_text() {
    let root = temp_dir("certview");
    let bundle =
        format!("{}{}", crate::certs::testdata::LEAF, crate::certs::testdata::INTERMEDIATE);
    std::fs::write(root.join("fullchain.pem"), &bundle).unwrap();
    std::fs::write(root.join("README.md"), format!("# TLS\n\nPaste this:\n\n{bundle}")).unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    let view = async |st: &mut AppState, name: &str| {
        st.viewer = None;
        st.panels[0].cursor = st.panels[0].entries.iter().position(|e| e.name == name).unwrap();
        st.open_view().await;
        st.viewer.as_ref().unwrap().active_certs().map(|c| c.report.summary.clone())
    };
    assert_eq!(view(&mut st, "fullchain.pem").await.as_deref(), Some("2 certificates"));
    assert_eq!(view(&mut st, "README.md").await, None);
    let _ = std::fs::remove_dir_all(&root);
}

/// Follow mode is driven by the render loop's tick, which only runs when
/// something asks for it — with the status widget and animations off, nothing
/// else would, and a followed log would sit still.
#[tokio::test]
async fn a_followed_file_keeps_the_tick_running_and_grows_on_it() {
    let root = temp_dir("follow_tick");
    let log = root.join("app.log");
    std::fs::write(&log, b"one\n").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.config.system_status = false;
    st.config.animation = false;
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    st.panels[0].cursor = st.panels[0].entries.iter().position(|e| e.name == "app.log").unwrap();
    st.open_view().await;
    assert!(!st.wants_ticks(), "an unfollowed viewer needs no tick");

    st.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE)).await;
    assert!(st.viewer.as_ref().unwrap().following(), "F3 on a local file can be followed");
    assert!(st.wants_ticks());

    use std::io::Write;
    std::fs::OpenOptions::new().append(true).open(&log).unwrap().write_all(b"two\n").unwrap();
    assert!(st.on_tick(), "the tick notices the growth and asks for a redraw");

    let _ = std::fs::remove_dir_all(&root);
}

/// The command line's readline editor runs *before* the panel shortcuts, so a
/// panel binding on a chord it always claims (Ctrl-A/B/F/D/K/Y/H, …) would never
/// fire. This guards the Git bindings against that trap — `Ctrl-D` was chosen
/// once and silently did nothing, because readline uses it to delete a character.
#[test]
fn git_shortcuts_are_not_swallowed_by_the_command_line() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
    let alt = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT);

    // The Git panel bindings must reach the panel handler on an empty *and* a
    // non-empty command line.
    for empty in [true, false] {
        assert!(
            !super::keys::cmdline_edit_wanted(ctrl('g'), empty),
            "Ctrl-G (stage) must reach the panel"
        );
        assert!(
            !super::keys::cmdline_edit_wanted(alt('g'), empty),
            "Alt-G (git menu) must reach the panel"
        );
        assert!(
            !super::keys::cmdline_edit_wanted(alt('d'), empty),
            "Alt-D (diff) must reach the panel"
        );
    }
    // ...and the chord we deliberately avoided really is claimed by readline.
    assert!(
        super::keys::cmdline_edit_wanted(ctrl('d'), true),
        "Ctrl-D is readline's delete-char, which is why the diff is on Alt-D"
    );
}

/// A panel chord on `Alt`+letter is dead if that letter also opens a menu, since
/// the menu bar claims it before `route_key` ever runs. This caught a real bug:
/// the clipboard copy was first put on Alt-C, which just opened the Command menu.
#[test]
fn panel_alt_chords_do_not_collide_with_the_menu_bar() {
    // The menu accelerators are the first letters of the bar's titles.
    let claimed: Vec<char> = crate::ui::menubar::TITLES
        .iter()
        .filter_map(|t| t.chars().next())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    assert!(claimed.contains(&'c'), "Command owns Alt-C — this is why the copy is on Ctrl-Insert");
    // Every Alt+letter the panel handler binds must be free of that set. Alt-O
    // and Alt-F are the documented exceptions: the dispatcher explicitly lets
    // them through to the panel (Alt-F only on an empty command line).
    for c in ['y', 'u', 'i', 't', 'k', 'j', 'g', 'd', 'h', 'n', 'p'] {
        assert!(
            !claimed.contains(&c),
            "Alt-{c} is a menu accelerator, so a panel binding on it would be dead code"
        );
    }
}

/// The sync flow end to end at the app level: options dialog → background plan →
/// preview → execute through the ops engine.
#[tokio::test]
async fn sync_plans_previews_and_executes_a_mirror() {
    use crate::app::event::AppEvent;
    use crate::ui::dialog::Submit;
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rc_syncflow_{}_{nanos}", std::process::id()));
    let (da, db) = (root.join("a"), root.join("b"));
    std::fs::create_dir_all(&da).unwrap();
    std::fs::create_dir_all(&db).unwrap();
    std::fs::write(da.join("new.txt"), b"fresh").unwrap();
    std::fs::write(db.join("stale.txt"), b"old").unwrap();

    let (tx, mut rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.panels[0].cwd = VfsPath::local(&da);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    st.panels[1].cwd = VfsPath::local(&db);
    st.panels[1].backend = st.registry.local();
    st.panels[1].reload().await.unwrap();

    // The Command menu / Compare dialog opens the options form.
    st.open_sync();
    assert!(matches!(st.dialog, Some(Dialog::Form(_))), "the sync options form opens");

    // Choosing a mirror plans it in the background behind a spinner.
    let mode = crate::ops::sync::SyncMode::OneWay { delete_extraneous: true };
    st.handle_submit(Submit::SyncPlan(mode)).await;
    assert!(matches!(st.dialog, Some(Dialog::Busy(_))), "planning shows a spinner");

    // The plan arrives as an event and becomes the preview.
    let plan = match rx.recv().await.expect("a SyncPlanned event") {
        AppEvent::SyncPlanned { result } => *result.expect("planning succeeded"),
        other => panic!("expected SyncPlanned, got {other:?}"),
    };
    // It copies the new file and removes the extraneous one — and nothing has
    // happened on disk yet.
    assert_eq!(plan.counts().copies, 1);
    assert_eq!(plan.counts().deletes, 1);
    assert!(db.join("stale.txt").exists(), "preview alone changes nothing");
    st.on_sync_planned(Ok(plan.clone()));
    assert!(matches!(st.dialog, Some(Dialog::SyncPreview(_))), "the plan is previewed");

    // Executing runs it as a normal backgroundable transfer task.
    st.handle_submit(Submit::SyncRun(Box::new(plan))).await;
    match &st.dialog {
        Some(Dialog::Progress(p)) => {
            assert!(p.backgroundable, "a sync can be sent to the background");
            assert!(st.task_progress.contains_key(&p.id), "it shows in the background list");
        }
        other => panic!("expected a progress dialog, got {:?}", other.is_some()),
    }

    // Drain events until the task reports done, then check the mirror landed.
    loop {
        match rx.recv().await.expect("task events") {
            AppEvent::TaskDone { .. } => break,
            _ => continue,
        }
    }
    assert_eq!(std::fs::read(db.join("new.txt")).unwrap(), b"fresh");
    assert!(!db.join("stale.txt").exists(), "the extraneous file was removed");

    let _ = std::fs::remove_dir_all(&root);
}

/// The panel-navigation shortcuts: Alt-I (sync the other panel), Alt-O (show the
/// cursor's directory there and step on), and Alt-H (the directory history list).
#[tokio::test]
async fn alt_panel_navigation_shortcuts() {
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rc_altnav_{}_{nanos}", std::process::id()));
    let sub = root.join("alpha");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::create_dir_all(root.join("beta")).unwrap();
    std::fs::write(root.join("zz.txt"), b"x").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    st.panels[1].cwd = VfsPath::local(std::env::temp_dir());
    st.panels[1].backend = st.registry.local();
    st.panels[1].reload().await.unwrap();
    let alt = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT);

    // -- Alt-I: the inactive panel joins the active one --
    assert_ne!(st.panels[1].cwd, st.panels[0].cwd);
    st.handle_key(alt('i')).await;
    assert_eq!(st.panels[1].cwd, VfsPath::local(&root), "Alt-I syncs the other panel");

    // -- Alt-O on a directory: the other panel enters it, the cursor steps on --
    let idx = st.panels[0].entries.iter().position(|e| e.name == "alpha").unwrap();
    st.panels[0].cursor = idx;
    st.handle_key(alt('o')).await;
    assert_eq!(st.panels[1].cwd, VfsPath::local(&sub), "Alt-O opens the directory there");
    assert_eq!(st.panels[0].cursor, idx + 1, "and the cursor advances");

    // -- Alt-O on a file: the other panel gets this directory instead --
    let f = st.panels[0].entries.iter().position(|e| e.name == "zz.txt").unwrap();
    st.panels[0].cursor = f;
    st.handle_key(alt('o')).await;
    assert_eq!(
        st.panels[1].cwd,
        VfsPath::local(&root),
        "Alt-O on a file shows the containing directory"
    );

    // -- Alt-H lists the directory history and can jump into it --
    // The left panel has visited nothing yet; give it some history.
    st.panels[0].cursor = idx;
    st.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await; // enter alpha/
    assert_eq!(st.panels[0].cwd, VfsPath::local(&sub));
    st.handle_key(alt('h')).await;
    assert!(matches!(st.dialog, Some(Dialog::DirHistory(_))), "Alt-H lists the history");
    // The previous directory is one row up; Enter jumps straight back to it.
    st.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)).await;
    st.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
    assert!(st.dialog.is_none(), "the window closes");
    assert_eq!(st.panels[0].cwd, VfsPath::local(&root), "picking an entry jumps there");

    let _ = std::fs::remove_dir_all(&root);
}

/// A transfer sent to the background and pulled back must keep the speed chart it
/// had built up — the whole run, including what it recorded while away.
///
/// The bug this guards: the history used to live only on the progress dialog, so
/// dismissing it for the background threw the samples away, and the dialog rebuilt
/// on return started from an empty chart.
#[tokio::test]
async fn a_backgrounded_transfer_keeps_its_speed_history() {
    use crate::app::event::AppEvent;
    use crate::ui::dialog::Submit;
    // The chart samples at most ~10×/s, so each recorded point needs a real gap.
    async fn tick(st: &mut AppState, done: u64) {
        std::thread::sleep(std::time::Duration::from_millis(110));
        st.apply_event(AppEvent::Progress(progress_update(1, "Copying", done, 1000))).await;
    }

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    // A live task, so it can be sent to the background and pulled back.
    let (reply_tx, _reply_rx) = tokio::sync::mpsc::channel(1);
    st.tasks.insert(1, TaskHandle { id: 1, cancel: CancelToken::new(), reply: reply_tx });
    st.task_progress.insert(
        1,
        BgTransfer { verb: "Copying", update: None, schemes: vec![], chart: Default::default() },
    );
    // Foreground: a couple of updates build some history.
    st.dialog = Some(Dialog::Progress(st.progress_dialog_for(1)));
    tick(&mut st, 100).await;
    tick(&mut st, 200).await;
    let before = match &st.dialog {
        Some(Dialog::Progress(p)) => p.chart.samples.len(),
        _ => panic!("a progress dialog"),
    };
    assert!(before > 0, "the foreground dialog charted something");

    // To background: the dialog goes away.
    st.handle_dialog_result(DialogResult::Background(1)).await;
    assert!(st.dialog.is_none(), "the progress dialog is dismissed");

    // It keeps transferring — and keeps charting — while nobody is watching.
    tick(&mut st, 300).await;
    tick(&mut st, 400).await;
    let while_away = st.task_progress[&1].chart.samples.len();
    assert!(while_away > before, "the chart grew while backgrounded: {while_away} vs {before}");

    // Back to the foreground: the whole run is there, not a fresh empty chart.
    st.handle_submit(Submit::ForegroundTask(1)).await;
    match &st.dialog {
        Some(Dialog::Progress(p)) => {
            assert_eq!(
                p.chart.samples.len(),
                while_away,
                "the restored dialog shows every sample, including the pre-background ones"
            );
            assert!(p.chart.peak_speed > 0.0, "the peak survives too");
        }
        _ => panic!("the progress dialog is restored"),
    }

    // And it carries on from there rather than restarting.
    tick(&mut st, 500).await;
    match &st.dialog {
        Some(Dialog::Progress(p)) => {
            assert!(p.chart.samples.len() > while_away, "charting continues after the restore")
        }
        _ => panic!("a progress dialog"),
    }
}

/// Saving a file must *finalize* the write, not merely flush it — for the remote
/// backends the upload only commits (and surfaces its error) on shutdown. This
/// pins `write_file` calling `shutdown()`; without it, an editor save over
/// SFTP/SCP/FTP could report success on an incomplete upload.
#[tokio::test]
async fn write_file_finalizes_the_write_with_shutdown() {
    use crate::vfs::testmock::MockVfs;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let flag = Arc::new(AtomicBool::new(false));
    let backend: Arc<dyn crate::vfs::Vfs> =
        MockVfs { file_size: 0, read_fail_after: None, shutdown_called: flag.clone() }.arc();
    super::write_file(&backend, &VfsPath::local("/out.txt"), b"hello")
        .await
        .expect("write succeeds");
    assert!(flag.load(Ordering::SeqCst), "write_file must call shutdown() to finalize the upload");
}

/// Esc on a cancellable Busy spinner aborts the task it was waiting on (so a
/// git pull against an unreachable remote can't hang forever) and closes it.
#[tokio::test]
async fn esc_on_a_cancellable_busy_aborts_its_task() {
    use crate::ui::dialog::{BusyDialog, DialogResult};
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);

    // A task that never finishes on its own — only an abort ends it.
    let handle = tokio::spawn(async { std::future::pending::<()>().await });
    st.busy_task = Some(handle);
    st.dialog = Some(Dialog::Busy(BusyDialog::new("Git", "Running git pull…").cancellable()));

    // Route Esc the way the dialog would: it reports Cancel, which the app applies.
    st.handle_dialog_result(DialogResult::Cancel).await;

    assert!(st.dialog.is_none(), "the spinner is dismissed");
    assert!(st.busy_task.is_none(), "the task handle was taken and aborted");
}

/// A destructive op on a file whose name isn't valid UTF-8 is refused with a
/// clear message, rather than acting on a lossily-decoded (wrong/nonexistent)
/// path. Guards copy/move/delete/rename.
#[tokio::test]
async fn operations_refuse_a_non_utf8_filename() {
    use crate::ops::OpKind;
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    // A path whose name carries the replacement char (as a lossy listing would).
    let lossy = VfsPath::local("/tmp/ba\u{FFFD}d.bin");
    assert!(lossy.has_lossy_name());

    st.start_op(OpKind::Delete, vec![lossy], None, None, None);

    assert!(
        matches!(&st.dialog, Some(Dialog::Message(m)) if m.is_error),
        "an error is shown instead of running the op"
    );
    assert!(st.tasks.is_empty(), "no delete task was spawned");
}

/// The session layout — panel directories, filter, split, hidden/half-height and
/// the active panel — is captured into the config on exit and restored on the
/// next launch. Uses `capture_session` (no disk write) plus a config round-trip.
#[tokio::test]
async fn session_layout_is_captured_and_survives_a_config_round_trip() {
    use crate::ui::layout::SplitDir;
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);

    // Arrange a distinctive layout on the live state.
    let dir = std::env::temp_dir();
    st.panels[0].cwd = VfsPath::local(&dir);
    st.panels[0].filter = Some("*.rs".to_string());
    st.panels[1].cwd = VfsPath { scheme: "sftp-1".into(), path: "/remote".into(), container: None };
    st.split = SplitDir::Horizontal;
    st.panel_hidden = [false, true];
    st.half_height = true;
    st.active = 1;

    st.capture_session();
    let c = &st.config;
    assert_eq!(c.panel_dirs[0], dir.to_string_lossy(), "a local dir is saved");
    assert_eq!(c.panel_dirs[1], "", "a remote panel saves no directory (not restorable)");
    assert_eq!(c.panel_filters[0], "*.rs");
    assert!(c.split_horizontal && c.half_height);
    assert_eq!(c.panel_hidden, [false, true]);
    assert_eq!(c.active_panel, 1);

    // The whole config survives a TOML save/load unchanged (the fields persist).
    let text = toml::to_string_pretty(&st.config).unwrap();
    let back: crate::config::Config = toml::from_str(&text).unwrap();
    assert_eq!(back.panel_dirs, st.config.panel_dirs);
    assert_eq!(back.panel_filters, st.config.panel_filters);
    assert_eq!(back.split_horizontal, st.config.split_horizontal);
    assert_eq!(back.panel_hidden, st.config.panel_hidden);
    assert_eq!(back.half_height, st.config.half_height);
    assert_eq!(back.active_panel, st.config.active_panel);
}

// ---------------------------------------------------------------------------
// Archives as panels
// ---------------------------------------------------------------------------
//
// The backend's own tests (`vfs::archive::tests`) cover the archive operations
// directly. These drive the same ground through `AppState`, so the panel keys
// and dialogs that reach them — F5/F6/F7/F8 — are wired to the right thing.

/// A private scratch directory holding one archive, removed when the test ends.
struct ArchiveFixture {
    dir: PathBuf,
    container: PathBuf,
}

impl ArchiveFixture {
    /// `<scratch>/box.zip` containing `notes.txt` and `data/{a.txt,b.txt}`,
    /// alongside an empty `<scratch>/out` directory for the other panel.
    fn new(tag: &str) -> Self {
        let dir = crate::util::temp::rc_temp_path(&format!("test-panel-{tag}"));
        std::fs::create_dir_all(dir.join("tree/data")).unwrap();
        std::fs::create_dir_all(dir.join("out")).unwrap();
        std::fs::write(dir.join("tree/notes.txt"), b"notes").unwrap();
        std::fs::write(dir.join("tree/data/a.txt"), b"alpha").unwrap();
        std::fs::write(dir.join("tree/data/b.txt"), b"beta").unwrap();
        let container = dir.join("box.zip");
        archive::create_archive(
            ArchiveFormat::Zip,
            &container,
            &[dir.join("tree/notes.txt"), dir.join("tree/data")],
        )
        .unwrap();
        ArchiveFixture { dir, container }
    }

    fn out(&self) -> PathBuf {
        self.dir.join("out")
    }

    fn inner(&self, path: &str) -> VfsPath {
        VfsPath::archive(&self.container, path)
    }

    /// Every member the archive stores, sorted.
    fn members(&self) -> Vec<String> {
        let mut v: Vec<String> =
            archive::formats::list_entries(ArchiveFormat::Zip, &self.container)
                .unwrap()
                .into_iter()
                .map(|e| e.path)
                .collect();
        v.sort();
        v
    }
}

impl Drop for ArchiveFixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

/// Panel 0 shows `inner` of the archive, panel 1 shows `right`; panel 0 active.
async fn archive_state(
    fx: &ArchiveFixture,
    inner: &str,
    right: VfsPath,
) -> (AppState, crate::util::async_bridge::AppReceiver) {
    let (tx, rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    let cwd = fx.inner(inner);
    st.panels[0].backend = st.registry.resolve(&cwd).unwrap();
    st.panels[0].cwd = cwd;
    st.panels[0].reload().await.unwrap();
    st.panels[1].backend = st.registry.resolve(&right).unwrap();
    st.panels[1].cwd = right;
    st.panels[1].reload().await.unwrap();
    st.active = 0;
    (st, rx)
}

/// Put the cursor on the named entry of the active panel.
fn point_at(st: &mut AppState, name: &str) {
    let p = &mut st.panels[st.active];
    p.cursor = p.entries.iter().position(|e| e.name == name).unwrap_or_else(|| {
        panic!("{name} is not listed: {:?}", p.entries.iter().map(|e| &e.name).collect::<Vec<_>>())
    });
}

fn entry_names(st: &AppState, side: usize) -> Vec<String> {
    let mut v: Vec<String> =
        st.panels[side].entries.iter().map(|e| e.name.clone()).filter(|n| n != "..").collect();
    v.sort();
    v
}

/// F7 inside an archive creates the directory, the way it does on disk — it
/// used to fail outright with "operation not supported by this filesystem".
#[tokio::test]
async fn f7_makes_a_directory_inside_an_archive() {
    let fx = ArchiveFixture::new("mkdir");
    let (mut st, _rx) = archive_state(&fx, "/", VfsPath::local(fx.out())).await;

    st.handle_submit(Submit::MkDir("fresh".into())).await;

    assert!(st.dialog.is_none(), "no error dialog");
    assert_eq!(entry_names(&st, 0), ["data", "fresh", "notes.txt"], "the panel shows it");
    assert!(fx.members().contains(&"/fresh".to_string()), "{:?}", fx.members());
}

/// Shift-F4 inside an archive writes the new file into the archive on save,
/// the way F7 creates a directory there.
#[tokio::test]
async fn shift_f4_creates_a_file_inside_an_archive() {
    let fx = ArchiveFixture::new("newfile");
    let (mut st, _rx) = archive_state(&fx, "/", VfsPath::local(fx.out())).await;

    st.open_new_file_editor("fresh.txt".into()).await;
    assert!(st.dialog.is_none(), "no error dialog");
    let ed = st.editor.as_ref().expect("the editor opens on the new archive member");
    assert_eq!(ed.path, fx.inner("/fresh.txt"));

    st.editor.as_mut().unwrap().insert_at_cursor("hello");
    st.save_editor(false).await;
    assert!(fx.members().contains(&"/fresh.txt".to_string()), "{:?}", fx.members());
}

/// F8 on a directory inside an archive deletes it and everything under it.
#[tokio::test]
async fn f8_deletes_a_directory_inside_an_archive() {
    let fx = ArchiveFixture::new("delete");
    let (mut st, mut rx) = archive_state(&fx, "/", VfsPath::local(fx.out())).await;

    st.handle_submit(Submit::Delete(vec![fx.inner("/data")])).await;
    drain_taskdone(&mut st, &mut rx).await;

    assert_eq!(fx.members(), ["/notes.txt"]);
    assert_eq!(entry_names(&st, 0), ["notes.txt"]);
}

/// F5 out of an archive extracts the file to the other panel, leaving the
/// archive alone.
#[tokio::test]
async fn f5_copies_a_file_out_of_an_archive() {
    let fx = ArchiveFixture::new("copy-out");
    let (mut st, mut rx) = archive_state(&fx, "/data", VfsPath::local(fx.out())).await;
    point_at(&mut st, "a.txt");

    st.handle_key(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE)).await;
    assert!(matches!(st.dialog, Some(Dialog::Input(_))), "the destination prompt opens");
    st.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
    drain_taskdone(&mut st, &mut rx).await;

    assert_eq!(std::fs::read(fx.out().join("a.txt")).unwrap(), b"alpha");
    assert!(fx.members().contains(&"/data/a.txt".to_string()), "a copy leaves the original");
}

/// F6 out of an archive extracts the file *and* removes it from the archive.
/// The delete half used to fail — the file landed on disk and the user got
/// "operation not supported by this filesystem" with the member still there.
#[tokio::test]
async fn f6_moves_a_file_out_of_an_archive() {
    let fx = ArchiveFixture::new("move-out");
    let (mut st, mut rx) = archive_state(&fx, "/data", VfsPath::local(fx.out())).await;
    point_at(&mut st, "a.txt");

    st.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE)).await;
    st.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
    drain_taskdone(&mut st, &mut rx).await;

    assert!(!matches!(st.dialog, Some(Dialog::Message(_))), "no error");
    assert_eq!(std::fs::read(fx.out().join("a.txt")).unwrap(), b"alpha");
    assert!(
        !fx.members().contains(&"/data/a.txt".to_string()),
        "gone from the archive: {:?}",
        fx.members()
    );
}

/// F6 with a bare name renames a member in place, the same gesture that renames
/// a file on disk. It used to be refused with "Cannot copy directly between
/// archives; extract first".
#[tokio::test]
async fn f6_renames_a_file_inside_an_archive() {
    let fx = ArchiveFixture::new("rename");
    let (mut st, mut rx) = archive_state(&fx, "/data", fx.inner("/")).await;
    point_at(&mut st, "a.txt");

    st.handle_submit(Submit::Move(vec![fx.inner("/data/a.txt")], "renamed.txt".into())).await;
    drain_taskdone(&mut st, &mut rx).await;

    assert!(!matches!(st.dialog, Some(Dialog::Message(_))), "no error");
    assert_eq!(fx.members(), ["/data", "/data/b.txt", "/data/renamed.txt", "/notes.txt"]);
    assert_eq!(entry_names(&st, 0), ["b.txt", "renamed.txt"]);
}

/// Moving between two directories of the *same* archive relocates the member
/// instead of refusing the whole operation.
#[tokio::test]
async fn f6_moves_a_file_between_directories_of_one_archive() {
    let fx = ArchiveFixture::new("move-within");
    let (mut st, mut rx) = archive_state(&fx, "/", fx.inner("/data")).await;
    point_at(&mut st, "notes.txt");

    st.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE)).await;
    match &st.dialog {
        Some(Dialog::Input(d)) => {
            assert_eq!(d.buffer, "/data", "prefilled with the path inside the archive")
        }
        _ => panic!("F6 into the same archive should open the destination prompt"),
    }
    st.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
    drain_taskdone(&mut st, &mut rx).await;

    assert!(!matches!(st.dialog, Some(Dialog::Message(_))), "no error");
    assert_eq!(fx.members(), ["/data", "/data/a.txt", "/data/b.txt", "/data/notes.txt"]);
}

/// Copying a local file into an archive over a member of the same name asks
/// first, then replaces it — one member, the new content. Before, the copy died
/// with the zip writer's "Duplicate filename" (and, in a tar, silently stored a
/// second member that readers never saw).
#[tokio::test]
async fn f5_into_an_archive_confirms_before_replacing_a_member() {
    let fx = ArchiveFixture::new("copy-in-clash");
    std::fs::write(fx.out().join("notes.txt"), b"REPLACED").unwrap();
    let (mut st, mut rx) = archive_state(&fx, "/", VfsPath::local(fx.out())).await;
    // Copy from the local panel into the archive panel.
    st.active = 1;
    point_at(&mut st, "notes.txt");
    assert!(st.config.confirm_overwrite, "the prompt is on by default");

    st.handle_key(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE)).await;
    // The archive is scanned in the background; the answer arrives as an event.
    let ev = rx.recv().await.unwrap();
    assert!(matches!(ev, AppEvent::ArchiveAddChecked { .. }), "the destination was checked");
    st.apply_event(ev).await;
    match &st.dialog {
        Some(Dialog::Confirm(d)) => {
            assert!(d.message.contains("notes.txt"), "names it: {}", d.message)
        }
        _ => panic!("an existing member must be confirmed before it is replaced"),
    }

    st.handle_dialog_result(DialogResult::Submit(Submit::ArchiveAdd(Box::new(
        crate::ops::ArchiveAdd {
            kind: OpKind::Copy,
            sources: vec![VfsPath::local(fx.out().join("notes.txt"))],
            dest: fx.inner("/"),
        },
    ))))
    .await;
    drain_taskdone(&mut st, &mut rx).await;

    assert_eq!(fx.members(), ["/data", "/data/a.txt", "/data/b.txt", "/notes.txt"], "one member");
    let fs = crate::vfs::archive::ArchiveFs::new();
    let mut r = fs.open_read(&fx.inner("/notes.txt")).await.unwrap();
    let mut got = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut r, &mut got).await.unwrap();
    assert_eq!(got, b"REPLACED");
}

/// Copying into an archive with nothing to overwrite goes straight through, no
/// question asked.
#[tokio::test]
async fn f5_into_an_archive_does_not_ask_when_nothing_is_replaced() {
    let fx = ArchiveFixture::new("copy-in-clean");
    std::fs::write(fx.out().join("new.txt"), b"new").unwrap();
    let (mut st, mut rx) = archive_state(&fx, "/", VfsPath::local(fx.out())).await;
    st.active = 1;
    point_at(&mut st, "new.txt");

    st.handle_key(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE)).await;
    st.apply_event(rx.recv().await.unwrap()).await;
    assert!(matches!(st.dialog, Some(Dialog::Progress(_))), "straight to the rebuild");
    drain_taskdone(&mut st, &mut rx).await;

    assert!(fx.members().contains(&"/new.txt".to_string()), "{:?}", fx.members());
}

/// F6 on a whole directory inside an archive extracts the subtree and takes it
/// out of the archive — the recursive delete has to walk the archive too.
#[tokio::test]
async fn f6_moves_a_directory_out_of_an_archive() {
    let fx = ArchiveFixture::new("move-dir-out");
    let (mut st, mut rx) = archive_state(&fx, "/", VfsPath::local(fx.out())).await;
    point_at(&mut st, "data");

    st.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE)).await;
    st.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
    drain_taskdone(&mut st, &mut rx).await;

    assert!(!matches!(st.dialog, Some(Dialog::Message(_))), "no error");
    assert_eq!(std::fs::read(fx.out().join("data/a.txt")).unwrap(), b"alpha");
    assert_eq!(std::fs::read(fx.out().join("data/b.txt")).unwrap(), b"beta");
    assert_eq!(fx.members(), ["/notes.txt"], "the whole subtree left the archive");
}

/// Copying between two *different* archives streams through the generic engine
/// (there is no local file to bulk-add), rather than being refused.
#[tokio::test]
async fn f5_copies_a_file_from_one_archive_into_another() {
    let src = ArchiveFixture::new("cross-src");
    let dst = ArchiveFixture::new("cross-dst");
    // Empty the destination so the copied name is unambiguous.
    archive::remove_from_archive(
        &dst.container,
        &HashSet::from(["/data".to_string(), "/notes.txt".to_string()]),
    )
    .unwrap();

    let (mut st, mut rx) = archive_state(&src, "/data", dst.inner("/")).await;
    point_at(&mut st, "a.txt");

    st.handle_key(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE)).await;
    match &st.dialog {
        Some(Dialog::Input(d)) => assert_eq!(d.buffer, "/", "the other archive's inner path"),
        _ => panic!("the destination prompt should open"),
    }
    st.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
    drain_taskdone(&mut st, &mut rx).await;

    assert!(!matches!(st.dialog, Some(Dialog::Message(_))), "no error");
    assert_eq!(dst.members(), ["/a.txt"]);
    assert!(src.members().contains(&"/data/a.txt".to_string()), "the source keeps its copy");
}

/// Directory sync writes file by file, and each write into an archive rebuilds
/// the whole container — so an archive destination is refused up front, with a
/// pointer at F5, which does the same job in one rebuild.
#[tokio::test]
async fn sync_refuses_an_archive_destination() {
    let fx = ArchiveFixture::new("sync-dest");
    let (mut st, _rx) = archive_state(&fx, "/", VfsPath::local(fx.out())).await;
    st.active = 1; // sync runs from the local panel into the archive panel

    st.open_sync();

    match &st.dialog {
        Some(Dialog::Message(m)) => {
            assert!(m.message.contains("F5"), "points at F5: {}", m.message)
        }
        _ => panic!("an archive sync destination should be refused"),
    }
}

/// A temp directory named after the calling test, unique per process/run.
fn temp_dir(tag: &str) -> std::path::PathBuf {
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("rc_{tag}_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The 3D view describes the *other* panel, so it has to keep following that
/// panel after the focus moves over there — which is the normal way to use it.
#[tokio::test]
async fn the_3d_view_follows_the_other_panel_even_when_it_is_the_active_one() {
    let root = temp_dir("space3d_follow");
    for name in ["alpha", "beta"] {
        std::fs::create_dir_all(root.join(name)).unwrap();
        std::fs::write(root.join(name).join("f"), vec![0u8; 4096]).unwrap();
    }
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    // Left panel shows the 3D view; the right panel is the one being driven.
    let backend = st.registry.local();
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[1].cwd = VfsPath::local(&root);
    st.panels[1].backend = backend;
    st.init().await;
    st.set_format(0, ViewFormat::Space3d).await;

    st.update_space3d();
    assert_eq!(
        st.panels[0].space3d.as_ref().map(|s| s.focus.clone()),
        Some(root.clone()),
        "the view starts on the other panel's directory"
    );

    // Now the user tabs to the right panel and walks into a subdirectory. The
    // 3D panel is no longer the active one — it must still follow.
    st.active = 1;
    st.panels[1].cwd = VfsPath::local(root.join("alpha"));
    st.update_space3d();
    assert_eq!(
        st.panels[0].space3d.as_ref().map(|s| s.focus.clone()),
        Some(root.join("alpha")),
        "the view followed the other panel after the focus moved away from it"
    );
    // And the crawler is pointed there too, rather than being left behind on
    // whatever the active panel happens to be.
    st.update_sizes();
    assert_eq!(
        st.sizes_focus.as_deref(),
        Some(root.join("alpha").as_path()),
        "the crawler follows an inactive panel's 3D view"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Enter and Backspace in the 3D view drive the *other* panel, the way Enter
/// does in Tree view — which is also what moves the view's own focus.
#[tokio::test]
async fn enter_and_backspace_in_the_3d_view_walk_the_other_panel() {
    let root = temp_dir("space3d_enter");
    std::fs::create_dir_all(root.join("alpha")).unwrap();
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[1].cwd = VfsPath::local(&root);
    st.init().await;
    st.set_format(0, ViewFormat::Space3d).await;
    st.active = 0;
    st.update_space3d();
    st.update_sizes();

    // Point the selection at "alpha" and press Enter.
    let target = root.join("alpha");
    if let Some(sp) = st.panels[0].space3d.as_mut() {
        sp.nodes.push(crate::space3d::SceneNode {
            name: "alpha".into(),
            path: target.clone(),
            size: 0,
            parent: Some(0),
            depth: 1,
            scale: 0.5,
            target: crate::space3d::vec3::v3(0.0, -1.0, 0.0),
            target_half: 0.1,
            partial: false,
            is_focus: false,
            is_cursor: false,
            context: false,
            files: Vec::new(),
            target_plat: 0.1,
        });
        sp.selected = sp.nodes.len() - 1;
    }
    st.space3d_enter().await;
    assert_eq!(st.panels[1].cwd.path, target, "Enter moved the other panel");
    assert_eq!(st.active, 0, "and left the focus on the 3D panel");

    st.space3d_up().await;
    assert_eq!(st.panels[1].cwd.path, root, "Backspace walked the other panel back up");
    let _ = std::fs::remove_dir_all(&root);
}

/// The highlight follows the other panel's cursor, and only lands on something
/// the scene can actually draw.
#[tokio::test]
async fn the_3d_view_highlights_the_directory_under_the_other_panels_cursor() {
    let root = temp_dir("space3d_cursor");
    std::fs::create_dir_all(root.join("alpha")).unwrap();
    std::fs::write(root.join("zzz.txt"), b"hi").unwrap();
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[1].cwd = VfsPath::local(&root);
    st.init().await;
    st.set_format(0, ViewFormat::Space3d).await;

    let cursor_of = |st: &AppState| st.panels[0].space3d.as_ref().and_then(|s| s.cursor.clone());
    let put_cursor_on = |st: &mut AppState, name: &str| {
        let i = st.panels[1].entries.iter().position(|e| e.name == name).expect(name);
        st.panels[1].cursor = i;
    };

    put_cursor_on(&mut st, "alpha");
    st.update_space3d();
    assert_eq!(cursor_of(&st), Some(root.join("alpha")), "a directory is highlighted");

    // A file has no box, so nothing is highlighted.
    put_cursor_on(&mut st, "zzz.txt");
    st.update_space3d();
    assert_eq!(cursor_of(&st), None, "a file highlights nothing");

    // Neither does "..", which leads out of the tree entirely.
    put_cursor_on(&mut st, "..");
    st.update_space3d();
    assert_eq!(cursor_of(&st), None, "`..` highlights nothing");
    let _ = std::fs::remove_dir_all(&root);
}

/// Dragging with either button turns the scene; releasing ends it.
#[tokio::test]
async fn dragging_over_the_3d_panel_orbits_the_camera() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let root = temp_dir("space3d_drag");
    std::fs::create_dir_all(root.join("alpha")).unwrap();
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[1].cwd = VfsPath::local(&root);
    st.init().await;
    st.set_format(0, ViewFormat::Space3d).await;

    // `handle_mouse` maps against the area of the last frame drawn.
    let mut t = Terminal::new(TestBackend::new(120, 30)).unwrap();
    t.draw(|f| crate::ui::draw(f, &mut st)).unwrap();
    let hit = st.panels[0].hit.expect("panel geometry");
    let (cx, cy) = (hit.body.x + hit.body.width / 2, hit.body.y + hit.body.height / 2);

    let ev = |kind, col, row| MouseEvent {
        kind,
        column: col,
        row,
        modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
    };
    let angles = |st: &AppState| st.panels[0].space3d.as_ref().unwrap().goal_angles();
    let yaw = |st: &AppState| angles(st).0;
    let pitch = |st: &AppState| angles(st).1;

    for button in [MouseButton::Left, MouseButton::Right] {
        let (y0, p0) = (yaw(&st), pitch(&st));
        st.handle_mouse(ev(MouseEventKind::Down(button), cx, cy)).await;
        st.handle_mouse(ev(MouseEventKind::Drag(button), cx + 8, cy + 3)).await;
        assert!(yaw(&st) != y0, "{button:?} drag turned the scene");
        assert!(pitch(&st) != p0, "and tilted it");

        // Releasing ends the drag: a later move with no button does nothing.
        let (y1, p1) = (yaw(&st), pitch(&st));
        st.handle_mouse(ev(MouseEventKind::Up(button), cx + 8, cy + 3)).await;
        st.handle_mouse(ev(MouseEventKind::Drag(button), cx + 30, cy + 9)).await;
        assert_eq!(yaw(&st), y1, "no orbit after the button came up");
        assert_eq!(pitch(&st), p1);
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// An orbit is measured from the pointer's own travel, so the input batch may
/// fold its drag reports into a single frame. A drag over a listing moves the
/// cursor onto whatever the last frame drew under the pointer, so it may not.
#[tokio::test]
async fn only_an_orbit_drag_folds_into_the_input_batch() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let root = temp_dir("space3d_fold");
    std::fs::create_dir_all(root.join("alpha")).unwrap();
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[1].cwd = VfsPath::local(&root);
    st.init().await;
    st.set_format(0, ViewFormat::Space3d).await;

    let mut t = Terminal::new(TestBackend::new(120, 30)).unwrap();
    t.draw(|f| crate::ui::draw(f, &mut st)).unwrap();
    let centre = |i: usize| {
        let hit = st.panels[i].hit.expect("panel geometry");
        (hit.body.x + hit.body.width / 2, hit.body.y + hit.body.height / 2)
    };
    let ((sx, sy), (lx, ly)) = (centre(0), centre(1));
    let ev = |kind, col, row| MouseEvent {
        kind,
        column: col,
        row,
        modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
    };
    let left = MouseButton::Left;

    assert!(st.mouse_folds(&ev(MouseEventKind::Moved, sx, sy)), "bare motion does nothing");
    assert!(!st.mouse_folds(&ev(MouseEventKind::Down(left), sx, sy)), "a press hit-tests");
    assert!(!st.mouse_folds(&ev(MouseEventKind::Drag(left), sx, sy)), "no orbit is armed yet");

    st.handle_mouse(ev(MouseEventKind::Down(left), sx, sy)).await;
    assert!(st.mouse_folds(&ev(MouseEventKind::Drag(left), sx + 8, sy + 3)), "an orbit folds");
    assert!(st.mouse_folds(&ev(MouseEventKind::Up(left), sx + 8, sy + 3)), "and so does its end");
    st.handle_mouse(ev(MouseEventKind::Up(left), sx + 8, sy + 3)).await;
    assert!(!st.mouse_folds(&ev(MouseEventKind::Drag(left), sx, sy)), "released");

    for button in [MouseButton::Left, MouseButton::Right] {
        st.handle_mouse(ev(MouseEventKind::Down(button), lx, ly)).await;
        assert!(
            !st.mouse_folds(&ev(MouseEventKind::Drag(button), lx, ly + 1)),
            "a {button:?} drag over the listing moves or marks what is under it"
        );
        st.handle_mouse(ev(MouseEventKind::Up(button), lx, ly + 1)).await;
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// A remote panel has nothing crawlable, so the 3D view opposite it holds
/// whatever it last showed rather than blanking.
#[tokio::test]
async fn a_remote_other_panel_leaves_the_3d_view_alone() {
    let root = temp_dir("space3d_remote");
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[1].cwd = VfsPath::local(&root);
    st.init().await;
    st.set_format(0, ViewFormat::Space3d).await;
    st.update_space3d();
    let before = st.panels[0].space3d.as_ref().map(|s| s.focus.clone());

    st.panels[1].cwd.scheme = "sftp".into();
    st.update_space3d();
    assert_eq!(
        st.panels[0].space3d.as_ref().map(|s| s.focus.clone()),
        before,
        "a remote directory cannot be sized, so the view keeps what it had"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// F9 → Command → "Go to line…" raises the prompt, and submitting it moves the
/// editor's cursor.
#[tokio::test]
async fn editor_goto_line_prompt_moves_the_cursor() {
    let dir = temp_dir("edgoto");
    let file = dir.join("lines.txt");
    std::fs::write(&file, b"a\nb\nc\nd\ne").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.open_path_in_editor(file).await;

    st.apply_editor_signal(EditorSignal::OpenGotoLine).await;
    assert!(matches!(st.dialog, Some(Dialog::Input(_))), "the line prompt is up");
    // The prompt is prefilled with the current line, 1-based.
    if let Some(Dialog::Input(d)) = st.dialog.as_ref() {
        assert_eq!(d.buffer, "1");
    }
    st.handle_submit(Submit::EditorGotoLine("4".into())).await;
    assert_eq!(st.editor.as_ref().unwrap().cursor_line_col(), (3, 0));

    // Nonsense is reported rather than silently ignored.
    st.handle_submit(Submit::EditorGotoLine("nope".into())).await;
    assert!(matches!(st.dialog, Some(Dialog::Message(_))));

    std::fs::remove_dir_all(&dir).ok();
}

/// A CSV file opens in the editor's spreadsheet grid, and a cell edited there
/// is written to disk by the ordinary save — quoted where the value needs it.
#[tokio::test]
async fn a_csv_edited_in_the_grid_saves_as_text() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let dir = temp_dir("edgrid");
    let file = dir.join("prices.csv");
    std::fs::write(&file, b"item,price\ntea,3\n").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.open_path_in_editor(file.clone()).await;
    let ed = st.editor.as_mut().unwrap();
    assert!(ed.sheet_active(), "a CSV file opens in the grid");
    // Backspace starts an edit of the cell with its old value cleared.
    for code in [KeyCode::Down, KeyCode::Backspace] {
        ed.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }
    for c in "green tea, loose".chars() {
        ed.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    ed.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    st.handle_submit(Submit::EditorSave).await;
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "item,price\n\"green tea, loose\",3\n");
    std::fs::remove_dir_all(&dir).ok();
}

/// A JSON file is checked on the app's tick while it is edited: the ticker
/// keeps running while a check is due, and the error shows once it lands.
#[tokio::test]
async fn an_edited_json_file_is_checked_on_the_tick() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let dir = temp_dir("edjson");
    let file = dir.join("config.json");
    std::fs::write(&file, b"{\"a\": 1}\n").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.open_path_in_editor(file).await;
    let settle = |st: &mut AppState| {
        let deadline = Instant::now() + Duration::from_secs(5);
        while st.editor.as_ref().is_some_and(|e| e.check_pending()) && Instant::now() < deadline {
            st.on_tick();
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    settle(&mut st);
    assert!(st.editor.as_ref().unwrap().check_errors().is_empty(), "the file as saved is valid");
    // Break it: a second member with no comma before it.
    let ed = st.editor.as_mut().unwrap();
    for code in [KeyCode::End, KeyCode::Left] {
        ed.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }
    for c in " \"b\": 2".chars() {
        ed.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    assert!(st.wants_ticks(), "a check is due, so the tick keeps coming");
    settle(&mut st);
    let errors = st.editor.as_ref().unwrap().check_errors();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].message, "Missing ',' after this value");
    std::fs::remove_dir_all(&dir).ok();
}

/// Alt-M in the editor opens the GeoJSON map at once, fills it from a read in
/// the background, and Go to puts the editor's cursor on what was picked; a file
/// F4 on a shapefile opens it as the GeoJSON it becomes, the map editor works
/// on it like any other GeoJSON, and a save turns it back into the file set.
#[tokio::test]
async fn the_editor_opens_a_shapefile_as_geojson_and_saves_it_back() {
    use crate::geo::geojson::Shape;
    use crate::geo::shapefile::{dbf, shp};

    let dir = temp_dir("shapefile");
    let stem = dir.join("cities");
    let shapes = vec![Shape::Point([16.3738, 48.2082]), Shape::Point([-0.1276, 51.5072])];
    let (shp_b, shx_b) = shp::write(shp::ShapeType::Point, &shapes);
    std::fs::write(stem.with_extension("shp"), &shp_b).unwrap();
    std::fs::write(stem.with_extension("shx"), &shx_b).unwrap();
    let table = dbf::Table {
        fields: vec![dbf::Field { name: "NAME".into(), kind: b'C', len: 10, decimals: 0 }],
        rows: vec![vec!["Vienna".into()], vec!["London".into()]],
    };
    std::fs::write(stem.with_extension("dbf"), dbf::write(&table)).unwrap();
    let file = stem.with_extension("shp");

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.open_path_in_editor(file.clone()).await;

    let ed = st.editor.as_ref().expect("the editor opened");
    assert!(ed.is_shapefile(), "a .shp opens as the shapefile it is");
    let text = ed.contents();
    assert!(text.contains("FeatureCollection"), "as GeoJSON: {text}");
    assert!(text.contains("Vienna") && text.contains("London"), "with its attributes");

    // The map dialog reads it exactly as it reads any other GeoJSON.
    let doc = crate::geo::geojson::extract(&text);
    assert_eq!(doc.objects.len(), 1, "one collection");
    assert_eq!(doc.objects[0].features.len(), 2, "two features");
    assert_eq!(doc.skipped, 0, "nothing dropped as out of range");

    // Rename one, the way an edit on the map reaches the buffer, and save.
    let edited = text.replace("London", "Londinium");
    st.editor.as_mut().unwrap().apply_map_edit(crate::geo::edit::TextEdit::Replace {
        start: 0,
        end: text.len(),
        text: edited,
    });
    st.save_editor(false).await;

    let back = dbf::read(&std::fs::read(stem.with_extension("dbf")).unwrap()).unwrap();
    assert_eq!(back.rows.len(), 2, "both records are still there");
    assert!(
        back.rows.iter().any(|r| r[0] == "Londinium"),
        "the edit reached the .dbf: {:?}",
        back.rows
    );
    // And the geometry survived the round trip.
    let (kind, after) = shp::read(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(kind, shp::ShapeType::Point);
    assert_eq!(after.len(), 2);

    let _ = std::fs::remove_dir_all(&dir);
}

/// F4 on an audio file opens the editor on its tags rather than on nonsense
/// text or a haystack of bytes, an edit saves back into the file, and Alt-T
/// still reaches the bytes underneath.
#[tokio::test]
async fn the_editor_opens_an_audio_file_on_its_tags_and_saves_them() {
    use lofty::config::WriteOptions;
    use lofty::prelude::{Accessor, TagExt};
    use lofty::tag::{Tag, TagType};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let dir = temp_dir("tagedit");
    let file = dir.join("song.wav");
    crate::audio::tests::write_wav(&file, 8_000, 0.05, 440.0, 0.2);
    let mut tag = Tag::new(TagType::Id3v2);
    tag.set_title("Before".to_string());
    // Something outside the well-known set, so the "other tags" rows are there.
    tag.insert_text(lofty::tag::ItemKey::Mood, "Rainy".to_string());
    tag.save_to_path(&file, WriteOptions::default()).unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.open_path_in_editor(file.clone()).await;

    let ed = st.editor.as_ref().expect("the editor opened");
    assert!(ed.tags_active(), "an audio file opens on its tags");
    // Nothing else is loaded: a hex view with a binary template panel over it
    // is not what F4 on a song should mean.
    assert!(!ed.is_hex(), "and on nothing else");

    // What is actually on screen — asserting the state alone once let a render
    // that drew the hex editor instead go unnoticed.
    let drawn = |st: &mut AppState| {
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        t.draw(|f| crate::ui::draw(f, st)).unwrap();
        let b = t.backend().buffer().clone();
        (0..b.area.height)
            .map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let screen = drawn(&mut st);
    assert!(screen.contains("Title"), "the tag page is drawn: {screen}");
    assert!(screen.contains("Before"), "with the file's own values");
    assert!(screen.contains("Artist") && screen.contains("Album"));

    // The title is the first row: replace it.
    let press = |st: &mut AppState, c: KeyCode, m: KeyModifiers| {
        let ed = st.editor.as_mut().unwrap();
        ed.handle_key(KeyEvent::new(c, m));
    };
    press(&mut st, KeyCode::Enter, KeyModifiers::NONE);
    for _ in 0.."Before".len() {
        press(&mut st, KeyCode::Backspace, KeyModifiers::NONE);
    }
    for c in "After".chars() {
        press(&mut st, KeyCode::Char(c), KeyModifiers::NONE);
    }
    press(&mut st, KeyCode::Enter, KeyModifiers::NONE);
    assert!(st.editor.as_ref().unwrap().tags_dirty(), "the change is noticed");

    // F2 writes it through the tag writer.
    st.save_editor(false).await;
    assert!(st.dialog.is_none(), "the save raised no error: {:?}", st.dialog.is_some());
    let on_disk = crate::tags::read(&file).expect("still readable after the write");
    let title = on_disk.fields.iter().find(|(f, _)| *f == crate::tags::Field::Title).unwrap();
    assert_eq!(title.1, "After", "the edit reached the file");
    assert!(!st.editor.as_ref().unwrap().tags_dirty(), "and the editor has settled");

    // The unnamed items are editable too: walk down to Mood and change it.
    for _ in 0..12 {
        press(&mut st, KeyCode::Down, KeyModifiers::NONE);
    }
    assert!(drawn(&mut st).contains("Mood"), "the unnamed items are listed");
    press(&mut st, KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        st.editor.as_ref().unwrap().tags.as_ref().unwrap().editing(),
        "an unnamed item can be typed into, not only the named fields"
    );
    for _ in 0.."Rainy".len() {
        press(&mut st, KeyCode::Backspace, KeyModifiers::NONE);
    }
    for c in "Sunny".chars() {
        press(&mut st, KeyCode::Char(c), KeyModifiers::NONE);
    }
    press(&mut st, KeyCode::Enter, KeyModifiers::NONE);
    st.save_editor(false).await;
    assert!(
        crate::tags::read(&file).unwrap().extra.iter().any(|e| e.value == "Sunny"),
        "and the change reaches the file"
    );
    // Back to the top for the key tests below.
    press(&mut st, KeyCode::Home, KeyModifiers::NONE);

    // F5 adds a tag the file does not have: the picker offers what this tag
    // format can hold, filtered as you type.
    // Through the app, not straight at the editor: opening a dialog is what
    // the editor's signal asks the app to do.
    st.handle_key(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE)).await;
    assert!(matches!(st.dialog, Some(Dialog::TagKey(_))), "F5 opens the key picker");
    for c in "bpm".chars() {
        st.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)).await;
    }
    let picker = drawn(&mut st);
    // ID3v2 spells its BPM key `IntegerBpm`, and that is what the format
    // supports — which is the point of offering only the keys it can hold.
    assert!(picker.contains("IntegerBpm"), "the query narrows the list: {picker}");
    st.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
    assert!(st.dialog.is_none(), "picking closes the picker");

    // The new row is there and ready to be typed into.
    let te = st.editor.as_ref().unwrap().tags.as_ref().unwrap();
    assert!(te.editing(), "the new tag is waiting for its value");
    for c in "92".chars() {
        press(&mut st, KeyCode::Char(c), KeyModifiers::NONE);
    }
    press(&mut st, KeyCode::Enter, KeyModifiers::NONE);
    st.save_editor(false).await;
    assert!(
        crate::tags::read(&file)
            .unwrap()
            .extra
            .iter()
            .any(|e| e.key == lofty::tag::ItemKey::IntegerBpm && e.value == "92"),
        "an added tag reaches the file under the key that was picked"
    );
    press(&mut st, KeyCode::Home, KeyModifiers::NONE);

    // The page's own F-keys: F3 for the bytes, F4 to edit, F8 to clear.
    press(&mut st, KeyCode::F(4), KeyModifiers::NONE);
    assert!(
        st.editor.as_ref().unwrap().tags.as_ref().unwrap().editing(),
        "F4 starts editing the selected tag"
    );
    press(&mut st, KeyCode::Esc, KeyModifiers::NONE);
    press(&mut st, KeyCode::F(3), KeyModifiers::NONE);
    assert!(!st.editor.as_ref().unwrap().tags_active(), "F3 shows the bytes");
    press(&mut st, KeyCode::F(3), KeyModifiers::NONE);
    assert!(st.editor.as_ref().unwrap().tags_active(), "and F3 again the tags");

    // Ctrl-F9 means the same thing here: there is no text to go back to.
    press(&mut st, KeyCode::F(9), KeyModifiers::CONTROL);
    assert!(!st.editor.as_ref().unwrap().tags_active(), "Ctrl-F9 shows the bytes");
    press(&mut st, KeyCode::F(9), KeyModifiers::CONTROL);
    assert!(st.editor.as_ref().unwrap().tags_active(), "and never loads it as text");
    assert_eq!(
        st.editor.as_ref().unwrap().contents(),
        "",
        "the buffer stays empty: an MP3 is never read as text"
    );

    // Alt-T opens the bytes on demand, and brings the tags back.
    press(&mut st, KeyCode::Char('t'), KeyModifiers::ALT);
    assert!(!st.editor.as_ref().unwrap().tags_active(), "Alt-T shows the bytes");
    assert!(st.editor.as_ref().unwrap().is_hex(), "opening them for the first time");
    let bytes_screen = drawn(&mut st);
    assert!(!bytes_screen.contains("Album artist"), "the tag page is no longer drawn");

    press(&mut st, KeyCode::Char('t'), KeyModifiers::ALT);
    assert!(st.editor.as_ref().unwrap().tags_active(), "and Alt-T again brings the tags back");
    assert!(drawn(&mut st).contains("Composer"), "drawing the tag page once more");

    let _ = std::fs::remove_dir_all(&dir);
}

/// with no GeoJSON in it says so instead.
#[tokio::test]
async fn the_geojson_map_opens_from_the_editor_and_goes_back_to_the_text() {
    use ratatui::crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    let dir = temp_dir("geomap");
    let file = dir.join("reply.json");
    let text =
        "{\"ok\": true,\n \"where\": {\"type\": \"Point\", \"coordinates\": [16.37, 48.21]}}\n";
    std::fs::write(&file, text).unwrap();

    let (tx, mut rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.open_path_in_editor(file).await;
    // Where the frame puts the hardware cursor: a frame that places none
    // leaves it where it was, off in the corner.
    let caret = |st: &mut AppState| {
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        ratatui::backend::Backend::set_cursor_position(t.backend_mut(), (99, 29)).unwrap();
        t.draw(|f| crate::ui::draw(f, st)).unwrap();
        t.get_cursor_position().unwrap()
    };
    assert_ne!(caret(&mut st), (99, 29).into(), "the editor places its caret");
    let alt_m = KeyEvent::new(KeyCode::Char('m'), KeyModifiers::ALT);
    let signal = st.editor.as_mut().unwrap().handle_key(alt_m);
    st.apply_editor_signal(signal).await;
    assert!(
        matches!(st.dialog, Some(Dialog::GeoMap(_))),
        "the dialog is up while the file is read"
    );
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("read")
            .expect("open");
        let done = matches!(ev, AppEvent::GeoJsonRead { .. });
        st.apply_event(ev).await;
        if done {
            break;
        }
    }
    assert!(matches!(st.dialog, Some(Dialog::GeoMap(_))));
    assert_eq!(caret(&mut st), (99, 29).into(), "no editor caret blinks through the map");
    // A drag over it is folded like an orbit while it pans.
    let at = |kind| MouseEvent { kind, column: 40, row: 12, modifiers: KeyModifiers::NONE };
    st.last_area = ratatui::layout::Rect::new(0, 0, 100, 30);
    if let Some(Dialog::GeoMap(d)) = st.dialog.as_mut() {
        let theme = crate::ui::theme::Theme::mc();
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        t.draw(|f| d.render(f, f.area(), &theme, None)).unwrap();
    }
    st.handle_mouse(at(MouseEventKind::Down(MouseButton::Left))).await;
    assert!(st.mouse_folds(&at(MouseEventKind::Drag(MouseButton::Left))));
    st.handle_mouse(at(MouseEventKind::Up(MouseButton::Left))).await;
    // Enter goes to the point in the text.
    st.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
    assert!(st.dialog.is_none());
    assert_eq!(
        st.editor.as_ref().unwrap().cursor_line_col(),
        (1, 10),
        "on the point object's brace"
    );

    // A file without GeoJSON opens the map all the same, to draw on.
    let plain = dir.join("plain.json");
    std::fs::write(&plain, "{\"a\": [1, 2]}").unwrap();
    st.editor = None;
    st.open_path_in_editor(plain).await;
    st.apply_editor_signal(EditorSignal::OpenGeoMap).await;
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("read")
            .expect("open");
        let done = matches!(ev, AppEvent::GeoJsonRead { .. });
        st.apply_event(ev).await;
        if done {
            break;
        }
    }
    assert!(matches!(st.dialog, Some(Dialog::GeoMap(_))));
    std::fs::remove_dir_all(&dir).ok();
}

/// GeoJSON made from nothing: the map opened on an empty file draws a point
/// into the editor's text, which becomes a FeatureCollection, and Ctrl-Z on
/// the map takes it back out of the editor again.
#[tokio::test]
async fn features_drawn_on_the_map_are_written_into_the_editor() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let dir = temp_dir("geoedit");
    let file = dir.join("new.geojson");
    std::fs::write(&file, "").unwrap();

    let (tx, mut rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.open_path_in_editor(file).await;
    st.apply_editor_signal(EditorSignal::OpenGeoMap).await;
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("read")
            .expect("open");
        let done = matches!(ev, AppEvent::GeoJsonRead { .. });
        st.apply_event(ev).await;
        if done {
            break;
        }
    }
    if let Some(Dialog::GeoMap(d)) = st.dialog.as_mut() {
        let theme = crate::ui::theme::Theme::mc();
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        t.draw(|f| d.render(f, f.area(), &theme, None)).unwrap();
    }
    let press = |code| KeyEvent::new(code, KeyModifiers::NONE);
    // A point at the crosshair, from the keyboard.
    st.handle_key(press(KeyCode::Char('1'))).await;
    st.handle_key(press(KeyCode::Char(' '))).await;
    let ed = st.editor.as_ref().unwrap();
    assert!(ed.dirty);
    let text = ed.contents();
    assert!(text.starts_with("{\n  \"type\": \"FeatureCollection\",\n"), "{text}");
    let doc = crate::geo::geojson::extract(&text);
    assert_eq!(doc.objects[0].features.len(), 1);
    assert_eq!(doc.errors, 0);
    // Undone on the map, undone in the editor.
    st.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL)).await;
    assert_eq!(st.editor.as_ref().unwrap().contents(), "");
    st.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL)).await;
    assert_eq!(st.editor.as_ref().unwrap().contents(), text);
    // Closing the map keeps the edit, unsaved, in the editor.
    st.handle_key(press(KeyCode::F(10))).await;
    assert!(st.dialog.is_none());
    assert_eq!(st.editor.as_ref().unwrap().contents(), text);
    std::fs::remove_dir_all(&dir).ok();
}

/// The editor options dialog round-trips: what it submits reaches the open
/// editor *and* the config, so the next file opens with the same settings.
#[tokio::test]
async fn editor_options_reach_the_editor_and_the_config() {
    let dir = temp_dir("edopts");
    let file = dir.join("f.txt");
    std::fs::write(&file, b"text").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.open_path_in_editor(file.clone()).await;
    assert!(!st.editor.as_ref().unwrap().wrap(), "wrap is off by default");

    st.apply_editor_signal(EditorSignal::OpenOptions).await;
    assert!(matches!(st.dialog, Some(Dialog::Form(_))), "the options form is up");

    let opts = crate::config::EditorOptions {
        wrap_mode: crate::config::WrapMode::Dynamic,
        tab_spacing: 2,
        confirm_before_saving: false,
        ..crate::config::EditorOptions::default()
    };
    st.handle_submit(Submit::EditorOptions(Box::new(opts.clone()))).await;
    let ed = st.editor.as_ref().unwrap();
    assert!(ed.wrap(), "dynamic paragraphing turns the display wrap on");
    assert!(!ed.confirm_before_saving());
    assert_eq!(st.config.editor_options, opts, "and the config keeps them");

    // With "confirm before saving" off, F2 writes straight away. (The real
    // dialog path clears `dialog` before handing the submit over; do the same.)
    st.dialog = None;
    st.apply_editor_signal(EditorSignal::Save { close_after: false }).await;
    assert!(st.dialog.is_none(), "no confirmation was raised");
    assert!(!st.editor.as_ref().unwrap().dirty);

    // A file opened afterwards starts from the saved options.
    st.editor = None;
    st.open_path_in_editor(file).await;
    assert!(st.editor.as_ref().unwrap().wrap(), "the new editor picked the options up");

    std::fs::remove_dir_all(&dir).ok();
}

/// File → Insert file / Copy to file / Open, as the browser delivers them.
#[tokio::test]
async fn editor_browse_actions_insert_write_and_open_files() {
    use crate::editor::BrowseKind;
    let dir = temp_dir("edbrowse");
    let main = dir.join("main.txt");
    let other = dir.join("other.txt");
    std::fs::write(&main, b"AB").unwrap();
    std::fs::write(&other, b"XY").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.open_path_in_editor(main.clone()).await;

    // Insert at the cursor (which starts at the top of the buffer).
    st.editor_browsed(BrowseKind::Insert, other.clone()).await;
    assert_eq!(st.editor.as_ref().unwrap().contents(), "XYAB");

    // Copy to file writes the marked block, or — as here — the whole buffer.
    let out = dir.join("out.txt");
    st.editor_browsed(BrowseKind::CopyTo, out.clone()).await;
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "XYAB");

    // Open refuses to discard unsaved work.
    st.editor_browsed(BrowseKind::Open, other.clone()).await;
    assert!(matches!(st.dialog, Some(Dialog::Message(_))), "unsaved changes are flagged");
    assert_eq!(st.editor.as_ref().unwrap().contents(), "XYAB", "the buffer is untouched");

    // Saved, it opens the chosen file in place.
    st.dialog = None;
    st.save_editor(false).await;
    st.editor_browsed(BrowseKind::Open, other).await;
    let ed = st.editor.as_ref().unwrap();
    assert_eq!(ed.contents(), "XY");
    assert_eq!(ed.name, "other.txt");

    std::fs::remove_dir_all(&dir).ok();
}

/// Format → "Paste output of…" runs the command and inserts what it printed.
#[tokio::test]
async fn editor_pastes_a_commands_output() {
    let dir = temp_dir("edpaste");
    let file = dir.join("f.txt");
    std::fs::write(&file, b"").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.open_path_in_editor(file).await;

    st.apply_editor_signal(EditorSignal::OpenPasteOutput).await;
    assert!(matches!(st.dialog, Some(Dialog::Input(_))), "the command prompt is up");
    st.dialog = None;

    st.editor_paste_output("echo rc-paste-marker".to_string()).await;
    assert!(
        st.editor.as_ref().unwrap().contents().contains("rc-paste-marker"),
        "the command's stdout landed in the buffer"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// Format → Sort, as the options dialog delivers it.
#[tokio::test]
async fn editor_sort_submit_sorts_the_buffer() {
    let dir = temp_dir("edsort");
    let file = dir.join("f.txt");
    std::fs::write(&file, b"c\na\nb").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.open_path_in_editor(file).await;

    st.apply_editor_signal(EditorSignal::OpenSortBlock).await;
    assert!(matches!(st.dialog, Some(Dialog::Form(_))));
    st.handle_submit(Submit::EditorSort { reverse: true, ignore_case: false, unique: false }).await;
    assert_eq!(st.editor.as_ref().unwrap().contents(), "c\nb\na");

    std::fs::remove_dir_all(&dir).ok();
}

/// Ctrl-L asks the run loop to repaint; About and Save setup are handled too.
#[tokio::test]
async fn editor_refresh_about_and_save_setup() {
    let dir = temp_dir("edmisc");
    let file = dir.join("f.txt");
    std::fs::write(&file, b"x").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.open_path_in_editor(file).await;

    assert!(!st.force_clear);
    st.apply_editor_signal(EditorSignal::RefreshScreen).await;
    assert!(st.force_clear, "the next frame is repainted from scratch");

    st.apply_editor_signal(EditorSignal::About).await;
    match st.dialog.as_ref() {
        Some(Dialog::Message(m)) => {
            assert!(m.message.contains(env!("CARGO_PKG_VERSION")), "About names the version");
            assert!(!m.is_error);
        }
        _ => panic!("About should raise a message box"),
    }
    st.dialog = None;

    // Save setup copies the editor's current options into the config.
    let dark = st.dark_ui();
    let opts = crate::config::EditorOptions { visible_tabs: true, ..Default::default() };
    st.editor.as_mut().unwrap().set_options(opts.clone(), dark);
    st.apply_editor_signal(EditorSignal::SaveSetup).await;
    assert_eq!(st.config.editor_options, opts);
    assert!(st.dialog.is_none(), "a successful save reports on the footer, not in a dialog");

    std::fs::remove_dir_all(&dir).ok();
}

/// While the editor's F9 menu is open, Esc closes the menu instead of being
/// held as a Midnight-Commander function-key prefix.
#[tokio::test]
async fn esc_closes_the_editor_menu_rather_than_arming_a_prefix() {
    let dir = temp_dir("edesc");
    let file = dir.join("f.txt");
    std::fs::write(&file, b"x").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.open_path_in_editor(file).await;

    st.handle_key(KeyEvent::new(KeyCode::F(9), KeyModifiers::NONE)).await;
    assert!(st.editor.as_ref().unwrap().menu_open());
    st.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).await;
    assert!(!st.editor.as_ref().unwrap().menu_open(), "Esc closed the menu");
    assert!(st.pending_esc.is_none(), "and was not held as a key prefix");
    assert!(st.editor.is_some(), "nor did it reach the editor as a quit request");

    std::fs::remove_dir_all(&dir).ok();
}

/// Del on a file the disk explorer picked out inside a box confirms, deletes it,
/// and folds the loss into the treemap in place — no rescan needed (issue #14).
#[tokio::test]
async fn disk_explorer_deletes_the_file_under_the_cursor() {
    use crate::disk::{DiskSignal, DiskView};

    let dir = temp_dir("dskdel");
    let sub = dir.join("cache");
    std::fs::create_dir_all(&sub).unwrap();
    let file = sub.join("huge.bin");
    std::fs::write(&file, vec![0u8; 64]).unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    let mut dv = DiskView::new(dir.clone());
    dv.scanning = false;
    dv.entries = crate::disk::scan_dir(&dir);
    st.diskview = Some(dv);
    // Draw once so the renderer records where the box's file rows landed — the
    // cursor only steps onto files that are really on screen.
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    term.draw(|f| crate::ui::draw(f, &mut st)).unwrap();
    let before = st.diskview.as_ref().unwrap().entries[0].size;
    assert!(!st.diskview.as_ref().unwrap().file_rects.is_empty(), "file rows drawn");

    // Tab moves the cursor into the file list, then Del raises the confirmation.
    st.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)).await;
    assert_eq!(st.diskview.as_ref().unwrap().on_file(), Some(0));
    let sig = st.diskview.as_mut().unwrap().handle_key(key_del());
    assert!(matches!(sig, DiskSignal::DeleteFile { .. }));
    st.apply_disk_signal(sig).await;
    assert!(st.dialog.is_some(), "deleting asks first");

    // Confirming removes it from disk and from the box, right away.
    st.handle_dialog_result(DialogResult::Submit(Submit::DeleteDiskFile(file.clone()))).await;
    assert!(!file.exists(), "the file is gone");
    let dv = st.diskview.as_ref().unwrap();
    assert!(dv.entries[0].files.is_empty(), "and off the box's list at once");
    assert!(dv.entries[0].size < before, "the box shrank by the file's size");
    assert!(!dv.scanning, "without re-walking the subtree");

    std::fs::remove_dir_all(&dir).ok();
}

fn key_del() -> KeyEvent {
    KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)
}

/// Temp directory unique to this test run, matching the inline idiom used
/// throughout this file.
fn lastdir_tmp(tag: &str) -> std::path::PathBuf {
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("rc_{tag}_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `rc --print-last-dir` has to hand the shell a real local directory. The two
/// awkward cases are a panel sitting inside an archive (a valid cwd, but not one
/// a shell can enter) and a remote panel (whose path isn't local at all).
#[tokio::test]
async fn last_dir_for_shell_falls_back_from_archive_and_remote() {
    let root = lastdir_tmp("lastdir");
    let inner = root.join("holder");
    std::fs::create_dir_all(&inner).unwrap();
    let zip = inner.join("a.zip");
    std::fs::write(&zip, b"not really a zip").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;

    // Plain local: handed back as-is.
    st.panels[0].cwd = VfsPath::local(&inner);
    assert_eq!(st.last_dir_for_shell(), inner);

    // Inside an archive: the directory *holding* the archive, not the archive
    // path and not a path inside it.
    st.panels[0].cwd =
        VfsPath { scheme: "archive".into(), path: "/sub".into(), container: Some(zip.clone()) };
    assert_eq!(st.last_dir_for_shell(), inner);

    // Remote: the local directory that panel last showed.
    st.last_local_cwd[0] = VfsPath::local(&root);
    setup_remote_panel(&mut st, 0, "sftp-lastdir", "/var/www");
    assert_eq!(st.last_dir_for_shell(), root);

    std::fs::remove_dir_all(&root).ok();
}

/// A cwd that no longer exists must not produce a path the shell would reject.
#[tokio::test]
async fn last_dir_for_shell_degrades_to_the_process_cwd() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local("/definitely/not/a/real/directory/here");
    assert_eq!(st.last_dir_for_shell(), std::env::current_dir().unwrap());
}

/// The file the shell wrapper reads must hold the directory plus one newline.
#[tokio::test]
async fn write_last_dir_writes_the_directory_with_a_trailing_newline() {
    let root = lastdir_tmp("lastdir_write");
    let out = root.join("out");

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.write_last_dir(&out);

    let written = std::fs::read(&out).unwrap();
    assert_eq!(written, format!("{}\n", root.display()).into_bytes());
    std::fs::remove_dir_all(&root).ok();
}

/// F8 goes through the trash and Shift-F8 deletes outright — and the two must
/// raise visibly different prompts, since only one of them is recoverable.
#[tokio::test]
async fn f8_trashes_but_shift_f8_deletes_permanently() {
    let root = lastdir_tmp("trashkeys");
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let _guard = crate::trash::test_home::TempHome::set(&home);
    std::fs::write(root.join("a.txt"), b"x").unwrap();
    std::fs::write(root.join("b.txt"), b"y").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.config.use_trash = true;
    st.config.confirm_delete = true;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    // Put the cursor on a real file: on ".." there is nothing to delete.
    st.panels[0].cursor =
        st.panels[0].entries.iter().position(|e| e.name == "a.txt").expect("a.txt listed");

    // Plain F8: the recoverable prompt, submitting a Trash op.
    st.handle_key(KeyEvent::from(KeyCode::F(8))).await;
    match &st.dialog {
        Some(Dialog::Confirm(c)) => {
            assert!(c.message.contains("Trash"), "trash wording: {}", c.message);
            assert!(!c.danger, "trashing is recoverable, so not a danger prompt");
        }
        _ => panic!("expected a confirm dialog"),
    }
    st.dialog = None;

    // Shift-F8: the permanent prompt, visibly marked as dangerous.
    st.handle_key(KeyEvent::new(KeyCode::F(8), KeyModifiers::SHIFT)).await;
    match &st.dialog {
        Some(Dialog::Confirm(c)) => {
            assert!(c.message.contains("Permanently"), "permanent wording: {}", c.message);
            assert!(c.danger, "an irreversible delete is a danger prompt");
        }
        _ => panic!("expected a confirm dialog"),
    }
    st.dialog = None;

    // With the trash switched off, F8 is the ordinary permanent delete again.
    st.config.use_trash = false;
    st.handle_key(KeyEvent::from(KeyCode::F(8))).await;
    match &st.dialog {
        Some(Dialog::Confirm(c)) => {
            assert!(c.message.starts_with("Delete"), "plain wording: {}", c.message);
        }
        _ => panic!("expected a confirm dialog"),
    }

    std::fs::remove_dir_all(&root).ok();
}

/// There is nowhere to trash a file to on a remote server, so F8 there must keep
/// deleting outright rather than implying it can be undone.
#[tokio::test]
async fn f8_on_a_remote_panel_still_deletes_permanently() {
    let root = lastdir_tmp("trashremote");
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let _guard = crate::trash::test_home::TempHome::set(&home);

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.config.use_trash = true;
    st.config.confirm_delete = true;
    setup_remote_panel(&mut st, 0, "sftp-trash", "/var/www");
    st.panels[0].reload().await.ok();

    if st.panels[0].operation_targets().is_empty() {
        // The stub backend listed nothing; nothing to assert about.
        std::fs::remove_dir_all(&root).ok();
        return;
    }
    st.handle_key(KeyEvent::from(KeyCode::F(8))).await;
    match &st.dialog {
        Some(Dialog::Confirm(c)) => {
            assert!(
                !c.message.contains("Trash"),
                "a remote delete must not claim to use the trash: {}",
                c.message
            );
        }
        _ => panic!("expected a confirm dialog"),
    }
    std::fs::remove_dir_all(&root).ok();
}

/// A new tab opens on the same directory, and switching back restores the
/// position (cursor, filter, view) the other tab was left in.
#[tokio::test]
async fn tabs_open_close_and_restore_their_position() {
    let root = lastdir_tmp("tabs");
    std::fs::create_dir_all(root.join("alpha")).unwrap();
    std::fs::create_dir_all(root.join("beta")).unwrap();
    for n in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(root.join(n), b"x").unwrap();
    }

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    assert_eq!(st.panels[0].tabs.len(), 1, "one tab to start with");

    // Set a distinctive position in tab 0. The filter goes on first and the
    // listing is reloaded, as the real filter path does, so the cursor lands on
    // an entry that is actually visible.
    st.panels[0].filter = Some("*.txt".into());
    st.panels[0].reload().await.unwrap();
    st.panels[0].cursor =
        st.panels[0].entries.iter().position(|e| e.name == "b.txt").expect("b.txt visible");
    let expected_cursor = st.panels[0].cursor;

    // Open a second tab: same directory, fresh position.
    st.tab_new(0).await;
    assert_eq!(st.panels[0].tabs.len(), 2);
    assert_eq!(st.panels[0].tab, 1, "the new tab is the one in front");
    assert_eq!(st.panels[0].cwd, VfsPath::local(&root));
    assert!(st.panels[0].filter.is_none(), "a new tab starts unfiltered");

    // Move the new tab somewhere else, then go back to the first.
    st.panels[0].cwd = VfsPath::local(root.join("alpha"));
    st.panels[0].reload().await.unwrap();
    st.tab_select(0, 0).await;

    assert_eq!(st.panels[0].tab, 0);
    assert_eq!(st.panels[0].cwd, VfsPath::local(&root), "back where tab 0 was");
    assert_eq!(st.panels[0].cursor, expected_cursor, "its cursor came back");
    assert_eq!(
        st.panels[0].current_entry().map(|e| e.name.as_str()),
        Some("b.txt"),
        "and on the same file, not just the same index"
    );
    assert_eq!(st.panels[0].filter.as_deref(), Some("*.txt"), "and its filter");

    // And the second tab still remembers where *it* got to.
    st.tab_select(0, 1).await;
    assert_eq!(st.panels[0].cwd, VfsPath::local(root.join("alpha")));

    // Closing it returns to the remaining tab.
    st.tab_close(0, 1).await;
    assert_eq!(st.panels[0].tabs.len(), 1);
    assert_eq!(st.panels[0].tab, 0);
    assert_eq!(st.panels[0].cwd, VfsPath::local(&root));

    std::fs::remove_dir_all(&root).ok();
}

/// A panel must always show something, so closing the only tab does nothing.
#[tokio::test]
async fn closing_the_last_tab_is_a_no_op() {
    let root = lastdir_tmp("tabs_last");
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();

    st.tab_close(0, 0).await;
    assert_eq!(st.panels[0].tabs.len(), 1, "the panel still has its tab");
    assert!(!st.pending_quit, "and closing a tab never quits");

    std::fs::remove_dir_all(&root).ok();
}

/// The one-remote-panel invariant applies to tab switches too: a tab sitting on
/// a remote connection must not become a second remote panel.
#[tokio::test]
async fn switching_to_a_remote_tab_respects_the_one_remote_rule() {
    let root = lastdir_tmp("tabs_remote");
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();

    // Register a remote scheme, then give panel 0 a second tab parked on it
    // while panel 0 itself stays local.
    setup_remote_panel(&mut st, 0, "sftp-tabs", "/var/www");
    let remote_cwd = st.panels[0].cwd.clone();
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].sync_active_tab();
    st.panels[0].tabs.push(crate::panel::tabs::TabState::new(
        remote_cwd,
        st.panels[0].format,
        st.panels[0].sort,
    ));

    // With the other panel local, switching to the remote tab is allowed.
    st.tab_select(0, 1).await;
    assert!(st.panels[0].cwd.is_remote(), "the remote tab opened fine on its own");

    // Go back, then make the *other* panel remote so the rule now bites.
    st.tab_select(0, 0).await;
    setup_remote_panel(&mut st, 1, "sftp-tabs2", "/srv");

    st.tab_select(0, 1).await;
    assert!(!st.panels[0].cwd.is_remote(), "the switch was refused, so panel 0 stayed local");
    assert!(st.dialog.is_some(), "and it said why");

    std::fs::remove_dir_all(&root).ok();
}

/// Both tab-cycling chords, driven through the real key path. Ctrl-Tab only
/// arrives on terminals that can encode it *and* don't grab it for their own
/// tabs, which is why Ctrl-PageUp/PageDown exist — a failure here is our
/// handling, not the terminal's.
#[tokio::test]
async fn ctrl_tab_and_ctrl_pageup_down_cycle_tabs() {
    let root = lastdir_tmp("ctrltab");
    std::fs::create_dir_all(root.join("alpha")).unwrap();
    std::fs::create_dir_all(root.join("beta")).unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();

    // Two tabs on different directories.
    st.tab_new(0).await;
    st.panels[0].cwd = VfsPath::local(root.join("alpha"));
    st.panels[0].reload().await.unwrap();
    st.panels[0].sync_active_tab();
    assert_eq!(st.panels[0].tabs.len(), 2);
    assert_eq!(st.panels[0].tab, 1);

    // Ctrl-Tab wraps forward to tab 0.
    st.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::CONTROL)).await;
    assert_eq!(st.panels[0].tab, 0, "Ctrl-Tab moved to the next tab");
    assert_eq!(st.panels[0].cwd, VfsPath::local(&root));

    // And again, back to tab 1.
    st.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::CONTROL)).await;
    assert_eq!(st.panels[0].tab, 1, "and wrapped round again");

    // Ctrl-Shift-Tab goes the other way.
    st.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::CONTROL | KeyModifiers::SHIFT))
        .await;
    assert_eq!(st.panels[0].tab, 0, "Ctrl-Shift-Tab moved back");

    // Ctrl-PageDown / Ctrl-PageUp do the same job, and are the reliable route:
    // terminals grab Ctrl-Tab for their own tabs far more often than not.
    st.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::CONTROL)).await;
    assert_eq!(st.panels[0].tab, 1, "Ctrl-PageDown moved to the next tab");
    st.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::CONTROL)).await;
    assert_eq!(st.panels[0].tab, 0, "Ctrl-PageUp moved back");

    // Unmodified PageUp/PageDown must still scroll the listing, not switch tabs.
    let before_tab = st.panels[0].tab;
    st.handle_key(KeyEvent::from(KeyCode::PageDown)).await;
    assert_eq!(st.panels[0].tab, before_tab, "plain PageDown does not switch tabs");

    // Plain Tab must still switch panels, not tabs.
    let before = st.panels[0].tab;
    st.handle_key(KeyEvent::from(KeyCode::Tab)).await;
    assert_eq!(st.active, 1, "plain Tab still flips panel focus");
    assert_eq!(st.panels[0].tab, before, "and does not touch the tabs");

    std::fs::remove_dir_all(&root).ok();
}

/// File → Receive over LAN opens a QR dialog serving an upload page for the
/// active panel's directory; a file sent to it lands there, and closing the
/// dialog takes the server down.
#[tokio::test]
async fn receive_over_lan_saves_into_the_panel_directory_until_closed() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let root = temp_dir("receive_menu");
    let (tx, mut rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.config.auto_refresh = false;
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();

    st.run_menu_action(MenuAction::ReceiveFiles).await;
    let Some(Dialog::Receive(d)) = &st.dialog else { panic!("the Receive dialog opens") };
    // http://<ip>:<port>/<token>/ — talk to it on loopback.
    let rest = d.url.trim_start_matches("http://");
    let (host_port, path) = rest.split_once('/').unwrap();
    let port: u16 = host_port.rsplit(':').next().unwrap().parse().unwrap();
    let token = path.trim_end_matches('/');
    assert_eq!(token.len(), 32, "an unguessable token");

    let mut sock = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let req =
        format!("PUT /{token}/upload?name=note.txt HTTP/1.1\r\nContent-Length: 5\r\n\r\nhello");
    sock.write_all(req.as_bytes()).await.unwrap();
    let mut resp = String::new();
    sock.read_to_string(&mut resp).await.unwrap();
    assert!(resp.starts_with("HTTP/1.1 201"), "{resp}");
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
        let done = matches!(ev, AppEvent::FileReceived { .. });
        st.apply_event(ev).await;
        if done {
            break;
        }
    }
    assert!(matches!(&st.dialog, Some(Dialog::Receive(d)) if d.received == 1));
    assert!(
        st.panels[0].entries.iter().any(|e| e.name == "note.txt"),
        "without auto-refresh the panel is re-read for it"
    );

    st.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).await;
    assert!(st.dialog.is_none() && st.receive_server.is_none(), "closing stops the server");
    // The aborted accept loop lets go of its socket once the runtime next gets a
    // turn, so give it a moment rather than expect it the same instant.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut refused = false;
    while Instant::now() < deadline {
        if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_err() {
            refused = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(refused, "nothing listens any more");
    let _ = std::fs::remove_dir_all(&root);
}

/// The screensaver's idle timer only runs when it is turned on, counting from
/// the last key press or mouse movement.
#[tokio::test]
async fn the_screensaver_is_armed_only_when_turned_on() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    assert!(!st.saver_armed(), "off by default");
    st.config.screensaver_minutes = 5;
    assert!(st.saver_armed());
    assert_eq!(st.saver_deadline() - st.last_input, Duration::from_secs(300));
    st.start_saver();
    assert!(!st.saver_armed(), "not while it is up");
    assert!(st.wants_ticks(), "it animates on the tick");
    assert!(st.force_clear, "pictures behind it are wiped");

    // Moving the mouse takes it down and restarts the count.
    let before = st.last_input;
    st.handle_mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column: 3,
        row: 3,
        modifiers: KeyModifiers::NONE,
    })
    .await;
    assert!(st.saver.is_none() && st.last_input >= before);
}

/// The key that wakes the screen does nothing else: over a running copy's
/// progress dialog, an Esc must not become "abort", nor be held as the first
/// half of an Esc-digit function key.
#[tokio::test]
async fn a_key_over_the_screensaver_only_takes_it_down() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    let (reply, _r) = tokio::sync::mpsc::channel(1);
    let cancel = crate::ops::CancelToken::new();
    st.tasks.insert(5, crate::ops::TaskHandle { id: 5, cancel: cancel.clone(), reply });
    st.task_progress.insert(
        5,
        BgTransfer {
            verb: "Copying",
            update: Some(progress_update(5, "Copying", 40, 80)),
            schemes: vec![],
            chart: Default::default(),
        },
    );
    st.dialog = Some(Dialog::Progress(st.progress_dialog_for(5)));
    st.start_saver();

    st.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).await;
    assert!(st.saver.is_none(), "the screen is back");
    assert!(matches!(st.dialog, Some(Dialog::Progress(_))), "the progress dialog stays");
    assert!(!cancel.is_cancelled(), "the copy carries on");
    assert!(st.pending_esc.is_none(), "and no Esc is left waiting for a digit");
}

/// A question appearing while nobody is looking (a copy stopping to ask about
/// an overwrite) ends the screensaver, so it is on screen when they come back.
#[tokio::test]
async fn a_dialog_appearing_ends_the_screensaver() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.start_saver();
    assert!(st.on_tick());
    assert!(st.saver.is_some(), "nothing changed: it keeps playing");
    st.show_error("Overwrite?");
    st.on_tick();
    assert!(st.saver.is_none());
}

/// A Details view of a file in a work tree counts the commits that touched it
/// into a calendar — after the cursor has rested, and from the cache when it
/// comes back — and makes no calendar outside one.
#[tokio::test]
async fn details_view_counts_git_activity_for_the_item_under_the_cursor() {
    use crate::details::ActivityView;
    use crate::panel::ViewFormat;
    let root = temp_dir("details_activity");
    let today = crate::git::activity::today();
    let run = |args: &[&str], days_ago: i64| {
        let date = format!("@{} +0000", (today - days_ago) * 86_400 + 3600);
        std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@e")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@e")
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    };
    if !run(&["init", "-q"], 0) {
        return; // no git here
    }
    for (i, days_ago) in [20, 5, 5].into_iter().enumerate() {
        std::fs::write(root.join("busy.txt"), format!("{i}")).unwrap();
        run(&["add", "-A"], days_ago);
        run(&["commit", "-qm", "busy"], days_ago);
    }
    std::fs::write(root.join("quiet.txt"), "q").unwrap();
    run(&["add", "-A"], 1);
    run(&["commit", "-qm", "quiet"], 1);

    let (tx, mut rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    st.panels[1].format = ViewFormat::Details;
    let cursor_on = |st: &mut AppState, name: &str| {
        st.panels[0].cursor = st.panels[0].entries.iter().position(|e| e.name == name).unwrap();
    };
    // Everything the background work sends, applied until `done` says so.
    async fn pump(
        st: &mut AppState,
        rx: &mut crate::util::async_bridge::AppReceiver,
        done: impl Fn(&AppEvent) -> bool,
    ) {
        loop {
            let ev = tokio::time::timeout(Duration::from_secs(10), rx.recv())
                .await
                .expect("the event arrives")
                .expect("channel open");
            let stop = done(&ev);
            st.apply_event(ev).await;
            if stop {
                break;
            }
        }
    }

    cursor_on(&mut st, "busy.txt");
    st.update_details();
    assert!(matches!(st.details[1].activity, ActivityView::None), "no git status yet");
    st.update_git();
    pump(&mut st, &mut rx, |ev| matches!(ev, AppEvent::GitStatusScanned { .. })).await;
    st.update_details();
    assert!(matches!(st.details[1].activity, ActivityView::Loading));
    pump(&mut st, &mut rx, |ev| matches!(ev, AppEvent::DetailsActivity { .. })).await;
    let ActivityView::Ready(a) = &st.details[1].activity else { panic!("counted") };
    assert_eq!((a.total, a.count(today - 5), a.count(today - 20)), (3, Some(2), Some(1)));

    cursor_on(&mut st, "quiet.txt");
    st.update_details();
    pump(&mut st, &mut rx, |ev| matches!(ev, AppEvent::DetailsActivity { .. })).await;
    assert!(matches!(&st.details[1].activity, ActivityView::Ready(a) if a.total == 1));

    // Back on the first file: straight from the cache, no git run.
    cursor_on(&mut st, "busy.txt");
    st.update_details();
    assert!(matches!(&st.details[1].activity, ActivityView::Ready(a) if a.total == 3));

    // Turned off, it goes away.
    st.config.details_activity = false;
    st.update_details();
    assert!(matches!(st.details[1].activity, ActivityView::None));
    let _ = std::fs::remove_dir_all(&root);
}

/// `b` in the viewer blames the file in the background, and Enter on a line
/// walks the panel into history, to that line's commit, with the file focused.
#[tokio::test]
async fn enter_on_a_blamed_line_opens_its_commit_in_the_panel() {
    let root = temp_dir("blame_open");
    let run = |args: &[&str], date: &str| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@e")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@e")
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    };
    if !run(&["init", "-q"], "2026-01-01T00:00:00+00:00") {
        return; // no git here
    }
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), b"one\ntwo\n").unwrap();
    run(&["add", "-A"], "2026-01-01T00:00:00+00:00");
    run(&["commit", "-qm", "Start"], "2026-01-01T00:00:00+00:00");
    std::fs::write(root.join("src/lib.rs"), b"one\nTWO\n").unwrap();
    run(&["commit", "-qam", "Shout"], "2026-02-01T00:00:00+00:00");

    let (tx, mut rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(root.join("src"));
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    st.panels[0].cursor = st.panels[0].entries.iter().position(|e| e.name == "lib.rs").unwrap();
    st.open_view().await;

    st.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE)).await;
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("the blame arrives")
            .expect("channel open");
        let done = matches!(ev, AppEvent::BlameLoaded { .. });
        st.apply_event(ev).await;
        if done {
            break;
        }
    }
    assert!(st.viewer.as_ref().unwrap().active_blame().is_some(), "{:?}", st.dialog.is_some());

    st.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)).await;
    st.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
    assert!(st.viewer.is_none(), "the viewer makes way for the panel");
    let p = &st.panels[0];
    assert_eq!(p.cwd.scheme, "git");
    let inner = p.cwd.path.to_string_lossy().into_owned();
    assert!(inner.starts_with("/2026-02-01_00-00-00_") && inner.ends_with("_Shout/src"), "{inner}");
    assert_eq!(p.current_entry().map(|e| e.name.as_str()), Some("lib.rs"));

    let _ = std::fs::remove_dir_all(&root);
}

/// Full dispatch for the Git menu's *Browse a revision*: mounting points the
/// active panel at the repository's history, walking into a commit lists that
/// commit's tree, and `..` climbs back out to the work tree.
#[tokio::test]
async fn git_browse_revisions_mounts_history_and_walks_back_out() {
    let git_ok = std::process::Command::new("git")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !git_ok {
        return;
    }

    let root = crate::util::temp::rc_temp_path("state-gitbrowse");
    std::fs::create_dir_all(&root).unwrap();
    let run = |args: &[&str], date: &str| {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@e")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@e")
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(ok.success(), "git {args:?}");
    };
    run(&["init", "-q", "-b", "main"], "2026-05-11T10:00:00+00:00");
    std::fs::write(root.join("tracked.txt"), b"hi").unwrap();
    run(&["add", "-A"], "2026-05-11T10:00:00+00:00");
    run(&["commit", "-q", "-m", "only commit"], "2026-05-11T10:00:00+00:00");

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();

    st.git_browse_revisions(root.clone()).await;
    assert_eq!(st.panels[0].cwd.scheme, "git", "the panel mounted the history");

    // The mount's root is the revision list, so the feature needs no picker.
    let revs: Vec<String> = st.panels[0].entries.iter().map(|e| e.name.clone()).collect();
    let rev = revs.iter().find(|n| n.contains("only-commit")).expect("{revs:?}").clone();
    assert!(rev.starts_with("2026-05-11_10-00-00_"), "{rev}");

    // Walking into a commit lists that commit's tree.
    st.panels[0].cursor = st.panels[0].entries.iter().position(|e| e.name == rev).unwrap();
    st.enter_dir().await;
    let names: Vec<String> = st.panels[0].entries.iter().map(|e| e.name.clone()).collect();
    assert!(names.contains(&"tracked.txt".to_string()), "{names:?}");

    // The border labels the revision rather than a branch.
    st.update_git();
    let label = st.panels[0].git.as_ref().expect("a mount labels itself").branch.clone();
    assert!(label.contains("only commit"), "{label}");

    // History is read-only, so a copy into it is refused before any bytes move.
    st.panels[1].cwd = VfsPath::local(&root);
    st.panels[1].backend = st.registry.local();
    st.panels[1].reload().await.unwrap();
    st.active = 1;
    st.panels[1].cursor =
        st.panels[1].entries.iter().position(|e| e.name == "tracked.txt").unwrap();
    st.open_transfer_dialog(OpKind::Copy);
    match st.dialog.take() {
        Some(Dialog::Message(_)) => {}
        other => panic!("a copy into history should be refused, got {:?}", other.is_some()),
    }

    // `..` at the mount root leaves for the work tree, not the repo's parent.
    st.active = 0;
    let up = st.panels[0].cwd.parent().unwrap().parent().unwrap();
    assert_eq!(up.path, root, "leaving history lands in the work tree");
    assert!(up.is_plain_local());

    std::fs::remove_dir_all(&root).ok();
}

/// F3 on an audio file shows its spectrogram with the transport controls; the
/// document's own keys are held back while it does, and F8 swaps in the bytes.
#[tokio::test]
async fn f3_opens_an_audio_file_on_its_picture_and_keeps_the_document_keys_out() {
    let root = temp_dir("audioview");
    crate::audio::tests::write_wav(&root.join("tone.wav"), 22_050, 1.0, 440.0, 0.4);
    std::fs::write(root.join("fake.mp3"), b"not really an mp3 at all").unwrap();

    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();

    let idx = st.panels[0].entries.iter().position(|e| e.name == "tone.wav").unwrap();
    st.panels[0].cursor = idx;
    st.open_view().await;
    let v = st.viewer.as_ref().expect("the viewer opened");
    let a = v.active_audio().expect("an audio file opens on its audio view");
    assert_eq!(a.display(), crate::config::AudioDisplay::Spectrogram, "the default picture");
    assert_eq!(a.info.sample_rate, 22_050);
    let labels = v.footer_labels();
    assert_eq!((labels[1], labels[3], labels[7]), ("Waveform", "", "Raw"));
    assert!(st.viewer.as_ref().unwrap().audio_busy(), "ticks while the picture fills in");

    // F2 switches the picture; the document's keys do nothing here.
    st.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE)).await;
    for code in [KeyCode::F(4), KeyCode::Char('f'), KeyCode::Char('b')] {
        st.handle_key(KeyEvent::new(code, KeyModifiers::NONE)).await;
    }
    let v = st.viewer.as_ref().unwrap();
    assert_eq!(v.active_audio().unwrap().display(), crate::config::AudioDisplay::Waveform);
    assert_eq!(v.mode, crate::viewer::ViewMode::Text, "F4 did not cycle the hidden mode");
    assert!(!v.following(), "f did not start following the file");

    // Space plays (through the test output), and the tick keeps up with it.
    st.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)).await;
    assert!(st.viewer.as_ref().unwrap().active_audio().unwrap().playing());
    assert!(st.wants_ticks());

    // F8 shows the raw bytes and offers the audio view back.
    st.handle_key(KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE)).await;
    let v = st.viewer.as_ref().unwrap();
    assert!(v.active_audio().is_none());
    assert_eq!(v.footer_labels()[7], "Audio");
    st.handle_key(KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE)).await;
    assert!(st.viewer.as_ref().unwrap().active_audio().is_some());

    // Closing the viewer silences it.
    st.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)).await;
    assert!(st.viewer.is_none());
    assert!(!st.audio_out.active(), "closing the viewer stopped playback");

    // With auto-play on, the viewer opens the file already playing.
    assert!(!st.config.audio_autoplay, "auto-play is off unless turned on");
    st.config.audio_autoplay = true;
    let idx = st.panels[0].entries.iter().position(|e| e.name == "tone.wav").unwrap();
    st.panels[0].cursor = idx;
    st.open_view().await;
    assert!(st.viewer.as_ref().unwrap().active_audio().unwrap().playing(), "auto-play started it");
    st.viewer = None;
    assert!(!st.audio_out.active());
    st.config.audio_autoplay = false;

    // Something named like audio that does not decode opens as bytes.
    let idx = st.panels[0].entries.iter().position(|e| e.name == "fake.mp3").unwrap();
    st.panels[0].cursor = idx;
    st.open_view().await;
    assert!(st.viewer.as_ref().unwrap().active_audio().is_none());

    let _ = std::fs::remove_dir_all(&root);
}

/// The Details view previews an audio file with working controls, which a click
/// operates without taking the focus from the file list; moving the cursor off
/// the file stops it.
#[tokio::test]
async fn the_details_view_plays_audio_until_the_cursor_moves_on() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let root = temp_dir("details_audio");
    crate::audio::tests::write_wav(&root.join("tone.wav"), 22_050, 2.0, 440.0, 0.4);
    std::fs::write(root.join("notes.txt"), b"hello\n").unwrap();

    let (tx, mut rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();
    st.panels[1].format = ViewFormat::Details;
    // Auto-play is the viewer's alone: the preview must still wait for Play.
    st.config.audio_autoplay = true;

    let idx = st.panels[0].entries.iter().position(|e| e.name == "tone.wav").unwrap();
    st.panels[0].cursor = idx;
    st.update_details();
    drain_until_preview(&mut st, &mut rx).await;
    assert!(matches!(st.details[1].preview, crate::details::Preview::Audio(_)));
    assert!(st.details[1].audio.is_some(), "the preview is drawn and playable");
    assert!(!st.details[1].audio.as_ref().unwrap().playing(), "a preview never plays by itself");
    assert!(!st.audio_out.active());

    // Draw a frame so the controls have somewhere to be clicked.
    let mut term = Terminal::new(TestBackend::new(160, 50)).unwrap();
    term.draw(|f| crate::ui::draw(f, &mut st)).unwrap();
    let screen: String = term.backend().buffer().content.iter().map(|c| c.symbol()).collect();
    assert!(screen.contains("0:00/0:02"), "the time is shown: {screen}");
    let hits = st.details[1].audio.as_ref().unwrap().hits();
    let (play, _) = *hits
        .buttons
        .iter()
        .find(|(_, t)| *t == crate::audio::view::Transport::PlayPause)
        .expect("a play button was drawn");
    let click =
        |kind| MouseEvent { kind, column: play.x + 1, row: play.y, modifiers: KeyModifiers::NONE };
    st.handle_mouse(click(MouseEventKind::Down(MouseButton::Left))).await;
    st.handle_mouse(click(MouseEventKind::Up(MouseButton::Left))).await;
    assert!(st.details[1].audio.as_ref().unwrap().playing(), "the click pressed Play");
    assert_eq!(st.active, 0, "and the file list kept the focus");

    // With the Details view active, Space pauses — unless the command line
    // has something typed in it, which Space then belongs to.
    st.active = 1;
    st.cmd.set("ls".into());
    st.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)).await;
    assert!(st.details[1].audio.as_ref().unwrap().playing());
    assert_eq!(st.cmd.buffer, "ls ");
    st.cmd.set(String::new());
    st.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)).await;
    assert!(!st.details[1].audio.as_ref().unwrap().playing(), "Space paused it");
    st.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)).await;
    assert!(st.wants_ticks(), "a playing preview keeps the tick going");
    st.active = 0;

    // Moving on stops it.
    let idx = st.panels[0].entries.iter().position(|e| e.name == "notes.txt").unwrap();
    st.panels[0].cursor = idx;
    st.update_details();
    assert!(st.details[1].audio.is_none());
    assert!(!st.audio_out.active(), "moving the cursor off the file stopped it");

    let _ = std::fs::remove_dir_all(&root);
}

/// Hex mode runs the file's binary template on the tick; the picker's choice
/// replaces it; and editing a template opens it over the hex editor, which
/// comes back — running the edited template — when that editor closes.
#[tokio::test]
async fn binary_templates_run_are_chosen_and_edited_over_the_hex_editor() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let dir = temp_dir("bttpl");
    let file = dir.join("data.zip");
    {
        let f = std::fs::File::create(&file).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        z.start_file("a.txt", opts).unwrap();
        std::io::Write::write_all(&mut z, b"template test").unwrap();
        z.finish().unwrap();
    }
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.open_path_in_editor(file.clone()).await;
    let ed = st.editor.as_mut().unwrap();
    ed.handle_key(KeyEvent::new(KeyCode::F(9), KeyModifiers::CONTROL));
    assert!(ed.is_hex());
    let settle = |st: &mut AppState| {
        let deadline = Instant::now() + Duration::from_secs(20);
        while st.editor.as_ref().is_some_and(|e| e.template_pending()) && Instant::now() < deadline
        {
            st.on_tick();
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    let screen = |st: &mut AppState| {
        let theme = crate::ui::theme::Theme::mc();
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 30)).unwrap();
        let ed = st.editor.as_mut().unwrap();
        t.draw(|f| crate::editor::render::render(f, f.area(), ed, &theme)).unwrap();
        let b = t.backend().buffer();
        (0..b.area.height)
            .map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(st.wants_ticks(), "a template run keeps the tick going");
    settle(&mut st);
    let ed = st.editor.as_ref().unwrap();
    assert_eq!(ed.active_template().map(|i| i.file_name), Some("ZIP.bt".into()));
    assert!(screen(&mut st).contains("record"));

    // F5's picker lists the templates; "(No template)" hides the panel.
    st.apply_editor_signal(EditorSignal::OpenTemplatePicker).await;
    assert!(matches!(st.dialog, Some(Dialog::TemplatePicker(_))));
    st.dialog = None;
    st.handle_submit(Submit::EditorTemplate(None)).await;
    assert!(!st.editor.as_ref().unwrap().template_panel());

    // A template of the user's own, chosen, then edited.
    let tpl = dir.join("Mine.bt");
    std::fs::write(&tpl, "// File Mask: *.zip\nuint magic <format=hex>;\n").unwrap();
    let mut info = crate::bt::header::parse_header("Mine.bt", &std::fs::read(&tpl).unwrap());
    info.path = Some(tpl.clone());
    st.handle_submit(Submit::EditorTemplate(Some(Box::new(info.clone())))).await;
    settle(&mut st);
    let s = screen(&mut st);
    assert!(s.contains("magic") && s.contains("0x4034B50"), "{s}");

    st.handle_submit(Submit::EditorEditTemplate(Box::new(info))).await;
    assert_eq!(st.editor_stack.len(), 1, "the hex editor waits underneath");
    let ed = st.editor.as_mut().unwrap();
    assert!(!ed.is_hex() && ed.name == "Mine.bt");
    ed.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL));
    for c in "ushort version;".chars() {
        ed.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    st.apply_editor_signal(EditorSignal::Save { close_after: true }).await;
    assert!(st.editor_stack.is_empty());
    let ed = st.editor.as_ref().unwrap();
    assert!(ed.is_hex(), "back in the hex editor");
    settle(&mut st);
    let s = screen(&mut st);
    assert!(s.contains("version"), "the edited template ran: {s}");

    // Closing the hex editor now leaves for the panels.
    st.apply_editor_signal(EditorSignal::Close).await;
    assert!(st.editor.is_none());
    std::fs::remove_dir_all(&dir).ok();
}

/// External panelize end to end: a command's stdout becomes the panel listing,
/// and the entries are real `VfsPath`s the file operations can act on.
#[cfg(unix)]
#[tokio::test]
async fn panelize_command_output_becomes_a_panelized_listing() {
    use crate::ui::dialog::Submit;
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rc_panelize_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("a.txt"), b"aa").unwrap();
    std::fs::write(root.join("sub/b.txt"), b"bbb").unwrap();

    let (tx, mut rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();

    // `missing.txt` names nothing: it must be dropped rather than listed.
    st.handle_submit(Submit::Panelize("printf 'a.txt\\nsub/b.txt\\nmissing.txt\\n'".to_string()))
        .await;
    loop {
        let ev = rx.recv().await.unwrap();
        let done = matches!(ev, AppEvent::PanelizeDone { .. });
        st.apply_event(ev).await;
        if done {
            break;
        }
    }

    assert!(st.panels[0].is_panelized(), "the output replaced the listing");
    let names: Vec<String> = st.panels[0].entries.iter().map(|e| e.name.clone()).collect();
    let has = |n: &str| names.iter().any(|x| x.ends_with(n));
    assert!(has("a.txt"), "a relative line resolved against the panel's cwd: {names:?}");
    assert!(has("sub/b.txt"), "so did one naming a subdirectory: {names:?}");
    assert!(!has("missing.txt"), "a line naming nothing is dropped: {names:?}");

    // The cursor opens on "..", which is never an operation target; step onto
    // the first result and the panel yields the real absolute path behind it.
    // That mapping is what makes F3/F5/F8 work on a listing no directory backs.
    assert!(st.panels[0].operation_targets().is_empty(), "\"..\" is never a target");
    st.panels[0].move_cursor(1);
    let targets = st.panels[0].operation_targets();
    assert_eq!(
        targets,
        vec![VfsPath::local(root.join("a.txt"))],
        "the cursor maps to the stored absolute path"
    );

    // The command was remembered for the next dialog, and did not leak into the
    // shell history, which only holds commands actually run at the prompt.
    assert!(st.last_panelize.starts_with("printf"), "the command is prefilled next time");

    std::fs::remove_dir_all(&root).ok();
}

/// Panelize needs a local directory: the shell cannot be pointed at a cwd
/// inside an archive or on a remote, the same limit the command line has.
#[tokio::test]
async fn panelize_refuses_a_non_local_panel() {
    let (tx, _rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::archive("/tmp/some.zip", "/");

    st.open_panelize_dialog();
    assert!(
        matches!(st.dialog, Some(Dialog::Message(_))),
        "an error is shown instead of the command prompt"
    );
}

/// Alt-D opens the diff *and* loads the hunks the staging keys act on. Without
/// this the view would look identical but `s` would report "no hunk", which is
/// exactly the failure a rendering test would miss.
#[tokio::test]
async fn the_git_diff_carries_the_hunks_that_staging_acts_on() {
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rc_gitdiff_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let run = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@e")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@e")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    if !run(&["init", "-q", "-b", "main"]) {
        eprintln!("git unavailable; skipping");
        let _ = std::fs::remove_dir_all(&root);
        return;
    }
    let body: String = (1..=20).map(|i| format!("line{i}\n")).collect();
    std::fs::write(root.join("f.txt"), &body).unwrap();
    run(&["add", "-A"]);
    run(&["commit", "-q", "-m", "init"]);
    // Two edits far enough apart that -U3 keeps them as separate hunks.
    let edited = body.replace("line2\n", "CHANGED2\n").replace("line19\n", "CHANGED19\n");
    std::fs::write(root.join("f.txt"), &edited).unwrap();

    let (tx, mut rx) = async_bridge::channel();
    let mut st = AppState::new(tx);
    st.active = 0;
    st.panels[0].cwd = VfsPath::local(&root);
    st.panels[0].backend = st.registry.local();
    st.panels[0].reload().await.unwrap();

    // The panel's git status is read on a background task, and `open_git_diff`
    // refuses without it, so wait for it to land.
    st.update_git();
    loop {
        let ev = rx.recv().await.unwrap();
        let done = matches!(ev, AppEvent::GitStatusScanned { .. });
        st.apply_event(ev).await;
        if done {
            break;
        }
    }
    assert!(st.panels[0].git.is_some(), "the panel knows it is in a repository");
    st.panels[0].cursor =
        st.panels[0].entries.iter().position(|e| e.name == "f.txt").expect("the file");

    st.open_git_diff().await;
    let view = st.diffview.as_ref().expect("the diff opened");
    let git = view.git.as_ref().expect("and it carries its git context");
    assert_eq!(git.unstaged.hunks.len(), 2, "both edits arrived as hunks");
    assert_eq!(git.rel, std::path::Path::new("f.txt"));
    assert!(!git.staged_dirty, "nothing is staged yet");
    // The cursor opens at the top, which is inside the first hunk (the edit is
    // on line 2, and -U3 starts the hunk at line 1).
    assert_eq!(view.hunk_position(), Some((1, 2)), "cursor is in hunk 1 of 2");

    // And moving to the end of the file finds the other one, which is what
    // makes `s` stage the hunk you are actually looking at.
    let view = st.diffview.as_mut().unwrap();
    view.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    assert_eq!(view.hunk_at_cursor(), Some(1), "the last row belongs to the second hunk");

    let _ = std::fs::remove_dir_all(&root);
}
