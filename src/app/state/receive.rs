//! "Receive files over LAN" (File → Receive over LAN): take files from a nearby
//! device into the active panel's directory. An ephemeral HTTP server serves an
//! upload page behind a random token, whose URL is shown as a QR code; see
//! [`crate::receive`].

use super::*;

impl AppState {
    /// Start receiving into the active panel's directory (menu / palette).
    pub(in crate::app::state) fn receive_files(&mut self) {
        let panel = &self.panels[self.active];
        if !panel.cwd.is_plain_local() {
            return self.show_error("Receive over LAN saves into local directories only");
        }
        let dir = panel.cwd.path.clone();
        self.stop_receive_server();
        let token = crate::util::rng::token(16);
        let (port, server) =
            match crate::receive::start(dir.clone(), token.clone(), self.tx.clone()) {
                Ok(v) => v,
                Err(e) => return self.show_error(format!("Cannot start receive server: {e}")),
            };
        let url = crate::receive::url_for(crate::send::lan_ip(), port, &token);
        match ReceiveDialog::new(url.clone(), dir.display().to_string()) {
            Some(d) => {
                self.receive_server = Some(server);
                self.dialog = Some(Dialog::Receive(d));
            }
            None => {
                // Too long for any QR version: offer the address to type in.
                server.shutdown();
                self.show_info("Receive files over LAN", format!("Open this URL:\n{url}"));
            }
        }
    }

    /// A file arrived: count it, and show it in any panel on that directory.
    pub(in crate::app::state) async fn on_file_received(&mut self, name: String, bytes: u64) {
        if let Some(Dialog::Receive(d)) = &mut self.dialog {
            d.on_received(name, bytes);
        }
        // With auto-refresh on, the directory watch picks the file up by itself.
        if self.config.auto_refresh {
            return;
        }
        let Some(dir) = self.receive_server.as_ref().map(|s| s.dir.clone()) else { return };
        for panel in &mut self.panels {
            if panel.cwd.is_plain_local() && panel.cwd.path == dir {
                let _ = panel.reload().await;
            }
        }
    }

    /// Stop the receive server, if one is running. Called when its dialog
    /// closes and on program exit.
    pub(crate) fn stop_receive_server(&mut self) {
        if let Some(s) = self.receive_server.take() {
            s.shutdown();
        }
    }
}
