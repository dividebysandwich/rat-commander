//! "Send file over LAN" (File → Send over LAN): share the active panel's
//! selection with a nearby device over an ephemeral HTTP server whose download
//! URL is shown as a QR code. A single local file is served in place; a
//! multi-file / directory selection is zipped to a temporary archive first (with
//! a progress dialog) and that zip is served instead.

use super::*;
use std::path::PathBuf;

impl AppState {
    /// Start the Send-file flow for the active panel's selection (menu / palette).
    pub(in crate::app::state) fn send_file(&mut self) {
        let panel = &self.panels[self.active];
        // Only local files can be served: a remote/archive panel has no on-disk
        // path to hand to the HTTP server.
        if panel.cwd.scheme != "file" || panel.cwd.container.is_some() {
            return self.show_error("Send over LAN works on local files only");
        }
        let sources = panel.operation_targets();
        if sources.is_empty() {
            return;
        }
        if sources.iter().any(|s| s.scheme != "file" || s.container.is_some()) {
            return self.show_error("Send over LAN works on local files only");
        }

        // A lone regular file is served directly; anything else (several items,
        // or a directory) is zipped first.
        if sources.len() == 1 && !sources[0].path.is_dir() {
            let path = sources[0].path.clone();
            let name = base_name(&path, "file");
            self.start_send_server(path, name, None);
            return;
        }

        // Name the zip after the single directory, else the panel's directory.
        let stem = if sources.len() == 1 {
            base_name(&sources[0].path, "files")
        } else {
            base_name(&panel.cwd.path, "files")
        };
        let download_name = format!("{stem}.zip");
        let id = self.next_task_id;
        self.next_task_id += 1;
        let dest = crate::util::temp::rc_temp_path("send").with_extension("zip");
        let local: Vec<PathBuf> = sources.iter().map(|s| s.path.clone()).collect();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                archive::create_archive(ArchiveFormat::Zip, &dest, &local).map(|()| dest)
            })
            .await;
            let out = match result {
                Ok(Ok(p)) => Ok(p),
                Ok(Err(e)) => Err(e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send(AppEvent::SendPrepared { name: download_name, result: out }).await;
        });
        self.dialog = Some(Dialog::Progress(ProgressDialog::new(id, "Compressing")));
    }

    /// A zip prepared for sending finished: on success serve it, on failure report.
    pub(in crate::app::state) fn on_send_prepared(
        &mut self,
        name: String,
        result: Result<PathBuf, String>,
    ) {
        match result {
            // Serve the temp zip and hand ownership of the temp path to the server
            // so it is removed when the dialog closes.
            Ok(path) => self.start_send_server(path.clone(), name, Some(path)),
            Err(e) => {
                if matches!(self.dialog, Some(Dialog::Progress(_))) {
                    self.dialog = None;
                }
                self.show_error(format!("Compress failed: {e}"));
            }
        }
    }

    /// A device fully downloaded the shared file: bump the dialog's counter.
    pub(in crate::app::state) fn on_file_sent(&mut self) {
        if let Some(Dialog::SendFile(d)) = &mut self.dialog {
            d.record_download();
        }
    }

    /// Launch the LAN HTTP server for `path` (advertised as `download_name`) and
    /// open the QR dialog. `temp` is a temporary archive to delete when the dialog
    /// closes; pass `None` for a file served in place.
    fn start_send_server(&mut self, path: PathBuf, download_name: String, temp: Option<PathBuf>) {
        // Replace any previous server (defensive — the dialog is modal).
        self.stop_send_server();
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let ip = crate::send::lan_ip();
        let (port, server) =
            match crate::send::start(path, download_name.clone(), temp.clone(), self.tx.clone()) {
                Ok(v) => v,
                Err(e) => {
                    if let Some(t) = temp {
                        let _ = std::fs::remove_file(t);
                    }
                    return self.show_error(format!("Cannot start send server: {e}"));
                }
            };
        let url = crate::send::url_for(ip, port, &download_name);
        match SendFileDialog::new(url.clone(), download_name, size) {
            Some(d) => {
                self.send_server = Some(server);
                self.dialog = Some(Dialog::SendFile(d));
            }
            None => {
                // The URL is somehow too long for any QR version: drop the server
                // and just show the address to type in manually.
                server.shutdown();
                self.show_info("Send file over LAN", format!("Open this URL:\n{url}"));
            }
        }
    }

    /// Shut down the running send server (if any): abort the accept loop and
    /// delete its temporary archive. Called when the Send dialog closes and on
    /// program exit.
    pub(crate) fn stop_send_server(&mut self) {
        if let Some(s) = self.send_server.take() {
            s.shutdown();
        }
    }
}

/// The final path component as a string, or `fallback` when there is none.
fn base_name(path: &std::path::Path, fallback: &str) -> String {
    path.file_name().and_then(|s| s.to_str()).unwrap_or(fallback).to_string()
}
