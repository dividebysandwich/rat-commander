use super::*;
use crate::config::RemoteHistoryEntry;
use crate::vfs::remote::Protocol;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn goto_dialog_collects_value_and_mode_by_keyboard() {
    let mut d = GotoDialog::new();
    for c in "0x1f".chars() {
        d.handle_key(key(KeyCode::Char(c)));
    }
    // Move the radio selection to "Hexadecimal offset" (mode 3).
    for _ in 0..3 {
        d.handle_key(key(KeyCode::Down));
    }
    match d.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::ViewerGoto(v, m)) => {
            assert_eq!(v, "0x1f");
            assert_eq!(m, crate::viewer::GotoMode::HexOffset);
        }
        _ => panic!("expected a ViewerGoto submit"),
    }
    // An empty value (or Esc) cancels rather than submitting.
    assert!(matches!(GotoDialog::new().handle_key(key(KeyCode::Enter)), DialogResult::Cancel));
    assert!(matches!(GotoDialog::new().handle_key(key(KeyCode::Esc)), DialogResult::Cancel));
}

#[test]
fn goto_dialog_mouse_selects_radio_and_buttons() {
    // 80x24 → the box is centered at {20,7,40,9}; inner {21,8,38,7}.
    let area = Rect::new(0, 0, 80, 24);
    let mut d = GotoDialog::new();
    for c in "12".chars() {
        d.handle_key(key(KeyCode::Char(c)));
    }
    // Radio rows are inner.y+1.. → row 11 is "Decimal offset" (index 2).
    assert!(matches!(d.handle_click(area, 25, 11), DialogResult::None));
    assert_eq!(d.mode, 2);
    // The button row is the last interior row (y=14); left half is OK.
    match d.handle_click(area, 25, 14) {
        DialogResult::Submit(Submit::ViewerGoto(v, m)) => {
            assert_eq!(v, "12");
            assert_eq!(m, crate::viewer::GotoMode::DecimalOffset);
        }
        _ => panic!("clicking OK submits"),
    }
    // The right half of the button row cancels.
    assert!(matches!(d.handle_click(area, 55, 14), DialogResult::Cancel));
}

#[test]
fn mount_path_and_password_inputs_submit() {
    // The mount-path input yields a Mount submit with the device + typed path.
    let mut d = InputDialog::new("Mount", "at:", "/mnt/x", InputPurpose::MountPath("/dev/sdb1".into()));
    match d.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::Mount { device, path }) => {
            assert_eq!(device, "/dev/sdb1");
            assert_eq!(path, "/mnt/x");
        }
        _ => panic!("expected Mount submit"),
    }
    // The password input is masked and submits the raw buffer (even empty).
    let mut d = InputDialog::password("Auth", "pw:", InputPurpose::SudoPassword);
    assert!(d.masked);
    d.handle_key(key(KeyCode::Char('s')));
    d.handle_key(key(KeyCode::Char('3')));
    match d.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::SudoPassword(pw)) => assert_eq!(pw, "s3"),
        _ => panic!("expected SudoPassword submit"),
    }
}

#[test]
fn device_and_mount_action_menus() {
    let dev = |mp: Option<&str>| crate::mount::BlockDevice {
        name: "sdb1".into(),
        dev: "/dev/sdb1".into(),
        mountpoint: mp.map(str::to_string),
        ..Default::default()
    };
    // Unmounted device: the focused "Mount" button → MountDevice.
    let mut d = ConfirmDialog::device_menu(&dev(None));
    match d.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::MountDevice(dev)) => assert_eq!(dev, "/dev/sdb1"),
        _ => panic!("expected MountDevice"),
    }
    // Mounted device: the only action is Unmount.
    let mut d = ConfirmDialog::device_menu(&dev(Some("/mnt/x")));
    match d.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::AskUnmount(mp)) => assert_eq!(mp, "/mnt/x"),
        _ => panic!("expected AskUnmount"),
    }
    // Mount menu: second button is Sync.
    let mut d = ConfirmDialog::mount_menu("/mnt/x");
    d.handle_key(key(KeyCode::Right)); // focus "Sync"
    match d.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::SyncPath(mp)) => assert_eq!(mp, "/mnt/x"),
        _ => panic!("expected SyncPath"),
    }
}

fn bdev(name: &str, dev: &str, size: u64, removable: bool) -> crate::mount::BlockDevice {
    crate::mount::BlockDevice {
        name: name.into(),
        dev: dev.into(),
        size,
        removable,
        ..Default::default()
    }
}

#[test]
fn device_menu_offers_flash_and_create_image() {
    // Free device: [Mount, Format, Flash image, Create image, Cancel].
    let mut menu = ConfirmDialog::device_menu(&bdev("sdb", "/dev/sdb", 100, true));
    menu.handle_key(key(KeyCode::Right)); // Format
    menu.handle_key(key(KeyCode::Right)); // Flash image
    match menu.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::FlashBrowse(t)) => assert_eq!(t.dev, "/dev/sdb"),
        _ => panic!("expected FlashBrowse"),
    }
    let mut menu = ConfirmDialog::device_menu(&bdev("sdb", "/dev/sdb", 100, true));
    for _ in 0..3 {
        menu.handle_key(key(KeyCode::Right)); // → Create image
    }
    match menu.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::ImageBrowse(t)) => assert_eq!(t.dev, "/dev/sdb"),
        _ => panic!("expected ImageBrowse"),
    }
}

#[test]
fn image_save_dialog_builds_a_spec() {
    let src = crate::flash::FlashTarget { dev: "/dev/sdb".into(), size: 4096, ..Default::default() };
    // Start in an existing dir (the temp dir) and confirm with the default name.
    let mut d = ImageSaveDialog::new(src, std::env::temp_dir());
    d.focus = SaveFocus::Name; // jump to the name field
    match d.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::ImageSave(spec)) => {
            assert_eq!(spec.source.dev, "/dev/sdb");
            assert_eq!(spec.dest_name, "sdb.img", "default name from the device");
            assert_eq!(spec.dest_path, std::env::temp_dir().join("sdb.img"));
        }
        _ => panic!("expected ImageSave submit"),
    }
    // The overwrite confirm routes to DoImage.
    let spec = crate::flash::ImageSpec {
        source: crate::flash::FlashTarget { dev: "/dev/sdb".into(), size: 10, ..Default::default() },
        dest_path: "/tmp/x.img".into(),
        dest_name: "x.img".into(),
    };
    let mut ov = ConfirmDialog::image_overwrite(spec);
    assert!(matches!(ov.handle_key(key(KeyCode::Enter)), DialogResult::Submit(Submit::DoImage(_))));
}

#[test]
fn drive_dialog_anchors_over_its_panel() {
    // Alt-F1 → left panel (side 0), Alt-F2 → right panel (side 1); other dialogs
    // are not panel-anchored.
    assert_eq!(Dialog::Drive(DriveDialog::new(0, vec![], None, false)).anchor_panel(), Some(0));
    assert_eq!(Dialog::Drive(DriveDialog::new(1, vec![], None, false)).anchor_panel(), Some(1));
    assert_eq!(Dialog::Confirm(ConfirmDialog::quit()).anchor_panel(), None);
}

#[test]
fn drive_dialog_connection_buttons() {
    // No drives (Linux/macOS): only SFTP/FTP/SCP; default cursor on SFTP.
    let mut d = DriveDialog::new(0, vec![], None, false);
    match d.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::OpenConnect(0, Protocol::Sftp)) => {}
        _ => panic!("expected SFTP OpenConnect"),
    }
    // Right, Right → SCP.
    let mut d = DriveDialog::new(1, vec![], None, false);
    d.handle_key(key(KeyCode::Right));
    d.handle_key(key(KeyCode::Right));
    assert!(matches!(
        d.handle_key(key(KeyCode::Enter)),
        DialogResult::Submit(Submit::OpenConnect(1, Protocol::Scp))
    ));
}

#[test]
fn drive_dialog_disconnect_only_when_connected() {
    // Connected → a trailing Disconnect button (End lands on it).
    let mut d = DriveDialog::new(0, vec![], None, true);
    d.handle_key(key(KeyCode::End));
    assert!(matches!(
        d.handle_key(key(KeyCode::Enter)),
        DialogResult::Submit(Submit::DisconnectPanel(0))
    ));
    // Not connected → End lands on the last connection (SCP), no Disconnect.
    let mut d = DriveDialog::new(0, vec![], None, false);
    d.handle_key(key(KeyCode::End));
    assert!(matches!(
        d.handle_key(key(KeyCode::Enter)),
        DialogResult::Submit(Submit::OpenConnect(0, Protocol::Scp))
    ));
}

#[test]
fn drive_dialog_letter_jumps_and_highlights_current() {
    // Windows-style: drive letters present, current drive highlighted.
    let mut d = DriveDialog::new(0, vec!['A', 'C', 'D', 'Z'], Some('C'), false);
    // A drive letter jumps straight to that drive.
    match d.handle_key(key(KeyCode::Char('z'))) {
        DialogResult::Submit(Submit::SetDrive(0, c)) => assert_eq!(c, 'Z'),
        _ => panic!("expected SetDrive Z"),
    }
    // Enter activates the highlighted (current) drive C.
    let mut d = DriveDialog::new(0, vec!['A', 'C', 'D', 'Z'], Some('C'), false);
    assert!(matches!(
        d.handle_key(key(KeyCode::Enter)),
        DialogResult::Submit(Submit::SetDrive(0, 'C'))
    ));
}

#[test]
fn flash_target_picker_enforces_size() {
    let devs = vec![
        bdev("sda", "/dev/sda", 1_000, false),  // too small
        bdev("sdb", "/dev/sdb", 10_000, true),  // fits
    ];
    let img = std::path::PathBuf::from("/img/x.iso");
    // Preselect the big device → Enter flashes it.
    let mut d = FlashTargetDialog::new(img.clone(), "x.iso".into(), 5_000, devs.clone(), Some("/dev/sdb"));
    match d.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::FlashSelected(spec)) => {
            assert_eq!(spec.target.dev, "/dev/sdb");
            assert_eq!(spec.image_size, 5_000);
        }
        _ => panic!("expected FlashSelected"),
    }
    // The default (first) device is too small → Enter is refused.
    let mut d = FlashTargetDialog::new(img, "x.iso".into(), 5_000, devs, None);
    assert!(matches!(d.handle_key(key(KeyCode::Enter)), DialogResult::None));
}

#[test]
fn flash_confirmations_emit_expected_submits() {
    let spec = |removable: bool| crate::flash::FlashSpec {
        image_path: "/x.iso".into(),
        image_name: "x.iso".into(),
        image_size: 10,
        target: crate::flash::FlashTarget {
            dev: "/dev/sdb".into(),
            size: 100,
            removable,
            ..Default::default()
        },
    };
    // Removable → straight to the destructive confirm; "Flash" → DoFlash.
    let mut d = ConfirmDialog::flash_confirm(spec(true));
    match d.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::DoFlash(s)) => assert_eq!(s.target.dev, "/dev/sdb"),
        _ => panic!("expected DoFlash"),
    }
    // Non-removable danger defaults to Cancel; "Continue" → FlashConfirm.
    let mut d = ConfirmDialog::flash_danger(spec(false));
    assert!(matches!(d.handle_key(key(KeyCode::Enter)), DialogResult::Cancel));
    let mut d = ConfirmDialog::flash_danger(spec(false));
    d.handle_key(key(KeyCode::Left)); // focus "Continue"
    assert!(matches!(
        d.handle_key(key(KeyCode::Enter)),
        DialogResult::Submit(Submit::FlashConfirm(_))
    ));
    // Abort prompt: Resume (default) vs really-abort.
    let mut d = ConfirmDialog::abort_flash(7);
    assert!(matches!(d.handle_key(key(KeyCode::Enter)), DialogResult::Submit(Submit::FlashResume)));
    let mut d = ConfirmDialog::abort_flash(7);
    d.handle_key(key(KeyCode::Right));
    match d.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::FlashAbort(id)) => assert_eq!(id, 7),
        _ => panic!("expected FlashAbort"),
    }
}

#[test]
fn file_browser_filters_and_picks() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rc_fb_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("disk.img"), b"x").unwrap();
    std::fs::write(dir.join("notes.txt"), b"x").unwrap();
    let target = crate::flash::FlashTarget { dev: "/dev/sdb".into(), size: 100, removable: true, ..Default::default() };
    let mut d = FileBrowserDialog::new(target, dir.clone());
    // The default *.img/*.iso/... filter shows the image + dirs, not the .txt.
    assert!(d.entries.iter().any(|e| e.name == "disk.img" && !e.is_dir));
    assert!(d.entries.iter().any(|e| e.name == "sub" && e.is_dir));
    assert!(!d.entries.iter().any(|e| e.name == "notes.txt"));
    // Picking the image emits its path + the target device.
    d.cursor = d.entries.iter().position(|e| e.name == "disk.img").unwrap();
    match d.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::FlashBrowsePicked(p, t)) => {
            assert_eq!(p, dir.join("disk.img"));
            assert_eq!(t.dev, "/dev/sdb");
        }
        _ => panic!("expected FlashBrowsePicked"),
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn progress_dialog_estimates_time() {
    let mut p = ProgressDialog::new(1, "Flashing");
    p.total_total = 1000;
    p.total_done = 500;
    assert_eq!(p.eta_text(), "--:--", "no speed sample yet");
    p.samples.push((500.0, 100.0)); // 100 B/s, 500 left → 5 s
    assert_eq!(p.eta_text(), "00:05");
}

#[test]
fn indeterminate_progress_abort_is_clickable_but_determinate_is_not() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let theme = crate::ui::theme::Theme::mc();
    let area = Rect::new(0, 0, 80, 24);

    // Indeterminate scan dialog: the Abort button (centered on the last interior
    // row of the 64x8 centered box) is hit-testable.
    let mut d = Dialog::Progress(ProgressDialog::scan(7, "Find duplicates", "duplicates"));
    let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
    t.draw(|f| d.render(f, area, &theme)).unwrap();
    // Box centered(80x24, 64, 8): origin (8,8); inner (9,9,62,6); button row 14.
    assert!(
        matches!(d.handle_click(area, 40, 14), DialogResult::Abort(7)),
        "clicking the scan dialog's Abort button cancels it"
    );
    assert!(matches!(d.handle_click(area, 40, 10), DialogResult::None), "a click elsewhere does nothing");

    // A determinate (copy) progress dialog ignores clicks entirely.
    let mut c = Dialog::Progress(ProgressDialog::new(8, "Copying"));
    t.draw(|f| c.render(f, area, &theme)).unwrap();
    for row in 0..24 {
        assert!(matches!(c.handle_click(area, 40, row), DialogResult::None));
    }
}

#[test]
fn unmount_danger_defaults_to_cancel_and_confirms_explicitly() {
    // The red essential-mount warning defaults focus to Cancel, so a stray
    // Enter is harmless.
    let mut d = ConfirmDialog::unmount_danger("/");
    assert!(d.danger, "dialog flagged dangerous");
    match d.handle_key(key(KeyCode::Enter)) {
        DialogResult::Cancel => {}
        _ => panic!("default focus must be Cancel"),
    }
    // Choosing "Unmount anyway" still goes through to DoUnmount.
    let mut d = ConfirmDialog::unmount_danger("/boot");
    d.handle_key(key(KeyCode::Left)); // move focus to "Unmount anyway"
    match d.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::DoUnmount(mp)) => assert_eq!(mp, "/boot"),
        _ => panic!("expected DoUnmount"),
    }
}

#[test]
fn formatter_collects_a_format_spec() {
    let mut d = FormDialog::format("/dev/sdb1".into());
    match d.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::Format(spec)) => {
            assert_eq!(spec.dev, "/dev/sdb1");
            assert_eq!(spec.fs, crate::mount::FsType::Fat32); // default choice
        }
        _ => panic!("expected Format submit"),
    }
}

#[test]
fn create_mountpoint_confirm_yields_mount_create() {
    let mut d = ConfirmDialog::create_mountpoint("/dev/sdb1", "/mnt/new");
    match d.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::MountCreate { device, path }) => {
            assert_eq!(device, "/dev/sdb1");
            assert_eq!(path, "/mnt/new");
        }
        _ => panic!("expected MountCreate submit"),
    }
}

#[test]
fn confirmations_form_collects_toggles() {
    let cfg = crate::config::Config::default(); // delete=T, overwrite=T, execute=F, exit=T
    // Submitting the defaults reflects the config.
    let mut d = FormDialog::confirmations(&cfg);
    match d.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::Confirmations(v)) => {
            assert!(v.delete && v.overwrite && !v.execute && v.exit);
        }
        _ => panic!("expected Confirmations submit"),
    }
    // Space toggles the focused field (Confirm delete); Enter then submits.
    let mut d = FormDialog::confirmations(&cfg);
    d.handle_key(key(KeyCode::Char(' ')));
    match d.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::Confirmations(v)) => assert!(!v.delete),
        _ => panic!("expected Confirmations submit"),
    }
}

#[test]
fn mix_rgb_blends_endpoints() {
    use ratatui::style::Color;
    let a = Color::Rgb(0, 0, 0);
    let b = Color::Rgb(100, 200, 50);
    assert_eq!(mix_rgb(a, b, 0.0), a);
    assert_eq!(mix_rgb(a, b, 1.0), b);
    assert_eq!(mix_rgb(a, b, 0.5), Color::Rgb(50, 100, 25));
}

#[test]
fn save_discard_cancel_has_three_buttons() {
    // Save.
    let mut d = ConfirmDialog::editor_quit("notes.txt");
    assert_eq!(d.buttons.len(), 3);
    assert!(matches!(
        d.handle_key(key(KeyCode::Enter)),
        DialogResult::Submit(Submit::EditorSaveQuit)
    ));

    // Discard via its hotkey.
    let mut d = ConfirmDialog::editor_quit("notes.txt");
    assert!(matches!(
        d.handle_key(key(KeyCode::Char('d'))),
        DialogResult::Submit(Submit::EditorDiscardQuit)
    ));

    // Cancel via its hotkey resumes editing (no submit).
    let mut d = ConfirmDialog::editor_quit("notes.txt");
    assert!(matches!(d.handle_key(key(KeyCode::Char('c'))), DialogResult::Cancel));

    // Esc still cancels.
    let mut d = ConfirmDialog::diff_quit();
    assert!(matches!(d.handle_key(key(KeyCode::Esc)), DialogResult::Cancel));

    // Focus the third button with Tab×2, then Enter cancels.
    let mut d = ConfirmDialog::diff_quit();
    d.handle_key(key(KeyCode::Tab));
    d.handle_key(key(KeyCode::Tab));
    assert_eq!(d.focus, 2);
    assert!(matches!(d.handle_key(key(KeyCode::Enter)), DialogResult::Cancel));
}

#[test]
fn two_button_confirm_still_works() {
    let mut d = ConfirmDialog::quit();
    assert_eq!(d.buttons.len(), 2);
    assert!(matches!(d.handle_key(key(KeyCode::Char('n'))), DialogResult::Cancel));
    let mut d = ConfirmDialog::quit();
    assert!(matches!(
        d.handle_key(key(KeyCode::Char('y'))),
        DialogResult::Submit(Submit::Quit)
    ));
}

#[test]
fn connect_history_dropdown_fills_fields() {
    let history = vec![
        RemoteHistoryEntry {
            protocol: "sftp".into(),
            host: "a.example".into(),
            port: 2222,
            user: "alice".into(),
            path: "/srv".into(),
        },
        // A different protocol must be filtered out of the dropdown.
        RemoteHistoryEntry {
            protocol: "ftp".into(),
            host: "nope".into(),
            port: 21,
            user: String::new(),
            path: String::new(),
        },
    ];
    let mut d = FormDialog::connect(Protocol::Sftp, 1, history);

    // ↓ on the Host field opens the dropdown; Enter selects the only entry.
    assert!(matches!(d.handle_key(key(KeyCode::Down)), DialogResult::None));
    assert!(matches!(d.handle_key(key(KeyCode::Enter)), DialogResult::None));

    // Submitting now yields the filled-in connection.
    match d.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::Connect(side, creds)) => {
            assert_eq!(side, 1);
            assert_eq!(creds.host, "a.example");
            assert_eq!(creds.port, 2222);
            assert_eq!(creds.user, "alice");
            assert_eq!(creds.path, "/srv");
        }
        _ => panic!("expected a Connect submit"),
    }
}

#[test]
fn down_does_not_open_dropdown_without_history() {
    let mut d = FormDialog::connect(Protocol::Scp, 0, vec![]);
    // With no history, ↓ just moves focus to the next field (no dropdown).
    d.handle_key(key(KeyCode::Down));
    assert!(d.connect.as_ref().is_some_and(|c| !c.open));
    assert_eq!(d.form.focus, 1);
}

#[test]
fn connect_dialog_renders_chevron_and_dropdown() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let history = vec![RemoteHistoryEntry {
        protocol: "sftp".into(),
        host: "host.example".into(),
        port: 22,
        user: "bob".into(),
        path: "/home".into(),
    }];
    let mut d = FormDialog::connect(Protocol::Sftp, 0, history);
    let theme = crate::ui::theme::Theme::mc();
    let mut t = Terminal::new(TestBackend::new(80, 20)).unwrap();

    let dump = |t: &Terminal<TestBackend>| {
        let b = t.backend().buffer();
        let mut s = String::new();
        for y in 0..b.area.height {
            for x in 0..b.area.width {
                s.push_str(b[(x, y)].symbol());
            }
        }
        s
    };

    t.draw(|f| d.render(f, f.area(), &theme)).unwrap();
    assert!(dump(&t).contains('▼'), "chevron shown on the host field");

    d.handle_key(key(KeyCode::Down)); // open the dropdown
    t.draw(|f| d.render(f, f.area(), &theme)).unwrap();
    let s = dump(&t);
    assert!(s.contains("Recent"), "dropdown box title");
    assert!(s.contains("bob@host.example:22"), "history entry label");
}

#[test]
fn multi_rename_mouse_focuses_and_toggles_fields() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let sources = vec![
        VfsPath::local("/tmp/one.txt"),
        VfsPath::local("/tmp/two.txt"),
    ];
    let mut d = MultiRenameDialog::new(sources, "20260101".into(), "120000".into());
    let theme = crate::ui::theme::Theme::mc();
    let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
    let area = ratatui::layout::Rect::new(0, 0, 100, 30);

    // Cell-accurate substring search: returns the (column, row) where `needle`
    // starts (byte offsets would be wrong on rows with multibyte box-drawing).
    let find = |t: &Terminal<TestBackend>, needle: &str| -> Option<(u16, u16)> {
        let b = t.backend().buffer();
        let nlen = needle.chars().count() as u16;
        for y in 0..b.area.height {
            for x in 0..=b.area.width.saturating_sub(nlen) {
                let mut s = String::new();
                for k in 0..nlen {
                    s.push_str(b[(x + k, y)].symbol());
                }
                if s == needle {
                    return Some((x, y));
                }
            }
        }
        None
    };

    // Render once so the dialog records its clickable field geometry.
    t.draw(|f| d.render(f, f.area(), &theme)).unwrap();
    assert!(find(&t, "[ ] Case sensitive").is_some(), "checkbox starts unchecked");
    assert!(find(&t, "unchanged").is_some(), "case starts unchanged");

    // Clicking the checkbox toggles it on.
    let (cx, cy) = find(&t, "Case sensitive").unwrap();
    d.handle_click(area, cx, cy);
    t.draw(|f| d.render(f, f.area(), &theme)).unwrap();
    assert!(find(&t, "[x] Case sensitive").is_some(), "click toggled the checkbox on");

    // Clicking the case chooser cycles unchanged → lowercase.
    let (kx, ky) = find(&t, "Case:").unwrap();
    d.handle_click(area, kx, ky);
    t.draw(|f| d.render(f, f.area(), &theme)).unwrap();
    assert!(find(&t, "lowercase").is_some(), "click cycled the case mode");
}

#[test]
fn save_as_dialog_confirms_joined_path() {
    let dir = std::env::temp_dir();
    let mut d = SaveAsDialog::new(dir.clone(), "notes.txt".into(), None);
    // Focus starts on the name field, so Enter submits the cwd-joined path.
    match d.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::EditorSaveAs(p)) => assert_eq!(p, dir.join("notes.txt")),
        _ => panic!("expected an EditorSaveAs submit"),
    }
    // An empty name refuses to submit.
    let mut d = SaveAsDialog::new(dir, "   ".into(), None);
    assert!(matches!(d.handle_key(key(KeyCode::Enter)), DialogResult::None));
}

#[test]
fn compare_dialog_selects_mode() {
    // Default focus is Quick; Enter submits it.
    let mut d = CompareDialog::new();
    assert!(matches!(
        d.handle_key(key(KeyCode::Enter)),
        DialogResult::Submit(Submit::CompareDirs(CompareMode::Quick))
    ));
    // Hotkeys pick a mode directly.
    assert!(matches!(
        d.handle_key(key(KeyCode::Char('s'))),
        DialogResult::Submit(Submit::CompareDirs(CompareMode::Size))
    ));
    assert!(matches!(
        d.handle_key(key(KeyCode::Char('c'))),
        DialogResult::Submit(Submit::CompareDirs(CompareMode::Content))
    ));
    // Arrow navigation then Enter.
    let mut d = CompareDialog::new();
    d.handle_key(key(KeyCode::Right));
    d.handle_key(key(KeyCode::Right));
    assert!(matches!(
        d.handle_key(key(KeyCode::Enter)),
        DialogResult::Submit(Submit::CompareDirs(CompareMode::Content))
    ));
    assert!(matches!(d.handle_key(key(KeyCode::Esc)), DialogResult::Cancel));
}
