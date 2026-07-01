//! Network-connections explorer (Linux): a full-screen view listing all open
//! listening ports (with their programs) and all active connections (with their
//! type and, where the kernel reports it, the traffic each has carried).
//!
//! Data comes from `ss` (iproute2). Run unprivileged it lists every socket but
//! can only attribute a program to the current user's own sockets; run through
//! `sudo` (when the user supplies a root password) it attributes every socket.
//! Parsing is split out and unit-tested; only [`scan`] touches the process.

pub mod render;

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use std::sync::LazyLock;

/// The `users:(("name",pid=N,...))` process field emitted by `ss -p`.
static USERS_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r#""([^"]+)",pid=(\d+)"#).unwrap());

/// One socket row (a listening port or an active connection).
#[derive(Debug, Clone, Default)]
pub struct Socket {
    /// Protocol/type: `tcp`, `tcp6`, `udp`, `udp6`.
    pub proto: String,
    /// Socket state (`LISTEN`, `ESTAB`, `TIME-WAIT`, `UNCONN`, …).
    pub state: String,
    pub local: String,
    pub peer: String,
    /// Owning program name (empty when not visible — another user's socket
    /// without root).
    pub program: String,
    pub pid: Option<u32>,
    /// Cumulative bytes received / sent on this connection, when the kernel
    /// reports them (TCP with `ss -i`); `None` otherwise.
    pub rx: Option<u64>,
    pub tx: Option<u64>,
}

impl Socket {
    fn traffic(&self) -> u64 {
        self.rx.unwrap_or(0) + self.tx.unwrap_or(0)
    }
}

/// The result of a scan: listening ports and active connections.
#[derive(Debug, Clone, Default)]
pub struct Scan {
    pub listening: Vec<Socket>,
    pub connections: Vec<Socket>,
}

/// Which of the two lists has the keyboard focus (cursor + scrolling).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Listening,
    Connections,
}

/// What handling a key asks the app to do.
pub enum NetSignal {
    Stay,
    Close,
    /// Re-run the scan (the view keeps its current password/root mode).
    Refresh,
}

pub struct NetView {
    pub listening: Vec<Socket>,
    pub connections: Vec<Socket>,
    pub focus: Pane,
    /// Cursor + scroll offset per pane (index 0 = listening, 1 = connections).
    pub cursor: [usize; 2],
    pub offset: [usize; 2],
    /// Visible data rows per pane, set by the renderer for paging math.
    pub view_rows: [usize; 2],
    /// Whether the scan runs with root privileges (a password was supplied).
    pub root: bool,
    /// The root password, kept in memory so periodic refreshes can re-run `sudo`
    /// without re-prompting. `None` in user mode.
    pub(crate) password: Option<String>,
    /// True until the first scan returns (shows a "scanning…" placeholder).
    pub scanning: bool,
    /// The last scan's error, if it failed.
    pub error: Option<String>,
    /// Bumped each scan so stale background results can be dropped.
    pub generation: u64,
    /// Auto-refresh interval (ms), adjustable with `+`/`-`.
    pub interval_ms: u64,
    tick_accum: u64,
}

impl NetView {
    pub fn new(root: bool, password: Option<String>) -> Self {
        NetView {
            listening: Vec::new(),
            connections: Vec::new(),
            focus: Pane::Listening,
            cursor: [0, 0],
            offset: [0, 0],
            view_rows: [1, 1],
            root,
            password,
            scanning: true,
            error: None,
            generation: 0,
            interval_ms: 2000,
            tick_accum: 0,
        }
    }

    fn pane_idx(&self) -> usize {
        match self.focus {
            Pane::Listening => 0,
            Pane::Connections => 1,
        }
    }

    fn len(&self, pane: usize) -> usize {
        if pane == 0 { self.listening.len() } else { self.connections.len() }
    }

    /// Apply a completed scan (dropping stale generations is done by the caller).
    pub fn apply(&mut self, scan: Scan) {
        self.listening = scan.listening;
        self.connections = scan.connections;
        self.scanning = false;
        self.error = None;
        self.clamp_cursors();
    }

    /// Record a failed scan.
    pub fn fail(&mut self, err: String) {
        self.scanning = false;
        self.error = Some(err);
    }

    fn clamp_cursors(&mut self) {
        for p in 0..2 {
            let n = self.len(p);
            if self.cursor[p] >= n {
                self.cursor[p] = n.saturating_sub(1);
            }
        }
    }

    /// True once `interval_ms` has elapsed (in 100 ms ticks) — time to refresh.
    pub fn tick_due(&mut self) -> bool {
        self.tick_accum += 100;
        if self.tick_accum >= self.interval_ms {
            self.tick_accum = 0;
            true
        } else {
            false
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> NetSignal {
        match key.code {
            KeyCode::Esc | KeyCode::F(10) | KeyCode::Char('q') | KeyCode::Char('Q') => {
                NetSignal::Close
            }
            KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                self.focus = match self.focus {
                    Pane::Listening => Pane::Connections,
                    Pane::Connections => Pane::Listening,
                };
                NetSignal::Stay
            }
            KeyCode::Char('r') | KeyCode::Char('R') | KeyCode::F(5) => NetSignal::Refresh,
            KeyCode::Up => {
                let p = self.pane_idx();
                self.cursor[p] = self.cursor[p].saturating_sub(1);
                NetSignal::Stay
            }
            KeyCode::Down => {
                let p = self.pane_idx();
                if self.cursor[p] + 1 < self.len(p) {
                    self.cursor[p] += 1;
                }
                NetSignal::Stay
            }
            KeyCode::PageUp => {
                let p = self.pane_idx();
                self.cursor[p] = self.cursor[p].saturating_sub(self.view_rows[p].max(1));
                NetSignal::Stay
            }
            KeyCode::PageDown => {
                let p = self.pane_idx();
                let step = self.view_rows[p].max(1);
                self.cursor[p] = (self.cursor[p] + step).min(self.len(p).saturating_sub(1));
                NetSignal::Stay
            }
            KeyCode::Home => {
                self.cursor[self.pane_idx()] = 0;
                NetSignal::Stay
            }
            KeyCode::End => {
                let p = self.pane_idx();
                self.cursor[p] = self.len(p).saturating_sub(1);
                NetSignal::Stay
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.interval_ms = (self.interval_ms + 500).min(60_000);
                NetSignal::Stay
            }
            KeyCode::Char('-') | KeyCode::Char('_') => {
                self.interval_ms = self.interval_ms.saturating_sub(500).max(500);
                NetSignal::Stay
            }
            _ => NetSignal::Stay,
        }
    }
}

// ---------------------------------------------------------------------------
// Data collection
// ---------------------------------------------------------------------------

/// `ss` flags: TCP+UDP, numeric, all states, one line per socket, processes,
/// and per-socket info (for the byte counters).
const SS_ARGS: [&str; 7] = ["-t", "-u", "-n", "-a", "-p", "-i", "-O"];

/// Run `ss` (via `sudo` when `password` is `Some`) and parse its output. Errors
/// carry a short message for the view to display.
pub async fn scan(password: Option<String>) -> Result<Scan, String> {
    let out = run_ss(password).await?;
    Ok(parse_ss(&out))
}

async fn run_ss(password: Option<String>) -> Result<String, String> {
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt;
    use tokio::process::Command;

    if let Some(pw) = password {
        // `sudo -S ss …`, feeding the password on stdin so it never appears on a
        // command line or a tty prompt.
        let mut child = Command::new("sudo")
            .arg("-S")
            .arg("ss")
            .args(SS_ARGS)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not run sudo: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(format!("{pw}\n").as_bytes()).await;
            let _ = stdin.shutdown().await;
        }
        let out = child.wait_with_output().await.map_err(|e| e.to_string())?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let msg = stderr
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty() && !l.starts_with("[sudo]"))
                .unwrap_or("")
                .to_string();
            return Err(if msg.is_empty() {
                "sudo failed (wrong password?)".to_string()
            } else {
                msg
            });
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let out = Command::new("ss")
            .args(SS_ARGS)
            .output()
            .await
            .map_err(|e| format!("could not run `ss`: {e} — is iproute2 installed?"))?;
        if !out.status.success() {
            let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(if msg.is_empty() { "`ss` failed".to_string() } else { msg });
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

/// Parse `ss -tunapiO` output into listening ports and active connections.
pub fn parse_ss(out: &str) -> Scan {
    let mut listening = Vec::new();
    let mut connections = Vec::new();

    for line in out.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        if t.len() < 6 {
            continue;
        }
        // Skip the header row (`Netid State Recv-Q …`).
        if t[0].eq_ignore_ascii_case("netid") || t[0].eq_ignore_ascii_case("state") {
            continue;
        }
        let (local, peer) = (t[4], t[5]);
        let rest = &t[6..];
        let (program, pid) = parse_program(rest);
        let (rx, tx) = parse_bytes(rest);
        let sock = Socket {
            proto: proto_label(t[0], local),
            state: t[1].to_string(),
            local: local.to_string(),
            peer: peer.to_string(),
            program,
            pid,
            rx,
            tx,
        };
        // LISTEN (TCP) and UNCONN (bound UDP) are open ports; the rest are
        // connections with a concrete or in-progress peer.
        if sock.state == "LISTEN" || sock.state == "UNCONN" {
            listening.push(sock);
        } else {
            connections.push(sock);
        }
    }

    // Listening ports by port number; connections busiest-first (then peer).
    listening.sort_by(|a, b| port_of(&a.local).cmp(&port_of(&b.local)).then(a.proto.cmp(&b.proto)));
    connections.sort_by(|a, b| b.traffic().cmp(&a.traffic()).then(a.peer.cmp(&b.peer)));
    Scan { listening, connections }
}

/// `tcp`/`udp` plus a `6` suffix when the local address is IPv6.
fn proto_label(netid: &str, local: &str) -> String {
    // Some `ss` builds already label the netid `tcp6`/`udp6`; leave those as-is.
    if netid.ends_with('6') {
        return netid.to_string();
    }
    // The address part is everything before the final `:port`; IPv6 addresses
    // contain a colon there (e.g. `[::]`, `::1`, `fe80::1%eth0`).
    let addr = local.rsplit_once(':').map(|(a, _)| a).unwrap_or(local);
    if addr.contains(':') {
        format!("{netid}6")
    } else {
        netid.to_string()
    }
}

/// The port number from an `addr:port` (or `*:*`) token; 0 when it's a wildcard.
fn port_of(addr: &str) -> u32 {
    addr.rsplit(':').next().and_then(|p| p.parse().ok()).unwrap_or(0)
}

/// Extract the first `("name",pid=N)` from an `ss` `users:(…)` token.
fn parse_program(rest: &[&str]) -> (String, Option<u32>) {
    let Some(tok) = rest.iter().find(|t| t.starts_with("users:")) else {
        return (String::new(), None);
    };
    match USERS_RE.captures(tok) {
        Some(c) => (
            c.get(1).map(|m| m.as_str().to_string()).unwrap_or_default(),
            c.get(2).and_then(|m| m.as_str().parse().ok()),
        ),
        None => (String::new(), None),
    }
}

/// Extract `(bytes_received, bytes_sent)` from the `ss -i` info tokens. Falls back
/// to `bytes_acked` for the sent count on kernels without `bytes_sent`.
fn parse_bytes(rest: &[&str]) -> (Option<u64>, Option<u64>) {
    let (mut rx, mut tx, mut acked) = (None, None, None);
    for t in rest {
        if let Some(v) = t.strip_prefix("bytes_received:") {
            rx = v.parse().ok();
        } else if let Some(v) = t.strip_prefix("bytes_sent:") {
            tx = v.parse().ok();
        } else if let Some(v) = t.strip_prefix("bytes_acked:") {
            acked = v.parse().ok();
        }
    }
    (rx, tx.or(acked))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
Netid State  Recv-Q Send-Q Local Address:Port Peer Address:Port Process
tcp   LISTEN 0      128    0.0.0.0:22         0.0.0.0:*         users:((\"sshd\",pid=800,fd=3))
tcp   LISTEN 0      128    [::]:22            [::]:*           users:((\"sshd\",pid=800,fd=4))
udp   UNCONN 0      0      0.0.0.0:68         0.0.0.0:*         users:((\"dhclient\",pid=500,fd=6))
tcp   ESTAB  0      0      10.0.0.1:22        10.0.0.2:5555    users:((\"sshd\",pid=1234,fd=5)) cubic rto:204 bytes_sent:14215788 bytes_retrans:2831 bytes_acked:14212958 bytes_received:370528 segs_out:1
tcp   ESTAB  0      0      10.0.0.1:443       10.0.0.9:4444    cubic rto:210 bytes_acked:271552 bytes_received:699850
tcp   TIME-WAIT 0   0      10.0.0.1:80        10.0.0.3:6666";

    #[test]
    fn splits_into_listening_and_connections() {
        let s = parse_ss(SAMPLE);
        // Three listening (two tcp/tcp6 :22, one udp :68); three connections.
        assert_eq!(s.listening.len(), 3, "LISTEN + UNCONN → listening");
        assert_eq!(s.connections.len(), 3, "ESTAB + TIME-WAIT → connections");
        // IPv6 listener labeled tcp6.
        assert!(s.listening.iter().any(|x| x.proto == "tcp6" && x.program == "sshd"));
        assert!(s.listening.iter().any(|x| x.proto == "udp" && x.program == "dhclient"));
    }

    #[test]
    fn parses_program_and_traffic() {
        let s = parse_ss(SAMPLE);
        // The busiest connection sorts first and carries the parsed byte counts.
        let top = &s.connections[0];
        assert_eq!(top.program, "sshd");
        assert_eq!(top.pid, Some(1234));
        assert_eq!(top.rx, Some(370528));
        assert_eq!(top.tx, Some(14215788), "bytes_sent used for the sent count");
        // The second connection has no bytes_sent, so bytes_acked is the fallback.
        let acked = s.connections.iter().find(|c| c.peer == "10.0.0.9:4444").unwrap();
        assert_eq!(acked.tx, Some(271552));
        assert_eq!(acked.rx, Some(699850));
        // A socket with no `users:(…)` has no program (limited visibility).
        assert!(acked.program.is_empty());
        // A connection with no info at all has no byte counts.
        let tw = s.connections.iter().find(|c| c.state == "TIME-WAIT").unwrap();
        assert_eq!(tw.rx, None);
        assert_eq!(tw.tx, None);
    }

    #[test]
    fn header_and_blank_lines_are_ignored() {
        assert_eq!(parse_ss("Netid State Recv-Q Send-Q Local Peer Proc\n\n").listening.len(), 0);
        assert_eq!(parse_ss("").connections.len(), 0);
    }

    #[test]
    fn renders_both_panes_without_panic() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut nv = NetView::new(false, None);
        nv.apply(parse_ss(SAMPLE));
        let theme = crate::ui::theme::Theme::mc();
        let mut t = Terminal::new(TestBackend::new(110, 30)).unwrap();
        t.draw(|f| render::render(f, f.area(), &mut nv, &theme)).unwrap();
        let b = t.backend().buffer();
        let mut s = String::new();
        for y in 0..b.area.height {
            for x in 0..b.area.width {
                s.push_str(b[(x, y)].symbol());
            }
        }
        assert!(s.contains("Network Connections"), "title");
        assert!(s.contains("user mode"), "limited-visibility banner in user mode");
        assert!(s.contains("Listening ports (3)"), "listening pane with count");
        assert!(s.contains("Connections (3)"), "connections pane with count");
        assert!(s.contains("sshd"), "a program name is shown");
        assert!(s.contains("Program") && s.contains("Peer"), "column headers");
    }
}
