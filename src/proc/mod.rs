//! Process explorer: a full-screen view listing running processes with their
//! CPU/memory usage (sortable, killable), plus an animated CPU-load line graph,
//! per-core load bars, and a memory display. Sampling is cross-platform via the
//! [`sysinfo`] crate (Linux, Windows and macOS). The battery readout is the one
//! platform-specific bit: it is read from Linux `/sys` and left unset elsewhere.

pub mod render;

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use std::collections::{HashMap, HashSet, VecDeque};
use sysinfo::{Networks, ProcessesToUpdate, System};

/// Number of CPU-load samples kept for the line graph.
pub const CPU_HISTORY: usize = 160;
/// Number of per-core samples kept for the small per-core graphs.
pub const CORE_HISTORY: usize = 48;
/// Number of samples kept for the memory / disk / network sparklines.
pub const SYS_HISTORY: usize = 120;
/// Number of CPU samples kept per process for the in-row sparkline.
pub const PROC_CPU_HISTORY: usize = 16;

/// One process row.
#[derive(Debug, Clone)]
pub struct ProcInfo {
    pub pid: i32,
    /// Parent PID, if known and different from `pid`. Used to build the tree.
    pub ppid: Option<i32>,
    pub name: String,
    /// CPU usage since the last sample, normalized so one busy core ≈ 100%.
    pub cpu: f32,
    /// Resident set size in bytes.
    pub rss: u64,
    /// RSS as a percentage of total RAM.
    pub mem_pct: f32,
    /// Number of threads.
    pub threads: u32,
}

/// One entry in the flattened, currently-visible process tree. The cursor and
/// scroll offset index into these rows (not directly into [`ProcInfo`]s), so
/// only expanded subtrees contribute rows.
#[derive(Debug, Clone, Copy)]
pub struct ProcRow {
    /// Index into [`ProcView::procs`] of the process shown on this row.
    pub proc_idx: usize,
    /// Indentation depth (0 = a top-level/root process).
    pub depth: u16,
    /// Whether this process has at least one child in the current set.
    pub has_children: bool,
    /// Whether this node is currently expanded (only meaningful with children).
    pub expanded: bool,
}

/// Which column the list is sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcSort {
    Cpu,
    Mem,
    Name,
    Pid,
    Threads,
}

/// Result of routing a key to the process explorer.
pub enum ProcSignal {
    Stay,
    Close,
    /// Request to kill `pid` (after the app confirms). `force` ⇒ SIGKILL.
    Kill { pid: i32, name: String, force: bool },
}

pub struct ProcView {
    /// All sampled processes, flat and sorted by the active column. The tree is
    /// derived from this via parent/child links; siblings inherit this order.
    pub procs: Vec<ProcInfo>,
    /// Flattened, currently-visible tree rows (roots + expanded descendants).
    pub rows: Vec<ProcRow>,
    /// PIDs whose children are hidden. The tree is *expanded by default* (empty
    /// set ⇒ everything visible), because on Linux user processes nest many
    /// levels deep under the systemd session and would otherwise be hidden; a
    /// process is collapsed only once the user (or "collapse all") folds it.
    pub collapsed: HashSet<i32>,
    pub cursor: usize,
    pub offset: usize,
    pub sort: ProcSort,
    pub reverse: bool,
    pub ncores: usize,
    /// Current overall CPU-busy percentage (0..=100), updated every refresh.
    pub cpu_now: f32,
    /// Overall CPU-busy percentage history (0..=100), oldest first. Advanced at a
    /// third of the refresh rate so the line graph scrolls at a readable pace.
    pub cpu_history: VecDeque<f32>,
    /// Per-core busy percentage (0..=100), updated every refresh.
    pub cores: Vec<f32>,
    /// Per-core busy-percentage history (parallel to `cores`).
    pub core_history: Vec<VecDeque<f32>>,
    pub mem_total: u64,
    pub mem_used: u64,
    /// Memory-used percentage history (0..=100) for the memory sparkline.
    pub mem_history: VecDeque<f64>,
    /// Disk read and write rates (bytes/s) and their histories, sampled by
    /// summing each process's read/written bytes since the last refresh.
    pub disk_read: f64,
    pub disk_write: f64,
    pub disk_read_history: VecDeque<f64>,
    pub disk_write_history: VecDeque<f64>,
    /// Network receive/transmit rates (bytes/s) and their histories.
    pub net_down: f64,
    pub net_up: f64,
    pub net_down_history: VecDeque<f64>,
    pub net_up_history: VecDeque<f64>,
    /// Recent CPU% history per process (by PID) for the in-row sparkline.
    pub proc_cpu_history: HashMap<i32, VecDeque<f32>>,
    /// CPU model name (from `/proc/cpuinfo`), shown on the core panel border.
    pub cpu_name: String,
    /// Battery percentage and charging state, if a battery is present.
    pub battery: Option<(u8, bool)>,
    /// Refresh interval in milliseconds (adjustable with +/-, min 100 ms).
    pub interval_ms: u64,
    /// Visible table rows, set by the renderer for paging math.
    pub view_rows: usize,

    // --- sampling state (cross-platform, via the `sysinfo` crate) ---
    /// 100 ms ticks accumulated since the last refresh.
    tick_accum: u64,
    refresh_count: u64,
    /// Wall-clock of the previous sample, used to convert sysinfo's
    /// bytes-since-last-refresh counters into per-second disk/network rates.
    last_instant: Option<std::time::Instant>,
    /// Live system handle; sysinfo computes per-refresh deltas internally.
    sys: System,
    /// Network interface byte counters (received/transmitted since last refresh).
    networks: Networks,
}

impl ProcView {
    pub fn new() -> Self {
        // Build the sysinfo handle and take a CPU baseline so the model name and
        // core count are known up front (and the first delta has a reference).
        let mut sys = System::new();
        sys.refresh_cpu_all();
        let cpu_name = sys
            .cpus()
            .first()
            .map(|c| c.brand().trim().to_string())
            .unwrap_or_default();
        let ncores = sys.cpus().len().max(1);
        let networks = Networks::new_with_refreshed_list();

        let mut v = ProcView {
            procs: Vec::new(),
            rows: Vec::new(),
            collapsed: HashSet::new(),
            cursor: 0,
            offset: 0,
            sort: ProcSort::Cpu,
            reverse: true, // CPU descending by default
            ncores,
            cpu_now: 0.0,
            cpu_history: VecDeque::with_capacity(CPU_HISTORY),
            cores: Vec::new(),
            core_history: Vec::new(),
            mem_total: 0,
            mem_used: 0,
            mem_history: VecDeque::with_capacity(SYS_HISTORY),
            disk_read: 0.0,
            disk_write: 0.0,
            disk_read_history: VecDeque::with_capacity(SYS_HISTORY),
            disk_write_history: VecDeque::with_capacity(SYS_HISTORY),
            net_down: 0.0,
            net_up: 0.0,
            net_down_history: VecDeque::with_capacity(SYS_HISTORY),
            net_up_history: VecDeque::with_capacity(SYS_HISTORY),
            proc_cpu_history: HashMap::new(),
            cpu_name,
            battery: None,
            interval_ms: 300,
            view_rows: 1,
            tick_accum: 0,
            refresh_count: 0,
            last_instant: None,
            sys,
            networks,
        };
        // Baseline sample so the next refresh can compute deltas.
        v.refresh();
        v
    }

    // --- key handling -----------------------------------------------------

    pub fn handle_key(&mut self, key: KeyEvent) -> ProcSignal {
        match key.code {
            KeyCode::Esc | KeyCode::F(10) | KeyCode::Char('q') | KeyCode::Char('Q') => {
                ProcSignal::Close
            }
            KeyCode::Up => {
                self.move_cursor(-1);
                ProcSignal::Stay
            }
            KeyCode::Down => {
                self.move_cursor(1);
                ProcSignal::Stay
            }
            KeyCode::PageUp => {
                self.move_cursor(-(self.view_rows as isize));
                ProcSignal::Stay
            }
            KeyCode::PageDown => {
                self.move_cursor(self.view_rows as isize);
                ProcSignal::Stay
            }
            KeyCode::Home => {
                self.cursor = 0;
                ProcSignal::Stay
            }
            KeyCode::End => {
                self.cursor = self.rows.len().saturating_sub(1);
                ProcSignal::Stay
            }
            // Tree navigation: → expands (or steps into) the current node,
            // ← collapses it (or steps out to the parent).
            KeyCode::Right => {
                self.expand_or_enter();
                ProcSignal::Stay
            }
            KeyCode::Left => {
                self.collapse_or_leave();
                ProcSignal::Stay
            }
            // Toggle the current subtree open/closed.
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.toggle_expand();
                ProcSignal::Stay
            }
            // Collapse every subtree / expand them all again.
            KeyCode::Char('*') => {
                self.toggle_all();
                ProcSignal::Stay
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                self.set_sort(ProcSort::Cpu);
                ProcSignal::Stay
            }
            KeyCode::Char('m') | KeyCode::Char('M') => {
                self.set_sort(ProcSort::Mem);
                ProcSignal::Stay
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                self.set_sort(ProcSort::Name);
                ProcSignal::Stay
            }
            KeyCode::Char('p') | KeyCode::Char('P') => {
                self.set_sort(ProcSort::Pid);
                ProcSignal::Stay
            }
            KeyCode::Char('t') | KeyCode::Char('T') => {
                self.set_sort(ProcSort::Threads);
                ProcSignal::Stay
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.reverse = !self.reverse;
                self.resort_keep_cursor();
                ProcSignal::Stay
            }
            // Update interval: + slower, - faster (100 ms steps, min 100 ms).
            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.interval_ms = (self.interval_ms + 100).min(10_000);
                ProcSignal::Stay
            }
            KeyCode::Char('-') | KeyCode::Char('_') => {
                self.interval_ms = self.interval_ms.saturating_sub(100).max(100);
                ProcSignal::Stay
            }
            // Kill: SIGTERM (k/F8/F9/Delete) or SIGKILL (K).
            KeyCode::Char('K') => self.kill_request(true),
            KeyCode::Char('k') | KeyCode::Delete | KeyCode::F(8) | KeyCode::F(9) => {
                self.kill_request(false)
            }
            _ => ProcSignal::Stay,
        }
    }

    fn kill_request(&self, force: bool) -> ProcSignal {
        match self.cursor_proc() {
            Some(p) => ProcSignal::Kill {
                pid: p.pid,
                name: p.name.clone(),
                force,
            },
            None => ProcSignal::Stay,
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.rows.is_empty() {
            self.cursor = 0;
            return;
        }
        let max = self.rows.len() as isize - 1;
        self.cursor = (self.cursor as isize + delta).clamp(0, max) as usize;
    }

    /// The process the cursor is currently on (via the visible tree row).
    pub fn cursor_proc(&self) -> Option<&ProcInfo> {
        self.rows.get(self.cursor).map(|r| &self.procs[r.proc_idx])
    }

    /// PID under the cursor, if any.
    fn cursor_pid(&self) -> Option<i32> {
        self.cursor_proc().map(|p| p.pid)
    }

    /// → key: expand a collapsed node, or step onto the first child if already
    /// expanded. No-op on a leaf.
    fn expand_or_enter(&mut self) {
        let Some(row) = self.rows.get(self.cursor).copied() else {
            return;
        };
        if !row.has_children {
            return;
        }
        if row.expanded {
            self.move_cursor(1); // step into the (immediately following) first child
        } else {
            let pid = self.procs[row.proc_idx].pid;
            self.collapsed.remove(&pid);
            self.rebuild_rows();
            self.restore_cursor(Some(pid));
        }
    }

    /// ← key: collapse an expanded node, or step out to the parent row.
    fn collapse_or_leave(&mut self) {
        let Some(row) = self.rows.get(self.cursor).copied() else {
            return;
        };
        if row.has_children && row.expanded {
            let pid = self.procs[row.proc_idx].pid;
            self.collapsed.insert(pid);
            self.rebuild_rows();
            self.restore_cursor(Some(pid));
        } else if row.depth > 0 {
            // Walk up to the nearest shallower row (this node's parent).
            let mut i = self.cursor;
            while i > 0 {
                i -= 1;
                if self.rows[i].depth < row.depth {
                    self.cursor = i;
                    break;
                }
            }
        }
    }

    /// Enter/Space: toggle the current node between expanded and collapsed.
    fn toggle_expand(&mut self) {
        let Some(row) = self.rows.get(self.cursor).copied() else {
            return;
        };
        if !row.has_children {
            return;
        }
        let pid = self.procs[row.proc_idx].pid;
        if !self.collapsed.remove(&pid) {
            self.collapsed.insert(pid);
        }
        self.rebuild_rows();
        self.restore_cursor(Some(pid));
    }

    /// `*` key: collapse everything if anything is currently expanded, otherwise
    /// expand everything back. Keeps the cursor on its process where possible.
    fn toggle_all(&mut self) {
        let pid = self.cursor_pid();
        if self.collapsed.is_empty() {
            // Fold every process (collapsing a leaf is harmless — it only takes
            // effect once a process has children).
            self.collapsed = self.procs.iter().map(|p| p.pid).collect();
        } else {
            self.collapsed.clear();
        }
        self.rebuild_rows();
        self.restore_cursor(pid);
    }

    fn set_sort(&mut self, key: ProcSort) {
        if self.sort == key {
            self.reverse = !self.reverse;
        } else {
            self.sort = key;
            // Numeric columns default to descending, names to ascending.
            self.reverse = matches!(key, ProcSort::Cpu | ProcSort::Mem | ProcSort::Threads);
        }
        self.resort_keep_cursor();
    }

    /// Re-sort, keeping the cursor on the same process.
    fn resort_keep_cursor(&mut self) {
        let pid = self.cursor_pid();
        self.sort_procs();
        self.rebuild_rows();
        self.restore_cursor(pid);
    }

    /// Rebuild the flattened visible tree ([`Self::rows`]) from the current
    /// (already-sorted) `procs` and the `collapsed` set (empty ⇒ fully expanded).
    ///
    /// Top-level rows are the *display roots*: every "reaper" root (a process
    /// whose parent isn't among the sampled processes, e.g. pid 1 `init`/`systemd`
    /// and pid 2 `kthreadd`), plus the direct children of **init** (pid 1). On
    /// Linux almost everything descends from pid 1, so promoting init's children
    /// is what lifts the real services and apps to the top level instead of
    /// showing just `init` itself. Crucially, other reapers such as `kthreadd`
    /// are *not* unwrapped, so the hundreds of kernel worker threads stay folded
    /// inside a single collapsible `kthreadd` subtree rather than flooding the
    /// list. Every remaining process hangs under its parent and is only emitted
    /// once that parent is expanded. Siblings (and roots) keep `procs`' order, so
    /// the active sort column drives the ordering at every level.
    fn rebuild_rows(&mut self) {
        let n = self.procs.len();
        let mut by_pid: HashMap<i32, usize> = HashMap::with_capacity(n);
        for (i, p) in self.procs.iter().enumerate() {
            by_pid.insert(p.pid, i);
        }
        // children[parent_pid] = child indices, in sorted `procs` order. A process
        // is a display root when it's a reaper root or a direct child of init;
        // everything else nests under its parent.
        let mut children: HashMap<i32, Vec<usize>> = HashMap::new();
        let mut roots: Vec<usize> = Vec::new();
        for (i, p) in self.procs.iter().enumerate() {
            if is_reaper_root(&self.procs, &by_pid, i) || p.ppid == Some(INIT_PID) {
                roots.push(i);
            } else {
                // Not a display root ⇒ its parent is present and isn't init.
                children.entry(p.ppid.unwrap()).or_default().push(i);
            }
        }

        // A parent/child cycle (A→B→A) leaves nodes with a present parent but no
        // path to a real root. Flood the full tree from the roots, then promote
        // any still-unreached node to a root so nothing disappears from the list.
        let mut reachable = vec![false; n];
        let mut stack: Vec<usize> = Vec::new();
        for &r in &roots {
            if !reachable[r] {
                reachable[r] = true;
                stack.push(r);
            }
        }
        flood_reachable(&self.procs, &children, &mut reachable, &mut stack);
        for i in 0..n {
            if !reachable[i] {
                roots.push(i);
                reachable[i] = true;
                stack.push(i);
                flood_reachable(&self.procs, &children, &mut reachable, &mut stack);
            }
        }

        // Emit the visible rows: each root and, unless collapsed, its descendants.
        let mut rows: Vec<ProcRow> = Vec::with_capacity(n);
        let mut emitted = vec![false; n];
        for &r in &roots {
            dfs_emit(&self.procs, &children, &self.collapsed, &mut emitted, &mut rows, r, 0);
        }
        self.rows = rows;
    }

    /// Sort the process list by the current key/direction (no cursor handling).
    fn sort_procs(&mut self) {
        match self.sort {
            ProcSort::Cpu => self
                .procs
                .sort_by(|a, b| a.cpu.partial_cmp(&b.cpu).unwrap_or(std::cmp::Ordering::Equal)),
            ProcSort::Mem => self.procs.sort_by_key(|p| p.rss),
            ProcSort::Name => self.procs.sort_by_key(|p| p.name.to_lowercase()),
            ProcSort::Pid => self.procs.sort_by_key(|p| p.pid),
            ProcSort::Threads => self.procs.sort_by_key(|p| p.threads),
        }
        if self.reverse {
            self.procs.reverse();
        }
    }

    /// Put the cursor back on process `pid`; if it's gone (or now hidden inside a
    /// collapsed subtree), keep the current row index (clamped), so the selection
    /// moves only to an adjacent visible entry.
    fn restore_cursor(&mut self, pid: Option<i32>) {
        if let Some(pid) = pid
            && let Some(i) = self.rows.iter().position(|r| self.procs[r.proc_idx].pid == pid)
        {
            self.cursor = i;
            return;
        }
        self.clamp_cursor();
    }

    fn clamp_cursor(&mut self) {
        if self.rows.is_empty() {
            self.cursor = 0;
        } else if self.cursor >= self.rows.len() {
            self.cursor = self.rows.len() - 1;
        }
    }

    pub fn cpu_last(&self) -> f32 {
        self.cpu_now
    }

    /// Called every 100 ms tick; returns true (and resets) when the refresh
    /// interval has elapsed, so the caller should `refresh()`.
    pub fn tick_due(&mut self) -> bool {
        self.tick_accum += 1;
        if self.tick_accum * 100 >= self.interval_ms {
            self.tick_accum = 0;
            true
        } else {
            false
        }
    }
}

impl Default for ProcView {
    fn default() -> Self {
        Self::new()
    }
}

fn push_hist(hist: &mut VecDeque<f32>, v: f32) {
    if hist.len() >= CPU_HISTORY {
        hist.pop_front();
    }
    hist.push_back(v);
}

fn push_core_hist(hist: &mut VecDeque<f32>, v: f32) {
    if hist.len() >= CORE_HISTORY {
        hist.pop_front();
    }
    hist.push_back(v);
}

fn push_sys(hist: &mut VecDeque<f64>, v: f64) {
    if hist.len() >= SYS_HISTORY {
        hist.pop_front();
    }
    hist.push_back(v);
}

/// PID of the init process (pid 1) on Unix — the parent of the real top-level
/// services/apps, whose children are promoted to the display's top level. On
/// Windows there is no pid 1, so this simply never matches and the natural
/// reaper roots stand as the top level.
const INIT_PID: i32 = 1;

/// Whether `procs[idx]` is a "reaper" root: a process with no parent among the
/// sampled set (or that is its own parent). These are the tops of the real
/// process forest (e.g. pid 1 `init` and pid 2 `kthreadd` on Linux) and each
/// becomes a top-level row.
fn is_reaper_root(procs: &[ProcInfo], by_pid: &HashMap<i32, usize>, idx: usize) -> bool {
    match procs[idx].ppid {
        Some(pp) => pp == procs[idx].pid || !by_pid.contains_key(&pp),
        None => true,
    }
}

/// Flood-fill `reachable` over the full child graph, draining `stack` (which the
/// caller seeds with already-reachable roots). Used to find nodes stranded by a
/// parent/child cycle so they can be promoted to roots.
fn flood_reachable(
    procs: &[ProcInfo],
    children: &HashMap<i32, Vec<usize>>,
    reachable: &mut [bool],
    stack: &mut Vec<usize>,
) {
    while let Some(idx) = stack.pop() {
        let pid = procs[idx].pid;
        if let Some(kids) = children.get(&pid) {
            for &c in kids {
                if !reachable[c] {
                    reachable[c] = true;
                    stack.push(c);
                }
            }
        }
    }
}

/// Pre-order depth-first walk from `start`, appending a [`ProcRow`] for it and,
/// when it's expanded, for its descendants. A node is expanded unless its PID is
/// in `collapsed`. `emitted` guards against emitting a node twice (and against
/// cycles). Children are pushed in reverse so they pop in `procs` (sorted) order.
fn dfs_emit(
    procs: &[ProcInfo],
    children: &HashMap<i32, Vec<usize>>,
    collapsed: &HashSet<i32>,
    emitted: &mut [bool],
    rows: &mut Vec<ProcRow>,
    start: usize,
    start_depth: u16,
) {
    let mut stack = vec![(start, start_depth)];
    while let Some((idx, depth)) = stack.pop() {
        if emitted[idx] {
            continue;
        }
        emitted[idx] = true;
        let pid = procs[idx].pid;
        let kids = children.get(&pid);
        let has_children = kids.is_some_and(|k| !k.is_empty());
        let is_expanded = has_children && !collapsed.contains(&pid);
        rows.push(ProcRow { proc_idx: idx, depth, has_children, expanded: is_expanded });
        if is_expanded && let Some(kids) = kids {
            for &c in kids.iter().rev() {
                if !emitted[c] {
                    stack.push((c, depth + 1));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Sampling (cross-platform, via `sysinfo`)
// ---------------------------------------------------------------------------

impl ProcView {
    /// Re-read CPU, memory, per-process and disk/network stats. `sysinfo` keeps
    /// the previous sample internally, so its byte counters are already deltas
    /// since the last `refresh()`.
    pub fn refresh(&mut self) {
        // Remember which process the cursor is on so it stays put across the
        // re-sort (and only moves if that process is gone).
        let anchor = self.cursor_pid();
        // Seconds since the last sample, to turn sysinfo's bytes-since-refresh
        // disk/network counters into per-second rates.
        let now = std::time::Instant::now();
        let dt = self
            .last_instant
            .map(|prev| now.duration_since(prev).as_secs_f64())
            .unwrap_or(0.0);
        self.last_instant = Some(now);

        self.sys.refresh_cpu_all();
        self.sys.refresh_memory();
        self.sys.refresh_processes(ProcessesToUpdate::All, true);

        // -- CPU (overall + per-core busy %). --
        self.cpu_now = self.sys.global_cpu_usage().clamp(0.0, 100.0);
        let cpus = self.sys.cpus();
        if !cpus.is_empty() {
            self.ncores = cpus.len();
            self.cores = cpus.iter().map(|c| c.cpu_usage().clamp(0.0, 100.0)).collect();
        }

        // -- Memory (bytes). --
        self.mem_total = self.sys.total_memory();
        self.mem_used = self.sys.used_memory();

        // -- Processes, plus the system-wide disk read/write throughput summed
        //    from each process's bytes read/written since the last refresh. --
        let max_cpu = 100.0 * self.ncores as f32;
        let mut procs = Vec::with_capacity(self.sys.processes().len());
        let (mut disk_read, mut disk_write) = (0u64, 0u64);
        for (pid, p) in self.sys.processes() {
            let pid = pid.as_u32() as i32;
            let ppid = p.parent().map(|pp| pp.as_u32() as i32);
            let name = p.name().to_string_lossy().into_owned();
            let cpu = p.cpu_usage().clamp(0.0, max_cpu);
            let rss = p.memory();
            let mem_pct = if self.mem_total > 0 {
                100.0 * rss as f32 / self.mem_total as f32
            } else {
                0.0
            };
            // Thread count is only exposed by sysinfo where process "tasks" are
            // available (Linux); it reads 0 elsewhere (e.g. Windows/macOS).
            let threads = p.tasks().map(|t| t.len() as u32).unwrap_or(0);
            let du = p.disk_usage();
            disk_read = disk_read.saturating_add(du.read_bytes);
            disk_write = disk_write.saturating_add(du.written_bytes);

            // Append to this process's CPU sparkline history.
            let h = self.proc_cpu_history.entry(pid).or_default();
            if h.len() >= PROC_CPU_HISTORY {
                h.pop_front();
            }
            h.push_back(cpu);

            procs.push(ProcInfo { pid, ppid, name, cpu, rss, mem_pct, threads });
        }
        // Drop sparkline history and collapse state for processes that exited.
        let live: HashSet<i32> = procs.iter().map(|p| p.pid).collect();
        self.proc_cpu_history.retain(|pid, _| live.contains(pid));
        self.collapsed.retain(|pid| live.contains(pid));
        self.procs = procs;
        self.disk_read = if dt > 0.0 { disk_read as f64 / dt } else { 0.0 };
        self.disk_write = if dt > 0.0 { disk_write as f64 / dt } else { 0.0 };

        // -- Network throughput (sum of non-loopback interfaces). --
        self.networks.refresh(true);
        let (mut rx, mut tx) = (0u64, 0u64);
        for (name, data) in &self.networks {
            if is_loopback(name) {
                continue;
            }
            rx = rx.saturating_add(data.received());
            tx = tx.saturating_add(data.transmitted());
        }
        self.net_down = if dt > 0.0 { rx as f64 / dt } else { 0.0 };
        self.net_up = if dt > 0.0 { tx as f64 / dt } else { 0.0 };

        // -- Battery (platform-specific; sysinfo doesn't sample it). --
        self.sample_battery();

        self.sort_procs();
        self.rebuild_rows();
        self.restore_cursor(anchor);

        // Advance the history graphs at a third of the refresh rate.
        self.refresh_count = self.refresh_count.wrapping_add(1);
        if self.refresh_count.is_multiple_of(3) {
            push_hist(&mut self.cpu_history, self.cpu_now);
            if self.core_history.len() < self.cores.len() {
                self.core_history
                    .resize(self.cores.len(), VecDeque::with_capacity(CORE_HISTORY));
            }
            for (i, &v) in self.cores.iter().enumerate() {
                push_core_hist(&mut self.core_history[i], v);
            }
            let mem_pct = if self.mem_total > 0 {
                100.0 * self.mem_used as f64 / self.mem_total as f64
            } else {
                0.0
            };
            push_sys(&mut self.mem_history, mem_pct);
            push_sys(&mut self.disk_read_history, self.disk_read);
            push_sys(&mut self.disk_write_history, self.disk_write);
            push_sys(&mut self.net_down_history, self.net_down);
            push_sys(&mut self.net_up_history, self.net_up);
        }
    }

    /// Read the first battery's charge percentage and charging state, if any.
    #[cfg(target_os = "linux")]
    fn sample_battery(&mut self) {
        let Ok(rd) = std::fs::read_dir("/sys/class/power_supply") else {
            self.battery = None;
            return;
        };
        for e in rd.flatten() {
            let name = e.file_name();
            if !name.to_string_lossy().starts_with("BAT") {
                continue;
            }
            let dir = e.path();
            let Some(pct) = std::fs::read_to_string(dir.join("capacity"))
                .ok()
                .and_then(|s| s.trim().parse::<u8>().ok())
            else {
                continue;
            };
            let charging = std::fs::read_to_string(dir.join("status"))
                .map(|s| s.trim().eq_ignore_ascii_case("Charging"))
                .unwrap_or(false);
            self.battery = Some((pct.min(100), charging));
            return;
        }
        self.battery = None;
    }

    /// Battery state isn't sampled via sysinfo, so it's left unset off Linux.
    #[cfg(not(target_os = "linux"))]
    fn sample_battery(&mut self) {
        self.battery = None;
    }
}

/// Whether a network interface name is a loopback device (excluded from the
/// throughput totals): `lo` on Linux, `lo0` on macOS, "Loopback…" on Windows.
fn is_loopback(name: &str) -> bool {
    name == "lo" || name == "lo0" || name.to_ascii_lowercase().contains("loopback")
}

#[cfg(test)]
mod tests {
    #[test]
    fn lists_running_processes() {
        // sysinfo enumerates processes/CPU/memory on every supported platform,
        // so this runs cross-platform (Linux, Windows, macOS).
        let pv = super::ProcView::new();
        assert!(!pv.procs.is_empty(), "should see at least this test process");
        assert!(pv.ncores >= 1);
        assert!(pv.mem_total > 0, "memory total should be read");
    }

    #[test]
    fn cursor_follows_process_across_resort() {
        use super::{ProcInfo, ProcView};
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut pv = ProcView::new();
        pv.procs = vec![
            ProcInfo { pid: 1, ppid: None, name: "a".into(), cpu: 10.0, rss: 0, mem_pct: 0.0, threads: 1 },
            ProcInfo { pid: 2, ppid: None, name: "b".into(), cpu: 50.0, rss: 0, mem_pct: 0.0, threads: 1 },
            ProcInfo { pid: 3, ppid: None, name: "c".into(), cpu: 30.0, rss: 0, mem_pct: 0.0, threads: 1 },
        ];
        pv.sort = super::ProcSort::Pid;
        pv.reverse = false;
        pv.rebuild_rows();
        pv.cursor = 1; // on pid 2

        // Sort by CPU (descending): order becomes 2,3,1 — cursor must stay on pid 2.
        pv.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert_eq!(pv.cursor_pid(), Some(2), "cursor stays on the same process");
    }

    #[test]
    fn renders_without_panic() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut pv = super::ProcView::new();
        let theme = crate::ui::theme::Theme::mc();
        let mut t = Terminal::new(TestBackend::new(120, 30)).unwrap();
        t.draw(|f| super::render::render(f, f.area(), &mut pv, &theme, None))
            .unwrap();
        let buf = t.backend().buffer();
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                s.push_str(buf[(x, y)].symbol());
            }
        }
        assert!(s.contains("Process Explorer"), "title present");
        assert!(s.contains("CPU"), "cpu graph label present");
        assert!(s.contains("PID"), "table header present");
        assert!(s.contains("[T]HR"), "threads column present");
        assert!(s.contains("cpu"), "per-process cpu sparkline column header present");
        assert!(s.contains("Mem"), "memory panel present");
        assert!(s.contains("Disk"), "disk panel present");
        assert!(s.contains("Net"), "network panel present");
        assert!(s.contains("300ms"), "update interval shown on the border");
    }

    #[test]
    fn interval_keys_adjust_and_clamp() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut pv = super::ProcView::new();
        pv.interval_ms = 300;
        let key = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        pv.handle_key(key('+'));
        assert_eq!(pv.interval_ms, 400, "+ raises the interval by 100ms");
        for _ in 0..5 {
            pv.handle_key(key('-'));
        }
        assert_eq!(pv.interval_ms, 100, "- lowers but never below 100ms");
    }

    #[test]
    fn tick_due_fires_each_interval() {
        let mut pv = super::ProcView::new();
        pv.interval_ms = 300;
        assert!(!pv.tick_due(), "100ms < 300ms");
        assert!(!pv.tick_due(), "200ms < 300ms");
        assert!(pv.tick_due(), "300ms reaches the interval");
        assert!(!pv.tick_due(), "counter reset after firing");
    }

    #[test]
    fn sort_by_threads_orders_descending() {
        use super::{ProcInfo, ProcView};
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut pv = ProcView::new();
        pv.procs = vec![
            ProcInfo { pid: 1, ppid: None, name: "a".into(), cpu: 0.0, rss: 0, mem_pct: 0.0, threads: 4 },
            ProcInfo { pid: 2, ppid: None, name: "b".into(), cpu: 0.0, rss: 0, mem_pct: 0.0, threads: 32 },
            ProcInfo { pid: 3, ppid: None, name: "c".into(), cpu: 0.0, rss: 0, mem_pct: 0.0, threads: 9 },
        ];
        pv.rebuild_rows();
        pv.cursor = 0;
        pv.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        let threads: Vec<u32> = pv.procs.iter().map(|p| p.threads).collect();
        assert_eq!(threads, vec![32, 9, 4], "threads sort is descending by default");
    }

    /// The tree is expanded by default (so deeply-nested processes stay visible),
    /// init's children are lifted to the top level, `←`/`→`/`*` fold and unfold
    /// subtrees, and the cursor follows across the changes.
    #[test]
    fn tree_expands_and_collapses() {
        use super::{ProcInfo, ProcSort, ProcView};
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mk = |pid, ppid, name: &str, cpu| ProcInfo {
            pid,
            ppid,
            name: name.into(),
            cpu,
            rss: 0,
            mem_pct: 0.0,
            threads: 1,
        };
        let mut pv = ProcView::new();
        // Two reapers: pid 1 (init) and pid 2 (kthreadd). 10 is an app under
        // init with two workers; 20 is a kernel thread under kthreadd.
        pv.procs = vec![
            mk(1, None, "init", 0.0),
            mk(2, None, "kthreadd", 3.0),
            mk(20, Some(2), "kworker/0", 1.0),
            mk(10, Some(1), "app", 50.0),
            mk(11, Some(10), "worker-a", 90.0),
            mk(12, Some(10), "worker-b", 10.0),
        ];
        pv.sort = ProcSort::Cpu;
        pv.reverse = true;
        pv.sort_procs();
        pv.rebuild_rows();
        let visible = |pv: &ProcView| -> Vec<i32> {
            pv.rows.iter().map(|r| pv.procs[r.proc_idx].pid).collect()
        };
        let row_of = |pv: &ProcView, pid: i32| pv.rows.iter().position(|r| pv.procs[r.proc_idx].pid == pid).unwrap();

        // Expanded default: every process is visible. init's child (app) and the
        // reapers are top-level (CPU-descending: app 50 > kthreadd 3 > init 0),
        // with the app's workers and kthreadd's kernel thread nested beneath.
        assert_eq!(visible(&pv), vec![10, 11, 12, 2, 20, 1], "all processes visible, grouped as a tree");
        assert_eq!(pv.rows[row_of(&pv, 11)].depth, 1, "workers are indented under the app");
        assert_eq!(pv.rows[row_of(&pv, 20)].depth, 1, "kernel thread indented under kthreadd");
        assert!(pv.rows[0].expanded, "the app starts expanded");

        // Collapse the app (←): its workers hide, the cursor stays on the app.
        pv.cursor = row_of(&pv, 10);
        pv.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(visible(&pv), vec![10, 2, 20, 1], "collapsing the app hides its workers");
        assert_eq!(pv.cursor_pid(), Some(10), "cursor stays on the collapsed app");

        // Re-expand it (→): the workers come back.
        pv.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(visible(&pv), vec![10, 11, 12, 2, 20, 1], "re-expanding restores the workers");

        // `*` collapses everything to just the top level, then restores it all.
        pv.handle_key(KeyEvent::new(KeyCode::Char('*'), KeyModifiers::NONE));
        assert_eq!(visible(&pv), vec![10, 2, 1], "collapse-all leaves only the top level");
        pv.handle_key(KeyEvent::new(KeyCode::Char('*'), KeyModifiers::NONE));
        assert_eq!(visible(&pv), vec![10, 11, 12, 2, 20, 1], "expand-all restores every row");
    }
}
