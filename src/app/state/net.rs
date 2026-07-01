//! Network-connections explorer: prompting for an optional root password,
//! opening the view, and running its (async, `ss`-based) scans.

use super::*;
use crate::net::{NetView, Scan};

impl AppState {
    /// Command-menu entry point: prompt for a root password before opening the
    /// network explorer. A blank password (or Esc) opens it in user mode with
    /// limited visibility; a password opens it with full (sudo) visibility.
    pub(in crate::app::state) fn open_network_prompt(&mut self) {
        self.dialog = Some(Dialog::Input(InputDialog::password(
            "Network connections",
            "Root password (blank = user mode, limited visibility):",
            InputPurpose::NetworkPassword,
        )));
    }

    /// Open the explorer with the entered password (blank ⇒ user mode) and kick
    /// off the first scan.
    pub(in crate::app::state) fn open_network(&mut self, password: String) {
        let password = (!password.is_empty()).then_some(password);
        self.netview = Some(NetView::new(password.is_some(), password));
        self.start_network_scan();
    }

    /// Spawn a background `ss` scan; its result arrives via
    /// [`AppEvent::NetworkScanned`].
    pub(in crate::app::state) fn start_network_scan(&mut self) {
        let Some(nv) = self.netview.as_mut() else {
            return;
        };
        nv.generation = nv.generation.wrapping_add(1);
        let generation = nv.generation;
        let password = nv.password.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = crate::net::scan(password).await;
            let _ = tx.send(AppEvent::NetworkScanned { generation, result }).await;
        });
    }

    /// Apply a completed scan (ignoring results from a superseded generation).
    pub(in crate::app::state) fn apply_network_scanned(
        &mut self,
        generation: u64,
        result: Result<Scan, String>,
    ) {
        let Some(nv) = self.netview.as_mut() else {
            return;
        };
        if nv.generation != generation {
            return; // a newer scan was started; drop this stale one
        }
        match result {
            Ok(scan) => nv.apply(scan),
            Err(e) => nv.fail(e),
        }
    }
}
