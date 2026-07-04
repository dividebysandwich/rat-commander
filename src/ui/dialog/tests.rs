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
    assert_eq!(
        Dialog::Drive(DriveDialog::new(0, vec![], None, None, vec![], true)).anchor_panel(),
        Some(0)
    );
    assert_eq!(
        Dialog::Drive(DriveDialog::new(1, vec![], None, None, vec![], true)).anchor_panel(),
        Some(1)
    );
    assert_eq!(Dialog::Confirm(ConfirmDialog::quit()).anchor_panel(), None);
}

#[test]
fn drive_dialog_local_button_is_default() {
    // No drives, no sessions: the always-present Local button is the default.
    let mut d = DriveDialog::new(0, vec![], None, None, vec![], true);
    assert!(matches!(
        d.handle_key(key(KeyCode::Enter)),
        DialogResult::Submit(Submit::GoLocal(0))
    ));
}

#[test]
fn drive_dialog_connection_buttons() {
    // No drives, no sessions: Local then SFTP/FTP/SCP. End lands on SCP.
    let mut d = DriveDialog::new(0, vec![], None, None, vec![], true);
    d.handle_key(key(KeyCode::End));
    assert!(matches!(
        d.handle_key(key(KeyCode::Enter)),
        DialogResult::Submit(Submit::OpenConnect(0, Protocol::Scp))
    ));
    // Right off Local → SFTP.
    let mut d = DriveDialog::new(1, vec![], None, None, vec![], true);
    d.handle_key(key(KeyCode::Right));
    assert!(matches!(
        d.handle_key(key(KeyCode::Enter)),
        DialogResult::Submit(Submit::OpenConnect(1, Protocol::Sftp))
    ));
}

#[test]
fn drive_dialog_sessions_switch_and_disconnect() {
    // One open session: a switch button + a ✕ disconnect button are present.
    let sessions = vec![(3usize, "sftp://u@host".to_string())];
    // current_session highlights the Session button, so Enter switches to it.
    let mut d = DriveDialog::new(0, vec![], None, Some(3), sessions.clone(), true);
    assert!(matches!(
        d.handle_key(key(KeyCode::Enter)),
        DialogResult::Submit(Submit::SwitchSession(0, 3))
    ));
    // Right of the highlighted session button → its ✕ (ask-disconnect).
    let mut d = DriveDialog::new(0, vec![], None, Some(3), sessions, true);
    d.handle_key(key(KeyCode::Right));
    assert!(matches!(
        d.handle_key(key(KeyCode::Enter)),
        DialogResult::Submit(Submit::AskDisconnectSession(3))
    ));
}

#[test]
fn drive_dialog_hides_remote_when_not_show_remote() {
    // Other panel is remote → show_remote=false: sessions and connect buttons are
    // gone; only Local (+ any drives) remains, so every position is Local.
    let sessions = vec![(1usize, "sftp://u@host".to_string())];
    let mut d = DriveDialog::new(0, vec![], None, None, sessions, false);
    // Home and End both land on Local (the sole item).
    d.handle_key(key(KeyCode::End));
    assert!(matches!(
        d.handle_key(key(KeyCode::Enter)),
        DialogResult::Submit(Submit::GoLocal(0))
    ));
    d.handle_key(key(KeyCode::Home));
    assert!(matches!(
        d.handle_key(key(KeyCode::Enter)),
        DialogResult::Submit(Submit::GoLocal(0))
    ));
}

#[test]
fn drive_dialog_letter_jumps_and_highlights_current() {
    // Windows-style: drive letters present, current drive highlighted.
    let mut d = DriveDialog::new(0, vec!['A', 'C', 'D', 'Z'], Some('C'), None, vec![], true);
    // A drive letter jumps straight to that drive.
    match d.handle_key(key(KeyCode::Char('z'))) {
        DialogResult::Submit(Submit::SetDrive(0, c)) => assert_eq!(c, 'Z'),
        _ => panic!("expected SetDrive Z"),
    }
    // Enter activates the highlighted (current) drive C.
    let mut d = DriveDialog::new(0, vec!['A', 'C', 'D', 'Z'], Some('C'), None, vec![], true);
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
    t.draw(|f| d.render(f, area, &theme, None)).unwrap();
    // Box centered(80x24, 64, 8): origin (8,8); inner (9,9,62,6); button row 14.
    assert!(
        matches!(d.handle_click(area, 40, 14), DialogResult::Abort(7)),
        "clicking the scan dialog's Abort button cancels it"
    );
    assert!(matches!(d.handle_click(area, 40, 10), DialogResult::None), "a click elsewhere does nothing");

    // A determinate (copy) progress dialog ignores clicks entirely.
    let mut c = Dialog::Progress(ProgressDialog::new(8, "Copying"));
    t.draw(|f| c.render(f, area, &theme, None)).unwrap();
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
    // The Filesystem field is now an Enter-opened dropdown; Tab past it so Enter
    // submits with the default (FAT32) rather than opening the list.
    d.handle_key(key(KeyCode::Tab));
    match d.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::Format(spec)) => {
            assert_eq!(spec.dev, "/dev/sdb1");
            assert_eq!(spec.fs, crate::mount::FsType::Fat32); // default choice
        }
        _ => panic!("expected Format submit"),
    }
}

#[test]
fn choice_dropdown_cursor_moves_freely() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let theme = crate::ui::theme::Theme::mc();
    let mut d = FormDialog::format("/dev/sdb1".into()); // 8 filesystem options
    let mut t = Terminal::new(TestBackend::new(60, 24)).unwrap();
    macro_rules! render {
        () => {
            t.draw(|f| d.render(f, f.area(), &theme, None)).unwrap();
        };
    }
    d.handle_key(key(KeyCode::Enter)); // open the dropdown (sel = 0)
    render!();
    assert_eq!(d.open_choice_state(), Some((0, 0)));
    // Scroll to the last option so the window has scrolled (top > 0).
    for _ in 0..7 {
        d.handle_key(key(KeyCode::Down));
        render!();
    }
    let (sel_hi, top_hi) = d.open_choice_state().unwrap();
    assert_eq!(sel_hi, 7);
    assert!(top_hi > 0, "the window scrolled to keep the highlight visible");
    // Moving up moves the highlight but does NOT scroll the window (free cursor,
    // not pinned to the bottom edge) until it reaches the top of the window.
    d.handle_key(key(KeyCode::Up));
    render!();
    let (sel_up, top_up) = d.open_choice_state().unwrap();
    assert_eq!(sel_up, 6, "the highlight moved up one");
    assert_eq!(top_up, top_hi, "the window did not scroll — the cursor moved freely");
}

#[test]
fn choice_dropdown_opens_navigates_and_selects() {
    let mut d = FormDialog::format("/dev/sdb1".into());
    // Enter on the focused Filesystem Choice opens its dropdown (does not submit).
    assert!(matches!(d.handle_key(key(KeyCode::Enter)), DialogResult::None));
    // Down highlights the next option, Enter confirms it (FsType::ALL = FAT32,
    // NTFS, …, so index 1 is NTFS).
    d.handle_key(key(KeyCode::Down));
    assert!(matches!(d.handle_key(key(KeyCode::Enter)), DialogResult::None));
    // Tab off the (now closed) Choice, then Enter submits with the chosen NTFS.
    d.handle_key(key(KeyCode::Tab));
    match d.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::Format(spec)) => {
            assert_eq!(spec.fs, crate::mount::FsType::Ntfs);
        }
        _ => panic!("expected Format submit with the chosen filesystem"),
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

    t.draw(|f| d.render(f, f.area(), &theme, None)).unwrap();
    assert!(dump(&t).contains('▼'), "chevron shown on the host field");

    d.handle_key(key(KeyCode::Down)); // open the dropdown
    t.draw(|f| d.render(f, f.area(), &theme, None)).unwrap();
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
    t.draw(|f| d.render(f, f.area(), &theme, None)).unwrap();
    assert!(find(&t, "[ ] Case sensitive").is_some(), "checkbox starts unchecked");
    assert!(find(&t, "unchanged").is_some(), "case starts unchanged");

    // Clicking the checkbox toggles it on.
    let (cx, cy) = find(&t, "Case sensitive").unwrap();
    d.handle_click(area, cx, cy);
    t.draw(|f| d.render(f, f.area(), &theme, None)).unwrap();
    assert!(find(&t, "[x] Case sensitive").is_some(), "click toggled the checkbox on");

    // Clicking the case chooser cycles unchanged → lowercase.
    let (kx, ky) = find(&t, "Case:").unwrap();
    d.handle_click(area, kx, ky);
    t.draw(|f| d.render(f, f.area(), &theme, None)).unwrap();
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




#[test]
fn form_ok_cancel_buttons_are_keyboard_navigable() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent};
    let cfg = crate::config::Config::default();
    let theme = crate::ui::theme::Theme::mc();
    let area = ratatui::layout::Rect::new(0, 0, 80, 24);

    // Confirmations form has 5 fields; slot 5 = OK, slot 6 = Cancel.
    let render_has = |d: &mut FormDialog, needle: &str| {
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        t.draw(|f| d.render(f, area, &theme, None)).unwrap();
        let buf = t.backend().buffer();
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width { s.push_str(buf[(x, y)].symbol()); }
        }
        s.contains(needle)
    };

    // Tab down onto OK → it renders highlighted, and Enter submits.
    let mut d = FormDialog::confirmations(&cfg);
    for _ in 0..5 { let _ = d.handle_key(KeyEvent::from(KeyCode::Tab)); }
    assert!(render_has(&mut d, "< OK >"), "OK should highlight when focused");
    assert!(
        matches!(d.handle_key(KeyEvent::from(KeyCode::Enter)), DialogResult::Submit(_)),
        "Enter on OK submits"
    );

    // Tab once more onto Cancel → Enter cancels.
    let mut d = FormDialog::confirmations(&cfg);
    for _ in 0..6 { let _ = d.handle_key(KeyEvent::from(KeyCode::Tab)); }
    assert!(render_has(&mut d, "< Cancel >"), "Cancel should highlight when focused");
    assert!(
        matches!(d.handle_key(KeyEvent::from(KeyCode::Enter)), DialogResult::Cancel),
        "Enter on Cancel cancels"
    );

    // Left/Right toggles between the two buttons.
    let mut d = FormDialog::confirmations(&cfg);
    for _ in 0..5 { let _ = d.handle_key(KeyEvent::from(KeyCode::Tab)); } // OK
    let _ = d.handle_key(KeyEvent::from(KeyCode::Right));
    assert!(render_has(&mut d, "< Cancel >"), "Right moves OK→Cancel");
    let _ = d.handle_key(KeyEvent::from(KeyCode::Left));
    assert!(render_has(&mut d, "< OK >"), "Left moves Cancel→OK");
}





#[test]
fn settings_dialog_renders_three_group_boxes() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    let cfg = crate::config::Config::default();
    let theme = crate::ui::theme::Theme::default();
    let area = Rect::new(0, 0, 80, 24);
    let mut d = FormDialog::settings(&cfg, true);

    let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
    t.draw(|f| d.render(f, area, &theme, None)).unwrap();
    let buf = t.backend().buffer();
    let mut s = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            s.push_str(buf[(x, y)].symbol());
        }
        s.push('\n');
    }

    // The three group titles are drawn as sub-box headers.
    for title in ["Language", "Edit/View", "Visual"] {
        assert!(s.contains(title), "settings should show the '{title}' group box");
    }
    // A representative field from each group is present.
    for field in ["Reshape RTL text", "External editor", "Theme", "Graphics"] {
        assert!(s.contains(field), "settings should show the '{field}' field");
    }
}

#[test]
fn form_ok_button_click_submits_over_a_focused_choice_field() {
    use ratatui::layout::Rect;
    let cfg = crate::config::Config::default();
    let area = Rect::new(0, 0, 80, 24);
    // Settings' first field (the default focus) is the Language *Choice*: a bare
    // Enter there opens its dropdown. A mouse click on OK must still submit the
    // form rather than acting on that field. Geometry mirrors `outer_rect` for
    // the grouped settings box (three group boxes + spacer + hint + border).
    let w = 72u16.min(area.width - 4);
    let h = 22u16;
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) / 2;
    let button_row = y + h - 2;

    let mut dlg = Dialog::Form(FormDialog::settings(&cfg, true));
    match dlg.handle_click(area, x + 5, button_row) {
        DialogResult::Submit(Submit::Settings(_)) => {}
        _ => panic!("clicking OK should submit the settings form"),
    }

    // A click on the Cancel (right) half cancels.
    let mut dlg = Dialog::Form(FormDialog::settings(&cfg, true));
    assert!(matches!(
        dlg.handle_click(area, x + w - 5, button_row),
        DialogResult::Cancel
    ));

    // A click that isn't on the button row leaves the dialog open (here, the
    // Theme choice row) — it must not submit or cancel.
    let mut dlg = Dialog::Form(FormDialog::settings(&cfg, true));
    assert!(matches!(
        dlg.handle_click(area, x + 5, y + 2),
        DialogResult::None
    ));
}

#[test]
fn checksum_form_submits_algorithm_and_comparison() {
    use crate::util::checksum::ChecksumKind;
    // The form defaults to SHA-256 with no comparison; the file name is in the title.
    let mut d = FormDialog::checksum(VfsPath::local("/tmp/image.iso"));
    assert!(d.title.contains("image.iso"), "the file name is shown in the title");
    // Choose MD5 (Algorithm is the first field): Enter opens the dropdown, Up
    // moves from SHA-256 (idx 3) toward the top; two Ups land on MD5 (idx 1).
    d.handle_key(key(KeyCode::Enter));
    d.handle_key(key(KeyCode::Up));
    d.handle_key(key(KeyCode::Up));
    d.handle_key(key(KeyCode::Enter)); // pick MD5, closes the dropdown
    // Tab to the comparison field and type an expected digest.
    d.handle_key(key(KeyCode::Tab));
    for c in "abc123".chars() {
        d.handle_key(key(KeyCode::Char(c)));
    }
    match d.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::Checksum { path, kind, expected }) => {
            assert_eq!(path.file_name(), "image.iso");
            assert_eq!(kind, ChecksumKind::Md5);
            assert_eq!(expected, "abc123");
        }
        _ => panic!("expected a Checksum submit"),
    }
}

#[test]
fn checksum_result_dialog_shows_verdict_and_digest() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use crate::util::checksum::{ChecksumKind, ChecksumReport, normalize_expected};
    let theme = crate::ui::theme::Theme::mc();
    let area = Rect::new(0, 0, 90, 24);
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
    let digest = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    // A matching comparison → a green ✓ MATCH verdict, and the digest is shown.
    let mut d = Dialog::ChecksumResult(ChecksumResultDialog::new(ChecksumReport {
        kind: ChecksumKind::Sha256,
        name: "abc.txt".into(),
        digest: digest.into(),
        expected: normalize_expected(&digest.to_uppercase()),
    }));
    let mut t = Terminal::new(TestBackend::new(90, 24)).unwrap();
    t.draw(|f| d.render(f, area, &theme, None)).unwrap();
    let s = dump(&t);
    assert!(s.contains("abc.txt"), "file name shown");
    assert!(s.contains("SHA-256"), "algorithm shown");
    assert!(s.contains(digest), "computed digest shown");
    assert!(s.contains("MATCH"), "match verdict shown");

    // A wrong comparison → a ✗ MISMATCH verdict plus the expected value.
    let mut d = Dialog::ChecksumResult(ChecksumResultDialog::new(ChecksumReport {
        kind: ChecksumKind::Sha256,
        name: "abc.txt".into(),
        digest: digest.into(),
        expected: normalize_expected("deadbeef"),
    }));
    t.draw(|f| d.render(f, area, &theme, None)).unwrap();
    let s = dump(&t);
    assert!(s.contains("MISMATCH"), "mismatch verdict shown");
    assert!(s.contains("deadbeef"), "expected value shown on mismatch");

    // No comparison supplied → the digest is shown but no verdict.
    let mut d = Dialog::ChecksumResult(ChecksumResultDialog::new(ChecksumReport {
        kind: ChecksumKind::Sha256,
        name: "abc.txt".into(),
        digest: digest.into(),
        expected: None,
    }));
    t.draw(|f| d.render(f, area, &theme, None)).unwrap();
    let s = dump(&t);
    assert!(s.contains(digest), "digest shown without a comparison");
    assert!(!s.contains("MATCH"), "no verdict without a comparison");

    // The dialog is dismissed only via its OK button, not by any key or click.
    let find = |t: &Terminal<TestBackend>, needle: &str| -> Option<(u16, u16)> {
        let b = t.backend().buffer();
        for y in 0..b.area.height {
            let mut rowtext = String::new();
            for x in 0..b.area.width {
                rowtext.push_str(b[(x, y)].symbol());
            }
            if let Some(byte) = rowtext.find(needle) {
                return Some((rowtext[..byte].chars().count() as u16, y));
            }
        }
        None
    };
    // A non-activating key does nothing; Enter (or Esc) closes.
    assert!(matches!(d.handle_key(key(KeyCode::Char('x'))), DialogResult::None));
    assert!(matches!(d.handle_key(key(KeyCode::Esc)), DialogResult::Cancel));
    assert!(matches!(d.handle_key(key(KeyCode::Enter)), DialogResult::Cancel));
    // A click off the button is ignored; a click on it closes.
    assert!(matches!(d.handle_click(area, 0, 0), DialogResult::None));
    let (ok_col, ok_row) = find(&t, "OK").expect("OK button rendered");
    assert!(matches!(d.handle_click(area, ok_col, ok_row), DialogResult::Cancel));
}

#[test]
fn graphical_buttons_paint_and_stay_clickable() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use crate::ui::graphics::Gfx;
    let theme = crate::ui::theme::Theme::mc();
    let area = Rect::new(0, 0, 80, 24);
    let mut gfx = Gfx::test_halfblocks();

    // A Yes/No confirmation rendered through the graphics path.
    let mut d = Dialog::Confirm(ConfirmDialog::delete(vec![VfsPath::local("/tmp/a")]));
    let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
    t.draw(|f| d.render(f, area, &theme, Some(&mut gfx))).unwrap();

    let image_cells = {
        let b = t.backend().buffer();
        (0..b.area.height)
            .flat_map(|y| (0..b.area.width).map(move |x| (x, y)))
            .filter(|&(x, y)| matches!(b[(x, y)].symbol(), "\u{2580}" | "\u{2584}"))
            .count()
    };
    // The whole button (chrome + baked label) paints as graphics (half-block
    // image cells); the label is baked into the pixels, not cell text, so it
    // survives every graphics protocol.
    assert!(image_cells > 0, "graphical buttons should paint image cells");

    // Hit zones are unchanged by the graphics path: the Yes button (first button,
    // centered on the last-but-one interior row of the centered 54×7 box) still
    // submits. Box: centered(80×24,54,7) → x=13,y=8; button row y=13; Yes at x≈32.
    assert!(
        matches!(d.handle_click(area, 34, 13), DialogResult::Submit(Submit::Delete(_))),
        "clicking the graphical Yes button still submits the delete"
    );
}

#[test]
fn button_labels_fall_back_to_text_for_unrenderable_scripts() {
    use super::widgets::all_renderable;
    // Scripts the bundled graphics font covers → graphical buttons.
    assert!(all_renderable(&["OK", "Cancel"]));
    assert!(all_renderable(&["ОК", "Отмена"])); // Russian (Cyrillic)
    assert!(all_renderable(&["Άκυρο"])); // Greek
    // Scripts it doesn't cover → the row falls back to regular text buttons, so
    // the terminal font renders them (no baked "tofu" boxes).
    assert!(!all_renderable(&["موافق", "إلغاء"])); // Arabic (ar.toml OK/Cancel)
    assert!(!all_renderable(&["キャンセル"])); // Japanese (ja.toml Cancel)
    assert!(!all_renderable(&["OK", "取消"])); // any unsupported member fails the row
}

#[test]
fn find_dialog_mouse_toggles_focuses_and_submits() {
    // Box: centered(80x24, 66, 13) → x=7, y=5; inner_x=8, inner.y=6; half=32.
    // Rows within: fields at 7/10/12, checkbox rows at 14/15, OK/Cancel at 16.
    let area = Rect::new(0, 0, 80, 24);
    let mut d = FindDialog::new("/tmp".into());
    // "Find recursively" (row 14, left half) toggles off; "Case sensitive"
    // (row 14, right half) toggles on.
    assert!(matches!(d.handle_click(area, 12, 14), DialogResult::None));
    assert!(matches!(d.handle_click(area, 50, 14), DialogResult::None));
    // Clicking OK (left half of the button row) submits with the updated flags.
    match d.handle_click(area, 20, 16) {
        DialogResult::Submit(Submit::Find(p)) => {
            assert!(!p.recursive, "recursively was unchecked by the click");
            assert!(p.case_sensitive, "case-sensitive was checked by the click");
            assert_eq!(p.file_name, "*");
        }
        _ => panic!("clicking OK should submit a Find"),
    }
    // The right half of the button row cancels; a click outside does nothing.
    let mut d = FindDialog::new("/tmp".into());
    assert!(matches!(d.handle_click(area, 60, 16), DialogResult::Cancel));
    assert!(matches!(d.handle_click(area, 0, 0), DialogResult::None));
    // Clicking the Content field (row 12) focuses it, so typing edits `content`.
    let mut d = FindDialog::new("/tmp".into());
    assert!(matches!(d.handle_click(area, 20, 12), DialogResult::None));
    d.handle_key(key(KeyCode::Char('x')));
    match d.handle_key(key(KeyCode::Enter)) {
        DialogResult::Submit(Submit::Find(p)) => assert_eq!(p.content, "x"),
        _ => panic!("expected a Find submit"),
    }
}
