//! Network-connections explorer (Linux): a full-screen view listing all open
//! listening ports (with their programs) and all active connections (with their
//! type and, where the kernel reports it, the traffic each has carried).
//!
//! Data comes from `ss` (iproute2). Run unprivileged it lists every socket but
//! can only attribute a program to the current user's own sockets; run through
//! `sudo` (when the user supplies a root password) it attributes every socket.
//!
//! On top of the raw lists the view offers everyday-triage tools: a live
//! substring **filter**, per-pane **sorting**, quick **toggles** (protocol,
//! established-only, hide-loopback), per-connection **traffic rates** (with a
//! sparkline), **service names** for ports, a **details** popup, and **killing**
//! a socket's owning process. Parsing and the pure helpers are unit-tested; only
//! [`scan`] touches the process.

pub mod render;

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::sync::LazyLock;
use std::time::Instant;

/// The `users:(("name",pid=N,...))` process field emitted by `ss -p`.
static USERS_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r#""([^"]+)",pid=(\d+)"#).unwrap());

/// How many rate samples to keep per connection (and overall) for the sparkline.
const RATE_HISTORY: usize = 90;

/// A stable identity for a socket across refreshes (for rate deltas + details).
fn socket_key(s: &Socket) -> String {
    format!("{}|{}|{}", s.proto, s.local, s.peer)
}

/// One socket row (a listening port or an active connection).
#[derive(Debug, Clone, Default)]
pub struct Socket {
    /// Protocol/type: `tcp`, `tcp6`, `udp`, `udp6`.
    pub proto: String,
    /// Socket state (`LISTEN`, `ESTAB`, `TIME-WAIT`, `UNCONN`, …).
    pub state: String,
    pub local: String,
    pub peer: String,
    /// Service name for this socket's notable port (`https`, `ssh`, …), if known.
    pub service: String,
    /// Owning program name (empty when not visible — another user's socket
    /// without root).
    pub program: String,
    pub pid: Option<u32>,
    /// Cumulative bytes received / sent on this connection, when the kernel
    /// reports them (TCP with `ss -i`); `None` otherwise.
    pub rx: Option<u64>,
    pub tx: Option<u64>,
    /// Bytes/sec in / out, computed from the change since the previous scan.
    pub rx_rate: Option<u64>,
    pub tx_rate: Option<u64>,
    /// The raw `ss -i` info tail (rtt, cwnd, retransmits, …), for the details view.
    pub info: String,
}

impl Socket {
    fn traffic(&self) -> u64 {
        self.rx.unwrap_or(0) + self.tx.unwrap_or(0)
    }
    fn rate(&self) -> u64 {
        self.rx_rate.unwrap_or(0) + self.tx_rate.unwrap_or(0)
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

impl Pane {
    fn idx(self) -> usize {
        match self {
            Pane::Listening => 0,
            Pane::Connections => 1,
        }
    }
}

/// A sortable column. Which apply to a pane is decided by [`NetView::sort_keys`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetSort {
    Port,
    Program,
    Proto,
    State,
    Peer,
    Traffic,
    Rate,
}

impl NetSort {
    fn label(self) -> &'static str {
        match self {
            NetSort::Port => "port",
            NetSort::Program => "program",
            NetSort::Proto => "proto",
            NetSort::State => "state",
            NetSort::Peer => "peer",
            NetSort::Traffic => "traffic",
            NetSort::Rate => "rate",
        }
    }
    /// Numeric columns default to descending (busiest first).
    fn default_desc(self) -> bool {
        matches!(self, NetSort::Traffic | NetSort::Rate)
    }
}

/// The protocol quick-filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtoFilter {
    All,
    Tcp,
    Udp,
}

impl ProtoFilter {
    fn label(self) -> &'static str {
        match self {
            ProtoFilter::All => "all",
            ProtoFilter::Tcp => "tcp",
            ProtoFilter::Udp => "udp",
        }
    }
    fn next(self) -> ProtoFilter {
        match self {
            ProtoFilter::All => ProtoFilter::Tcp,
            ProtoFilter::Tcp => ProtoFilter::Udp,
            ProtoFilter::Udp => ProtoFilter::All,
        }
    }
}

/// Extra process info shown in the details popup (loaded once when it opens).
#[derive(Debug, Clone, Default)]
pub struct DetailInfo {
    pub cmdline: String,
    pub user: String,
}

/// The open details popup: a snapshot of the selected socket plus its loaded
/// process info. Live rate/history are looked up by `key` at render time.
pub struct DetailState {
    pub key: String,
    pub sock: Socket,
    pub info: DetailInfo,
}

/// What handling a key asks the app to do.
pub enum NetSignal {
    Stay,
    Close,
    /// Re-run the scan (the view keeps its current password/root mode).
    Refresh,
    /// Kill the owning process of the selected socket (`force` ⇒ SIGKILL). The
    /// app confirms first.
    Kill { pid: i32, program: String, force: bool },
}

pub struct NetView {
    pub listening: Vec<Socket>,
    pub connections: Vec<Socket>,
    /// Filtered + sorted row indices into the two lists (what the renderer shows).
    pub view: [Vec<usize>; 2],
    pub focus: Pane,
    /// Cursor + scroll offset per pane (indices into `view`).
    pub cursor: [usize; 2],
    pub offset: [usize; 2],
    /// Visible data rows per pane, set by the renderer for paging math.
    pub view_rows: [usize; 2],

    // --- filtering / sorting / toggles ---
    pub filter: String,
    pub filter_cursor: usize,
    pub filtering: bool,
    pub sort: [NetSort; 2],
    pub reverse: [bool; 2],
    pub proto_filter: ProtoFilter,
    pub established_only: bool,
    pub hide_loopback: bool,

    // --- details popup ---
    pub detail: Option<DetailState>,

    // --- rate tracking ---
    /// Previous cumulative `(rx, tx)` per connection key.
    prev: HashMap<String, (u64, u64)>,
    last_instant: Option<Instant>,
    /// Per-connection `(rx_rate, tx_rate)` history for the per-row + details
    /// sparklines.
    pub rate_history: HashMap<String, VecDeque<(u64, u64)>>,
    /// Overall in/out rate (bytes/sec), shown numerically in the header.
    pub rate_in: u64,
    pub rate_out: u64,

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
            view: [Vec::new(), Vec::new()],
            focus: Pane::Listening,
            cursor: [0, 0],
            offset: [0, 0],
            view_rows: [1, 1],
            filter: String::new(),
            filter_cursor: 0,
            filtering: false,
            // Listening: by port; connections: busiest (traffic) first.
            sort: [NetSort::Port, NetSort::Traffic],
            reverse: [false, true],
            proto_filter: ProtoFilter::All,
            established_only: false,
            hide_loopback: false,
            detail: None,
            prev: HashMap::new(),
            last_instant: None,
            rate_history: HashMap::new(),
            rate_in: 0,
            rate_out: 0,
            root,
            password,
            scanning: true,
            error: None,
            generation: 0,
            interval_ms: 2000,
            tick_accum: 0,
        }
    }

    fn list(&self, pane: usize) -> &[Socket] {
        if pane == 0 { &self.listening } else { &self.connections }
    }

    /// Number of *visible* rows in a pane (after filtering).
    fn len(&self, pane: usize) -> usize {
        self.view[pane].len()
    }

    /// The socket under the cursor of the focused pane.
    pub fn selected(&self) -> Option<&Socket> {
        let p = self.focus.idx();
        let &row = self.view[p].get(self.cursor[p])?;
        self.list(p).get(row)
    }

    /// The sort keys offered for a pane, cycled by `s`.
    fn sort_keys(pane: usize) -> &'static [NetSort] {
        const LISTEN: [NetSort; 3] = [NetSort::Port, NetSort::Program, NetSort::Proto];
        const CONN: [NetSort; 6] = [
            NetSort::Traffic,
            NetSort::Rate,
            NetSort::Program,
            NetSort::State,
            NetSort::Peer,
            NetSort::Proto,
        ];
        if pane == 0 { &LISTEN } else { &CONN }
    }

    /// Apply a completed scan: compute rates, then rebuild the visible views.
    pub fn apply(&mut self, scan: Scan) {
        let now = Instant::now();
        let dt = self.last_instant.map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(0.0);
        self.last_instant = Some(now);
        self.listening = scan.listening;
        self.connections = scan.connections;
        self.compute_rates(dt);
        self.scanning = false;
        self.error = None;
        self.rebuild_views();
    }

    /// Compute per-connection and overall byte rates from the change since the
    /// previous scan. `dt` is the elapsed seconds (0 ⇒ first scan, no rates yet).
    fn compute_rates(&mut self, dt: f64) {
        // Take the maps out so the loop can freely borrow `self.connections`.
        let prev = std::mem::take(&mut self.prev);
        let mut history = std::mem::take(&mut self.rate_history);
        let mut next_prev = HashMap::with_capacity(self.connections.len());
        let (mut sum_rx, mut sum_tx) = (0u64, 0u64);
        for s in &mut self.connections {
            let (cur_rx, cur_tx) = (s.rx.unwrap_or(0), s.tx.unwrap_or(0));
            let key = socket_key(s);
            if dt > 0.0
                && let Some(&(prx, ptx)) = prev.get(&key)
            {
                // Counters only grow; a smaller value means the socket was reused.
                let drx = cur_rx.saturating_sub(prx);
                let dtx = cur_tx.saturating_sub(ptx);
                let rx_rate = (drx as f64 / dt) as u64;
                let tx_rate = (dtx as f64 / dt) as u64;
                if s.rx.is_some() {
                    s.rx_rate = Some(rx_rate);
                }
                if s.tx.is_some() {
                    s.tx_rate = Some(tx_rate);
                }
                sum_rx += drx;
                sum_tx += dtx;
                let hist = history.entry(key.clone()).or_default();
                hist.push_back((rx_rate, tx_rate));
                while hist.len() > RATE_HISTORY {
                    hist.pop_front();
                }
            }
            next_prev.insert(key, (cur_rx, cur_tx));
        }
        // Drop history for connections that have gone away.
        history.retain(|k, _| next_prev.contains_key(k));
        self.rate_history = history;
        self.prev = next_prev;

        if dt > 0.0 {
            self.rate_in = (sum_rx as f64 / dt) as u64;
            self.rate_out = (sum_tx as f64 / dt) as u64;
        }
    }

    /// Record a failed scan.
    pub fn fail(&mut self, err: String) {
        self.scanning = false;
        self.error = Some(err);
    }

    /// Whether `s` passes the active filters (protocol, established-only,
    /// hide-loopback, and the text filter).
    fn passes(&self, s: &Socket, pane: usize) -> bool {
        match self.proto_filter {
            ProtoFilter::Tcp if !s.proto.starts_with("tcp") => return false,
            ProtoFilter::Udp if !s.proto.starts_with("udp") => return false,
            _ => {}
        }
        if pane == 1 && self.established_only && s.state != "ESTAB" {
            return false;
        }
        if self.hide_loopback && (is_loopback(&s.local) || (pane == 1 && is_loopback(&s.peer))) {
            return false;
        }
        if !self.filter.is_empty() {
            let needle = self.filter.to_lowercase();
            let hay = format!(
                "{} {} {} {} {} {}",
                s.proto, s.state, s.local, s.peer, s.program, s.service
            )
            .to_lowercase();
            if !hay.contains(&needle) {
                return false;
            }
        }
        true
    }

    /// Recompute the filtered + sorted index list for both panes.
    fn rebuild_views(&mut self) {
        for pane in 0..2 {
            let mut idx: Vec<usize> =
                (0..self.list(pane).len()).filter(|&i| self.passes(&self.list(pane)[i], pane)).collect();
            let (key, rev) = (self.sort[pane], self.reverse[pane]);
            let list = self.list(pane);
            idx.sort_by(|&a, &b| {
                let o = sort_cmp(&list[a], &list[b], key);
                if rev { o.reverse() } else { o }
            });
            self.view[pane] = idx;
        }
        self.clamp_cursors();
    }

    fn clamp_cursors(&mut self) {
        for p in 0..2 {
            let n = self.len(p);
            if self.cursor[p] >= n {
                self.cursor[p] = n.saturating_sub(1);
            }
        }
    }

    /// Cycle the focused pane's sort key (or reverse it).
    fn cycle_sort(&mut self, reverse_only: bool) {
        let p = self.focus.idx();
        if reverse_only {
            self.reverse[p] = !self.reverse[p];
        } else {
            let keys = Self::sort_keys(p);
            let cur = keys.iter().position(|&k| k == self.sort[p]).unwrap_or(0);
            let next = keys[(cur + 1) % keys.len()];
            self.sort[p] = next;
            self.reverse[p] = next.default_desc();
        }
        self.rebuild_views();
    }

    /// The active sort description for a pane, e.g. `traffic↓`.
    pub fn sort_desc(&self, pane: usize) -> String {
        let arrow = if self.reverse[pane] { "↓" } else { "↑" };
        format!("{}{arrow}", self.sort[pane].label())
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

    /// Minimal in-place edit of the filter buffer (insert/backspace/caret).
    fn edit_filter(&mut self, key: KeyEvent) {
        let byte_at = |s: &str, idx: usize| s.char_indices().nth(idx).map(|(b, _)| b).unwrap_or(s.len());
        match key.code {
            KeyCode::Char(c) => {
                let b = byte_at(&self.filter, self.filter_cursor);
                self.filter.insert(b, c);
                self.filter_cursor += 1;
            }
            KeyCode::Backspace if self.filter_cursor > 0 => {
                let b = byte_at(&self.filter, self.filter_cursor - 1);
                self.filter.remove(b);
                self.filter_cursor -= 1;
            }
            KeyCode::Delete if self.filter_cursor < self.filter.chars().count() => {
                let b = byte_at(&self.filter, self.filter_cursor);
                self.filter.remove(b);
            }
            KeyCode::Left => self.filter_cursor = self.filter_cursor.saturating_sub(1),
            KeyCode::Right if self.filter_cursor < self.filter.chars().count() => {
                self.filter_cursor += 1
            }
            KeyCode::Home => self.filter_cursor = 0,
            KeyCode::End => self.filter_cursor = self.filter.chars().count(),
            _ => {}
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> NetSignal {
        // The details popup captures keys first: anything dismisses it.
        if self.detail.is_some() {
            if matches!(key.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q')) {
                self.detail = None;
            }
            return NetSignal::Stay;
        }

        // Filter-entry mode: text edits the filter; navigation still works.
        if self.filtering {
            match key.code {
                KeyCode::Enter => self.filtering = false,
                KeyCode::Esc => {
                    self.filtering = false;
                    self.filter.clear();
                    self.filter_cursor = 0;
                    self.rebuild_views();
                }
                KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown | KeyCode::Tab => {
                    self.navigate(key.code)
                }
                _ => {
                    self.edit_filter(key);
                    self.rebuild_views();
                }
            }
            return NetSignal::Stay;
        }

        match key.code {
            KeyCode::Esc | KeyCode::F(10) | KeyCode::Char('q') | KeyCode::Char('Q') => {
                return NetSignal::Close;
            }
            KeyCode::Char('r') | KeyCode::Char('R') | KeyCode::F(5) => return NetSignal::Refresh,
            KeyCode::Char('/') => {
                self.filtering = true;
                self.filter_cursor = self.filter.chars().count();
            }
            KeyCode::Char('s') => self.cycle_sort(false),
            KeyCode::Char('S') => self.cycle_sort(true),
            KeyCode::Char('p') | KeyCode::Char('P') => {
                self.proto_filter = self.proto_filter.next();
                self.rebuild_views();
            }
            KeyCode::Char('e') | KeyCode::Char('E') => {
                self.established_only = !self.established_only;
                self.rebuild_views();
            }
            KeyCode::Char('h') | KeyCode::Char('H') => {
                self.hide_loopback = !self.hide_loopback;
                self.rebuild_views();
            }
            KeyCode::Char('k') => return self.kill_request(false),
            KeyCode::Char('K') => return self.kill_request(true),
            KeyCode::Enter => self.open_detail(),
            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.interval_ms = (self.interval_ms + 500).min(60_000);
            }
            KeyCode::Char('-') | KeyCode::Char('_') => {
                self.interval_ms = self.interval_ms.saturating_sub(500).max(500);
            }
            code => self.navigate(code),
        }
        NetSignal::Stay
    }

    /// Cursor / pane navigation shared between normal and filter modes.
    fn navigate(&mut self, code: KeyCode) {
        match code {
            KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                self.focus = match self.focus {
                    Pane::Listening => Pane::Connections,
                    Pane::Connections => Pane::Listening,
                };
            }
            KeyCode::Up => {
                let p = self.focus.idx();
                self.cursor[p] = self.cursor[p].saturating_sub(1);
            }
            KeyCode::Down => {
                let p = self.focus.idx();
                if self.cursor[p] + 1 < self.len(p) {
                    self.cursor[p] += 1;
                }
            }
            KeyCode::PageUp => {
                let p = self.focus.idx();
                self.cursor[p] = self.cursor[p].saturating_sub(self.view_rows[p].max(1));
            }
            KeyCode::PageDown => {
                let p = self.focus.idx();
                let step = self.view_rows[p].max(1);
                self.cursor[p] = (self.cursor[p] + step).min(self.len(p).saturating_sub(1));
            }
            KeyCode::Home => self.cursor[self.focus.idx()] = 0,
            KeyCode::End => {
                let p = self.focus.idx();
                self.cursor[p] = self.len(p).saturating_sub(1);
            }
            _ => {}
        }
    }

    fn kill_request(&self, force: bool) -> NetSignal {
        match self.selected() {
            Some(s) if s.pid.is_some() => NetSignal::Kill {
                pid: s.pid.unwrap() as i32,
                program: if s.program.is_empty() { "?".to_string() } else { s.program.clone() },
                force,
            },
            _ => NetSignal::Stay,
        }
    }

    fn open_detail(&mut self) {
        let Some(s) = self.selected().cloned() else {
            return;
        };
        let info = s.pid.map(load_detail_info).unwrap_or_default();
        self.detail = Some(DetailState { key: socket_key(&s), sock: s, info });
    }
}

/// Compare two sockets by `key` (ascending); the caller reverses if needed.
fn sort_cmp(a: &Socket, b: &Socket, key: NetSort) -> Ordering {
    match key {
        NetSort::Port => port_of(&a.local).cmp(&port_of(&b.local)),
        NetSort::Program => a.program.to_lowercase().cmp(&b.program.to_lowercase()),
        NetSort::Proto => a.proto.cmp(&b.proto),
        NetSort::State => a.state.cmp(&b.state),
        NetSort::Peer => a.peer.cmp(&b.peer),
        NetSort::Traffic => a.traffic().cmp(&b.traffic()),
        NetSort::Rate => a.rate().cmp(&b.rate()),
    }
    // Stable tiebreak so equal keys keep a deterministic order.
    .then_with(|| a.local.cmp(&b.local))
}

/// Whether an `addr:port` token is a loopback address.
fn is_loopback(addr: &str) -> bool {
    let host = addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(addr);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.starts_with("127.") || host == "::1" || host.starts_with("::1%") || host.contains("%lo")
}

/// Read a process's full command line + user (Linux `/proc`); empty elsewhere or
/// when unreadable. `cmdline` is world-readable, so this works in user mode too.
#[cfg(target_os = "linux")]
fn load_detail_info(pid: u32) -> DetailInfo {
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline"))
        .ok()
        .map(|b| {
            String::from_utf8_lossy(&b)
                .split('\0')
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    let user = std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| {
                l.strip_prefix("Uid:")
                    .and_then(|v| v.split_whitespace().next())
                    .and_then(|u| u.parse::<u32>().ok())
            })
        })
        .and_then(uid_name)
        .unwrap_or_default();
    DetailInfo { cmdline, user }
}

#[cfg(not(target_os = "linux"))]
fn load_detail_info(_pid: u32) -> DetailInfo {
    DetailInfo::default()
}

#[cfg(unix)]
fn uid_name(uid: u32) -> Option<String> {
    nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid)).ok().flatten().map(|u| u.name)
}

#[cfg(not(unix))]
fn uid_name(_uid: u32) -> Option<String> {
    None
}

// ---------------------------------------------------------------------------
// Service names (/etc/services)
// ---------------------------------------------------------------------------

/// Common port→service fallbacks used when `/etc/services` is unavailable.
const COMMON_SERVICES: &[(u16, &str, &str)] = &[
    (21, "tcp", "ftp"),
    (22, "tcp", "ssh"),
    (25, "tcp", "smtp"),
    (53, "tcp", "domain"),
    (53, "udp", "domain"),
    (67, "udp", "bootps"),
    (68, "udp", "bootpc"),
    (80, "tcp", "http"),
    (110, "tcp", "pop3"),
    (123, "udp", "ntp"),
    (143, "tcp", "imap"),
    (443, "tcp", "https"),
    (465, "tcp", "smtps"),
    (587, "tcp", "submission"),
    (631, "tcp", "ipp"),
    (993, "tcp", "imaps"),
    (995, "tcp", "pop3s"),
    (3306, "tcp", "mysql"),
    (5353, "udp", "mdns"),
    (5432, "tcp", "postgresql"),
    (6379, "tcp", "redis"),
    (8080, "tcp", "http-alt"),
];

static SERVICES: LazyLock<HashMap<(u16, String), String>> = LazyLock::new(|| {
    let mut m: HashMap<(u16, String), String> = HashMap::new();
    for &(port, proto, name) in COMMON_SERVICES {
        m.insert((port, proto.to_string()), name.to_string());
    }
    if let Ok(txt) = std::fs::read_to_string("/etc/services") {
        for line in txt.lines() {
            let line = line.split('#').next().unwrap_or("");
            let mut it = line.split_whitespace();
            let (Some(name), Some(port_proto)) = (it.next(), it.next()) else {
                continue;
            };
            let Some((port, proto)) = port_proto.split_once('/') else {
                continue;
            };
            if let Ok(port) = port.parse::<u16>() {
                m.entry((port, proto.to_string())).or_insert_with(|| name.to_string());
            }
        }
    }
    m
});

/// The service name for `port` on `proto` (`tcp`/`udp`), if known.
fn service_name(port: u16, proto: &str) -> String {
    if port == 0 {
        return String::new();
    }
    SERVICES.get(&(port, proto.to_string())).cloned().unwrap_or_default()
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
        let proto = proto_label(t[0], local);
        let state = t[1].to_string();
        let is_listener = state == "LISTEN" || state == "UNCONN";
        // The service is named after the notable port: the local port for a
        // listener, the peer (server) port for an outbound connection.
        let base_proto = proto.trim_end_matches('6');
        let svc_port = if is_listener { port_of(local) } else { port_of(peer) };
        let service = service_name(svc_port as u16, base_proto);
        // Keep the `ss -i` info (everything except the process token) for details.
        let info = rest
            .iter()
            .filter(|s| !s.starts_with("users:"))
            .copied()
            .collect::<Vec<_>>()
            .join(" ");
        let sock = Socket {
            proto,
            state,
            local: local.to_string(),
            peer: peer.to_string(),
            service,
            program,
            pid,
            rx,
            tx,
            rx_rate: None,
            tx_rate: None,
            info,
        };
        if is_listener {
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
mod tests;
