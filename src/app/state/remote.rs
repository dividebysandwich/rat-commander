//! Remote connections, persistent sessions, and Windows drive selection.

use super::*;

impl AppState {
    // -- One-remote guard & bookkeeping ------------------------------------

    /// True when the *other* panel (not `side`) is currently on a remote
    /// session. Used to forbid remote-to-remote setups: one panel must stay
    /// local so we never have to copy from one server directly to another.
    pub(in crate::app::state) fn other_panel_is_remote(&self, side: usize) -> bool {
        self.panels[1 - side].cwd.is_remote()
    }

    /// The open sessions as `(id, label)` pairs, for the drive picker and the
    /// Left/Right panel menus.
    pub(in crate::app::state) fn session_list(&self) -> Vec<(usize, String)> {
        self.sessions.iter().map(|s| (s.id, s.label.clone())).collect()
    }

    /// Whether each panel `[left, right]` is currently on a remote directory,
    /// used to grey out the "Go local" menu item when a panel is already local.
    pub(in crate::app::state) fn side_remote(&self) -> [bool; 2] {
        [self.panels[0].cwd.is_remote(), self.panels[1].cwd.is_remote()]
    }

    /// Open the Yes/No confirmation for disconnecting a remote session.
    pub(in crate::app::state) fn ask_disconnect_session(&mut self, id: usize) {
        let label = self
            .sessions
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.label.clone())
            .unwrap_or_default();
        self.dialog = Some(Dialog::Confirm(ConfirmDialog::disconnect_session(id, &label)));
    }

    /// Snapshot the panel's current location before a transition: a remote cwd
    /// is saved back into its session (so switching back returns to the same
    /// directory), while a local cwd is remembered in `last_local_cwd[side]`
    /// for the "Local" button.
    fn snapshot_session_cwd(&mut self, side: usize) {
        let cwd = self.panels[side].cwd.clone();
        if cwd.is_remote() {
            if let Some(s) = self.sessions.iter_mut().find(|s| s.scheme == cwd.scheme) {
                s.cwd = cwd;
            }
        } else {
            self.last_local_cwd[side] = cwd;
        }
    }

    // -- Remote connections ------------------------------------------------

    pub(in crate::app::state) async fn connect_remote(&mut self, side: usize, creds: RemoteCreds) {
        // One panel must stay local — refuse a second remote panel.
        if self.other_panel_is_remote(side) {
            self.show_error(
                "The other panel is already on a remote connection. Return it to \
                 Local first — one panel must stay local."
                    .to_string(),
            );
            return;
        }
        match crate::vfs::remote::connect(&creds).await {
            Ok(conn) => {
                // Remember where this panel was (local dir, or a prior session).
                self.snapshot_session_cwd(side);

                let id = self.next_session_id;
                self.next_session_id += 1;
                let scheme = format!("{}-{}", creds.protocol.scheme_prefix(), id);
                self.registry.register(scheme.clone(), conn.backend.clone());
                let cwd = VfsPath {
                    scheme: scheme.clone(),
                    path: PathBuf::from(&conn.root),
                    container: None,
                };
                let p = &mut self.panels[side];
                p.cwd = cwd.clone();
                p.backend = conn.backend;
                p.selection.clear();
                let _ = p.reload().await;

                // Record the session so it persists and can be switched back to
                // like a drive letter.
                self.sessions.push(RemoteSession {
                    id,
                    scheme,
                    label: conn.label,
                    cwd,
                    creds: creds.clone(),
                });

                // Remember this server (without the password) for the dropdown.
                self.config.add_recent_remote(crate::config::RemoteHistoryEntry {
                    protocol: creds.protocol.scheme_prefix().to_string(),
                    host: creds.host,
                    port: creds.port,
                    user: creds.user,
                    path: creds.path,
                });
                let _ = self.config.save();
            }
            Err(e) => self.show_error(format!("Connection failed: {e}")),
        }
    }

    /// When a transfer is sent to the background, open a fresh browsing
    /// connection for any panel sitting on an **FTP** session that the transfer
    /// uses — FTP holds a single control connection for the whole transfer, so
    /// browsing would otherwise block. SFTP/SCP multiplex safely and are left
    /// alone. The transfer keeps its own backend `Arc`, so re-registering schemes
    /// never disturbs it.
    pub(in crate::app::state) async fn background_reconnect_ftp(&mut self, id: TaskId) {
        let schemes = match self.task_progress.get(&id) {
            Some(t) if !t.schemes.is_empty() => t.schemes.clone(),
            _ => return,
        };
        for side in 0..2 {
            let scheme = self.panels[side].cwd.scheme.clone();
            if !schemes.contains(&scheme) {
                continue;
            }
            let Some(sess_idx) = self.sessions.iter().position(|s| s.scheme == scheme) else {
                continue;
            };
            if self.sessions[sess_idx].creds.protocol != crate::vfs::remote::Protocol::Ftp {
                continue;
            }
            let creds = self.sessions[sess_idx].creds.clone();
            match crate::vfs::remote::connect(&creds).await {
                Ok(conn) => {
                    let new_id = self.next_session_id;
                    self.next_session_id += 1;
                    let new_scheme = format!("{}-{}", creds.protocol.scheme_prefix(), new_id);
                    self.registry.register(new_scheme.clone(), conn.backend.clone());
                    // Safe: the transfer holds its own Arc, not the registry entry.
                    self.registry.unregister(&scheme);
                    let path = self.panels[side].cwd.path.clone();
                    let new_cwd =
                        VfsPath { scheme: new_scheme.clone(), path, container: None };
                    self.panels[side].cwd = new_cwd.clone();
                    self.panels[side].backend = conn.backend;
                    let _ = self.panels[side].reload().await;
                    // Repoint the session record in place (same id/label, new scheme).
                    self.sessions[sess_idx].scheme = new_scheme;
                    self.sessions[sess_idx].cwd = new_cwd;
                }
                Err(e) => {
                    self.show_error(format!("Could not open a browsing connection: {e}"))
                }
            }
        }
    }

    /// Switch panel `side` to an already-open remote session, landing on the
    /// directory it was last viewing there.
    pub(in crate::app::state) async fn switch_to_session(&mut self, side: usize, id: usize) {
        let Some(s) = self.sessions.iter().find(|s| s.id == id) else {
            return;
        };
        let target = s.cwd.clone();
        // Already on this session? nothing to do.
        if self.panels[side].cwd.scheme == target.scheme {
            return;
        }
        if self.other_panel_is_remote(side) {
            self.show_error(
                "The other panel is already remote — return it to Local first.".to_string(),
            );
            return;
        }
        let backend = match self.registry.resolve(&target) {
            Ok(b) => b,
            Err(e) => {
                self.show_error(e.to_string());
                return;
            }
        };
        self.snapshot_session_cwd(side);
        let ok = self.panels[side].try_enter(target.clone(), backend, None).await;
        if !ok {
            // The session is still registered; just report the failed listing.
            self.show_error(format!("Cannot open {}", target.display()));
        }
    }

    /// Return panel `side` to its last local directory *without* closing any
    /// remote session (drive-letter style). Falls back to the process cwd if
    /// the remembered directory is gone.
    pub(in crate::app::state) async fn go_local(&mut self, side: usize) {
        if !self.panels[side].cwd.is_remote() {
            return; // already local (or an archive, which counts as local)
        }
        self.snapshot_session_cwd(side);
        let target = self.last_local_cwd[side].clone();
        let backend = self.registry.local();
        let ok = self.panels[side].try_enter(target, backend.clone(), None).await;
        if !ok {
            let _ = self.panels[side]
                .try_enter(VfsPath::local_cwd(), backend, None)
                .await;
        }
    }

    /// Close a remote session for good: if a panel is on it, send that panel
    /// back to local first, then unregister the backend and drop the record.
    pub(in crate::app::state) async fn disconnect_session(&mut self, id: usize) {
        let Some(pos) = self.sessions.iter().position(|s| s.id == id) else {
            return;
        };
        let scheme = self.sessions[pos].scheme.clone();
        for side in 0..2 {
            if self.panels[side].cwd.scheme == scheme {
                self.go_local(side).await;
            }
        }
        self.registry.unregister(&scheme);
        self.sessions.retain(|s| s.id != id);
    }

    /// Open the drive/connection picker for `side` (Alt-F1 left, Alt-F2 right):
    /// a Local button, drive letters (Windows), and — unless the other panel is
    /// already remote — the open sessions and SFTP/FTP/SCP connect buttons.
    pub(in crate::app::state) fn open_drive_dialog(&mut self, side: usize) {
        let drives = crate::drive::available_drives();
        let current_drive = crate::drive::drive_of(&self.panels[side].cwd.path);
        let cur_scheme = self.panels[side].cwd.scheme.clone();
        let current_session = self
            .sessions
            .iter()
            .find(|s| s.scheme == cur_scheme)
            .map(|s| s.id);
        // Hide the session + connect buttons when the other panel is remote, so
        // a second remote panel can't be opened (one-remote rule).
        let show_remote = !self.other_panel_is_remote(side);
        let sessions = self.session_list();
        self.dialog = Some(Dialog::Drive(DriveDialog::new(
            side,
            drives,
            current_drive,
            current_session,
            sessions,
            show_remote,
        )));
    }

    /// Switch panel `side` to the root of a drive letter.
    pub(in crate::app::state) async fn set_drive(&mut self, side: usize, letter: char) {
        // A drive letter is local, so leaving a remote panel this way must
        // remember where we were on that session.
        self.snapshot_session_cwd(side);
        let root = VfsPath::local(crate::drive::drive_root(letter));
        let backend = self.registry.local();
        let ok = self.panels[side].try_enter(root, backend, None).await;
        if !ok {
            self.show_error(format!("Drive {letter}: is not ready"));
        }
    }
}
