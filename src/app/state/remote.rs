//! Remote connections and Windows drive selection.

use super::*;

impl AppState {
    // -- Remote connections ------------------------------------------------

    pub(in crate::app::state) async fn connect_remote(&mut self, side: usize, creds: RemoteCreds) {
        match crate::vfs::remote::connect(&creds).await {
            Ok(conn) => {
                let scheme = format!("{}-{}", creds.protocol.scheme_prefix(), self.next_session_id);
                self.next_session_id += 1;
                self.registry.register(scheme.clone(), conn.backend.clone());
                let cwd = VfsPath {
                    scheme,
                    path: PathBuf::from(&conn.root),
                    container: None,
                };
                let p = &mut self.panels[side];
                p.cwd = cwd;
                p.backend = conn.backend;
                p.selection.clear();
                let _ = p.reload().await;

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

    pub(in crate::app::state) async fn disconnect(&mut self, side: usize) {
        if self.panels[side].cwd.scheme == "file" {
            return;
        }
        let local = self.registry.local();
        let p = &mut self.panels[side];
        p.cwd = VfsPath::local_cwd();
        p.backend = local;
        p.selection.clear();
        let _ = p.reload().await;
    }

    /// Open the drive/connection picker for `side` (Alt-F1 left, Alt-F2 right):
    /// drive letters (Windows) plus SFTP/FTP/SCP, and Disconnect when on a remote.
    pub(in crate::app::state) fn open_drive_dialog(&mut self, side: usize) {
        let drives = crate::drive::available_drives();
        let current = crate::drive::drive_of(&self.panels[side].cwd.path);
        // A remote panel uses a `sftp-…/ftp-…/scp-…` scheme (not file/archive).
        let connected = !matches!(self.panels[side].cwd.scheme.as_str(), "file" | "archive");
        self.dialog = Some(Dialog::Drive(DriveDialog::new(side, drives, current, connected)));
    }

    /// Switch panel `side` to the root of a drive letter.
    pub(in crate::app::state) async fn set_drive(&mut self, side: usize, letter: char) {
        let root = VfsPath::local(crate::drive::drive_root(letter));
        let backend = self.registry.local();
        let ok = self.panels[side].try_enter(root, backend, None).await;
        if !ok {
            self.show_error(format!("Drive {letter}: is not ready"));
        }
    }

}
