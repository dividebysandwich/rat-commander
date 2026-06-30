//! Disk-manager cluster: mount/format/flash/image and privileged commands.

use super::*;

impl AppState {
    /// Open the full-screen process explorer.
    pub(in crate::app::state) fn open_proc_explorer(&mut self) {
        self.procview = Some(ProcView::new());
    }

    // -- Disk manager ------------------------------------------------------

    /// Prompt for the path to mount `device` at (suggesting `/mnt/<name>`).
    pub(in crate::app::state) fn prompt_mount_path(&mut self, device: String) {
        let base = device.rsplit('/').next().unwrap_or("disk");
        let suggest = format!("/mnt/{base}");
        self.dialog = Some(Dialog::Input(InputDialog::new(
            "Mount",
            format!("Mount {device} at:"),
            suggest,
            InputPurpose::MountPath(device),
        )));
    }

    /// Mount `device` at `path` (optionally creating the mount point first),
    /// escalating with sudo when not running as root.
    pub(in crate::app::state) async fn do_mount(&mut self, device: String, path: String, create: bool) {
        let q = crate::mount::shell_quote;
        let cmd = if create {
            format!("mkdir -p {p} && mount {d} {p}", p = q(&path), d = q(&device))
        } else {
            format!("mount {} {}", q(&device), q(&path))
        };
        let busy = format!("Mounting {device}...");
        self.run_privileged(cmd, format!("Mounted {device} at {path}"), busy).await;
    }

    /// Apply a [`MountSignal`] produced by the disk manager (from a key or a
    /// mouse gesture): open the relevant action dialog, unmount, or close.
    pub(in crate::app::state) async fn apply_mount_signal(&mut self, sig: MountSignal) {
        match sig {
            MountSignal::Stay => {}
            MountSignal::Close => self.mountview = None,
            MountSignal::DeviceMenu(d) => {
                self.dialog = Some(Dialog::Confirm(ConfirmDialog::device_menu(&d)));
            }
            MountSignal::MountMenu(mountpoint) => {
                self.dialog = Some(Dialog::Confirm(ConfirmDialog::mount_menu(&mountpoint)));
            }
            MountSignal::Unmount(mountpoint) => self.ask_unmount(mountpoint).await,
        }
    }

    /// Unmount `mountpoint`, prompting for confirmation when enabled. Essential
    /// system mount points always raise a loud red warning, regardless of the
    /// confirmation setting.
    pub(in crate::app::state) async fn ask_unmount(&mut self, mountpoint: String) {
        if crate::mount::is_essential_mount(&mountpoint) {
            self.dialog = Some(Dialog::Confirm(ConfirmDialog::unmount_danger(&mountpoint)));
        } else if self.config.confirm_unmount {
            self.dialog = Some(Dialog::Confirm(ConfirmDialog::unmount(&mountpoint)));
        } else {
            self.do_unmount(mountpoint).await;
        }
    }

    /// Unmount the filesystem at `mountpoint`.
    pub(in crate::app::state) async fn do_unmount(&mut self, mountpoint: String) {
        let cmd = format!("umount {}", crate::mount::shell_quote(&mountpoint));
        let busy = format!("Unmounting {mountpoint}...");
        self.run_privileged(cmd, format!("Unmounted {mountpoint}"), busy).await;
    }

    /// Flush filesystem buffers for `mountpoint` (no privileges needed).
    pub(in crate::app::state) async fn do_sync(&mut self, mountpoint: String) {
        let cmd = format!("sync -f {}", crate::mount::shell_quote(&mountpoint));
        let result = crate::mount::run_shell(&cmd).await;
        self.finish_privileged(result, format!("Synced {mountpoint}"));
    }

    /// Run a confirmed format request (creating the chosen filesystem).
    pub(in crate::app::state) async fn do_format(&mut self, spec: crate::mount::FormatSpec) {
        let ok = format!("Formatted {} as {}", spec.dev, spec.fs.label());
        let busy = format!("Formatting {} as {}...", spec.dev, spec.fs.label());
        let cmd = crate::mount::format_command(&spec);
        self.run_privileged(cmd, ok, busy).await;
    }

    /// Run a privileged `sh -c` command: directly when root, via non-interactive
    /// sudo when possible, otherwise queue it and prompt for a sudo password.
    /// The command runs on a background task (showing `busy` meanwhile) so the
    /// UI keeps redrawing while a slow operation like `mkfs` runs.
    async fn run_privileged(&mut self, cmd: String, ok_msg: String, busy: String) {
        if crate::mount::is_root() {
            self.spawn_privileged(PrivExec::Root(cmd), ok_msg, busy);
        } else if crate::mount::sudo_can_noninteractive().await {
            self.spawn_privileged(PrivExec::SudoNonInteractive(cmd), ok_msg, busy);
        } else {
            // Need a password: stash the command and prompt for it.
            self.pending_sudo = Some(PendingPriv { cmd, ok_msg, busy });
            self.dialog = Some(Dialog::Input(InputDialog::password(
                "Authentication required",
                "Enter sudo password:",
                InputPurpose::SudoPassword,
            )));
        }
    }

    /// Run the queued privileged command with the entered sudo `password`.
    pub(in crate::app::state) async fn run_pending_sudo(&mut self, password: String) {
        let Some(p) = self.pending_sudo.take() else {
            return;
        };
        self.spawn_privileged(PrivExec::SudoPassword(p.cmd, password), p.ok_msg, p.busy);
    }

    /// Show a busy spinner and run a privileged command on a background task,
    /// reporting its result back through [`AppEvent::PrivilegedDone`].
    fn spawn_privileged(&mut self, exec: PrivExec, ok_msg: String, busy: String) {
        self.dialog = Some(Dialog::Busy(BusyDialog::new("Please wait", busy)));
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = match exec {
                PrivExec::Root(cmd) => crate::mount::run_shell(&cmd).await,
                PrivExec::SudoNonInteractive(cmd) => {
                    crate::mount::run_sudo_noninteractive(&cmd).await
                }
                PrivExec::SudoPassword(cmd, pw) => crate::mount::run_sudo_password(&cmd, &pw).await,
            };
            let _ = tx.send(AppEvent::PrivilegedDone { ok_msg, result }).await;
        });
    }

    /// Report the outcome of a privileged op on the mounter's status line and
    /// refresh its lists.
    pub(in crate::app::state) fn finish_privileged(&mut self, result: Result<(), String>, ok_msg: String) {
        match self.mountview.as_mut() {
            Some(mv) => {
                mv.refresh();
                mv.status = match result {
                    Ok(()) => ok_msg,
                    Err(e) => format!("Error: {e}"),
                };
            }
            None => {
                if let Err(e) = result {
                    self.show_error(e);
                }
            }
        }
    }

    // -- Image flashing ----------------------------------------------------

    /// If the cursor is on a local image file, open the flash device picker and
    /// return `true`; otherwise `false` (so the caller opens it normally).
    #[cfg(target_os = "linux")]
    pub(in crate::app::state) fn try_flash_under_cursor(&mut self) -> bool {
        let p = &self.panels[self.active];
        if p.cwd.scheme != "file" {
            return false;
        }
        let Some(e) = p.current_entry() else {
            return false;
        };
        if e.kind != VfsKind::File || !crate::flash::is_image_file(&e.name) {
            return false;
        }
        let name = e.name.clone();
        let size = e.size;
        let path = p.cwd.path.join(&e.name);
        self.start_flash_from_panel(name, path, size);
        true
    }

    /// Open the flash target picker for an image chosen in a file panel.
    fn start_flash_from_panel(&mut self, name: String, path: PathBuf, size: u64) {
        let devices = crate::mount::list_block_devices(&crate::mount::list_mounts());
        if devices.is_empty() {
            return self.show_error("No block devices available to flash to");
        }
        self.dialog = Some(Dialog::FlashTarget(FlashTargetDialog::new(
            path, name, size, devices, None,
        )));
    }

    /// Open the file browser to pick an image to flash onto `target` (from the
    /// disk manager). Starts in the active panel's directory, or home.
    pub(in crate::app::state) fn open_flash_browser(&mut self, target: crate::flash::FlashTarget) {
        let start = if self.panels[self.active].cwd.scheme == "file" {
            self.panels[self.active].cwd.path.clone()
        } else {
            home_dir()
        };
        self.dialog = Some(Dialog::FileBrowser(FileBrowserDialog::new(target, start)));
    }

    /// An image was picked in the browser: stat it, guard the size, and proceed
    /// to the confirmation flow (red warning first for non-removable targets).
    pub(in crate::app::state) fn flash_picked_image(&mut self, path: PathBuf, target: crate::flash::FlashTarget) {
        let size = match std::fs::metadata(&path) {
            Ok(m) => m.len(),
            Err(e) => return self.show_error(format!("Cannot read image: {e}")),
        };
        if size == 0 {
            return self.show_error("The chosen image is empty");
        }
        if target.size < size {
            return self
                .show_error(format!("Device {} is too small for this image", target.dev));
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let spec = crate::flash::FlashSpec { image_path: path, image_name: name, image_size: size, target };
        self.dialog = Some(Dialog::Confirm(if spec.target.removable {
            ConfirmDialog::flash_confirm(spec)
        } else {
            ConfirmDialog::flash_danger(spec)
        }));
    }

    /// Determine privileges, then start (or queue, pending a password) a flash.
    pub(in crate::app::state) async fn start_flash(&mut self, spec: crate::flash::FlashSpec) {
        // Re-check the size guard in case the device list was stale.
        if spec.target.size < spec.image_size {
            return self
                .show_error(format!("Device {} is too small for this image", spec.target.dev));
        }
        if crate::mount::is_root() {
            self.begin_flash(spec, crate::flash::FlashAuth::Root);
        } else if crate::mount::sudo_can_noninteractive().await {
            self.begin_flash(spec, crate::flash::FlashAuth::SudoNonInteractive);
        } else {
            self.pending_flash = Some(spec);
            self.dialog = Some(Dialog::Input(InputDialog::password(
                "Authentication required",
                "Enter sudo password:",
                InputPurpose::FlashPassword,
            )));
        }
    }

    /// Spawn the flash task and show its progress dialog.
    pub(in crate::app::state) fn begin_flash(&mut self, spec: crate::flash::FlashSpec, auth: crate::flash::FlashAuth) {
        let id = self.next_task_id;
        self.next_task_id += 1;
        let cancel = crate::flash::spawn_flash(id, spec, auth, self.tx.clone());
        self.flash_tasks.insert(id, cancel);
        self.dialog = Some(Dialog::Progress(ProgressDialog::new(id, "Flashing")));
    }

    /// Open the "save image" browser for reading `target` out to a file. Starts
    /// in the active panel's directory, or home.
    pub(in crate::app::state) fn open_image_browser(&mut self, target: crate::flash::FlashTarget) {
        let start = if self.panels[self.active].cwd.scheme == "file" {
            self.panels[self.active].cwd.path.clone()
        } else {
            home_dir()
        };
        self.dialog = Some(Dialog::ImageSave(ImageSaveDialog::new(target, start)));
    }

    /// Determine privileges, then start (or queue, pending a password) imaging.
    pub(in crate::app::state) async fn start_image(&mut self, spec: crate::flash::ImageSpec) {
        if crate::mount::is_root() {
            self.begin_image(spec, crate::flash::FlashAuth::Root);
        } else if crate::mount::sudo_can_noninteractive().await {
            self.begin_image(spec, crate::flash::FlashAuth::SudoNonInteractive);
        } else {
            self.pending_image = Some(spec);
            self.dialog = Some(Dialog::Input(InputDialog::password(
                "Authentication required",
                "Enter sudo password:",
                InputPurpose::ImagePassword,
            )));
        }
    }

    /// Spawn the imaging task and show its progress dialog.
    pub(in crate::app::state) fn begin_image(&mut self, spec: crate::flash::ImageSpec, auth: crate::flash::FlashAuth) {
        let id = self.next_task_id;
        self.next_task_id += 1;
        let cancel = crate::flash::spawn_image(id, spec, auth, self.tx.clone());
        self.flash_tasks.insert(id, cancel);
        self.dialog = Some(Dialog::Progress(ProgressDialog::new(id, "Imaging")));
    }

    /// Open the full-screen disk-usage explorer at the active panel's directory.
    pub(in crate::app::state) fn open_disk_explorer(&mut self) {
        let p = &self.panels[self.active];
        let cwd = if p.cwd.scheme == "file" {
            p.cwd.path.clone()
        } else {
            std::env::current_dir().unwrap_or_else(|_| home_dir())
        };
        self.diskview = Some(DiskView::new(cwd));
        self.start_disk_scan();
    }

    /// Kick off a background scan of the disk explorer's current directory.
    pub(in crate::app::state) fn start_disk_scan(&mut self) {
        let Some(dv) = self.diskview.as_mut() else {
            return;
        };
        dv.generation = dv.generation.wrapping_add(1);
        dv.scanning = true;
        dv.scan_done = 0;
        dv.scan_total = 0;
        dv.entries.clear();
        dv.selected = 0;
        let generation = dv.generation;
        let cwd = dv.cwd.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let txp = tx.clone();
            let entries = tokio::task::spawn_blocking(move || {
                crate::disk::scan_dir_with(&cwd, |done, total| {
                    // Progress is advisory; drop updates if the channel is full.
                    let _ = txp.try_send(AppEvent::DiskScanProgress { generation, done, total });
                })
            })
            .await
            .unwrap_or_default();
            let _ = tx.send(AppEvent::DiskScanned { generation, entries }).await;
        });
    }

    /// Kill a process (from the explorer), then refresh the listing.
    pub(in crate::app::state) fn kill_process(&mut self, pid: i32, force: bool) {
        #[cfg(unix)]
        {
            use nix::sys::signal::{Signal, kill};
            use nix::unistd::Pid;
            let sig = if force { Signal::SIGKILL } else { Signal::SIGTERM };
            let _ = kill(Pid::from_raw(pid), sig);
        }
        #[cfg(not(unix))]
        {
            let _ = (pid, force);
        }
        if let Some(pv) = self.procview.as_mut() {
            pv.refresh();
        }
    }

}
