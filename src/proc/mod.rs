//! Process explorer: a full-screen view listing running processes with their
//! CPU/memory usage (sortable, killable), plus an animated CPU-load line graph,
//! per-core load bars, and a memory display. Sampling is self-contained, reading
//! Linux `/proc`; on other platforms the list is simply empty.

pub mod render;

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use std::collections::{HashMap, VecDeque};

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
    pub procs: Vec<ProcInfo>,
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
    /// Combined disk read+write rate (bytes/s) and its history.
    pub disk_rate: f64,
    pub disk_history: VecDeque<f64>,
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

    // --- sampling state ---
    /// 100 ms ticks accumulated since the last refresh.
    tick_accum: u64,
    refresh_count: u64,
    prev_pid: HashMap<i32, u64>,
    prev_total: u64,
    prev_idle: u64,
    prev_cores: Vec<(u64, u64)>, // (idle_all, total) per core
    last_total_delta: u64,
    prev_disk_bytes: u64,
    prev_net_rx: u64,
    prev_net_tx: u64,
    last_instant: Option<std::time::Instant>,
}

impl ProcView {
    pub fn new() -> Self {
        let mut v = ProcView {
            procs: Vec::new(),
            cursor: 0,
            offset: 0,
            sort: ProcSort::Cpu,
            reverse: true, // CPU descending by default
            ncores: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            cpu_now: 0.0,
            cpu_history: VecDeque::with_capacity(CPU_HISTORY),
            cores: Vec::new(),
            core_history: Vec::new(),
            mem_total: 0,
            mem_used: 0,
            mem_history: VecDeque::with_capacity(SYS_HISTORY),
            disk_rate: 0.0,
            disk_history: VecDeque::with_capacity(SYS_HISTORY),
            net_down: 0.0,
            net_up: 0.0,
            net_down_history: VecDeque::with_capacity(SYS_HISTORY),
            net_up_history: VecDeque::with_capacity(SYS_HISTORY),
            proc_cpu_history: HashMap::new(),
            cpu_name: read_cpu_name(),
            battery: None,
            interval_ms: 300,
            view_rows: 1,
            tick_accum: 0,
            refresh_count: 0,
            prev_pid: HashMap::new(),
            prev_total: 0,
            prev_idle: 0,
            prev_cores: Vec::new(),
            last_total_delta: 0,
            prev_disk_bytes: 0,
            prev_net_rx: 0,
            prev_net_tx: 0,
            last_instant: None,
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
                self.cursor = self.procs.len().saturating_sub(1);
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
        match self.procs.get(self.cursor) {
            Some(p) => ProcSignal::Kill {
                pid: p.pid,
                name: p.name.clone(),
                force,
            },
            None => ProcSignal::Stay,
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.procs.is_empty() {
            self.cursor = 0;
            return;
        }
        let max = self.procs.len() as isize - 1;
        self.cursor = (self.cursor as isize + delta).clamp(0, max) as usize;
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
        let pid = self.procs.get(self.cursor).map(|p| p.pid);
        self.sort_procs();
        self.restore_cursor(pid);
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

    /// Put the cursor back on process `pid`; if it's gone, keep the current row
    /// index (clamped), so the selection moves only to an adjacent entry.
    fn restore_cursor(&mut self, pid: Option<i32>) {
        if let Some(pid) = pid
            && let Some(i) = self.procs.iter().position(|p| p.pid == pid)
        {
            self.cursor = i;
            return;
        }
        self.clamp_cursor();
    }

    fn clamp_cursor(&mut self) {
        if self.procs.is_empty() {
            self.cursor = 0;
        } else if self.cursor >= self.procs.len() {
            self.cursor = self.procs.len() - 1;
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

// ---------------------------------------------------------------------------
// Sampling (Linux /proc)
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
impl ProcView {
    /// Re-read CPU, memory and per-process stats and recompute usage deltas.
    pub fn refresh(&mut self) {
        // Remember which process the cursor is on so it stays put across the
        // re-sort (and only moves if that process is gone).
        let anchor = self.procs.get(self.cursor).map(|p| p.pid);
        // Seconds since the last sample, for rate (disk/net) computation.
        let now = std::time::Instant::now();
        let dt = self
            .last_instant
            .map(|prev| now.duration_since(prev).as_secs_f64())
            .unwrap_or(0.0);
        self.last_instant = Some(now);

        self.sample_cpu();
        self.sample_mem();
        self.sample_disk(dt);
        self.sample_net(dt);
        self.sample_battery();
        self.sample_procs();
        self.sort_procs();
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
            push_sys(&mut self.disk_history, self.disk_rate);
            push_sys(&mut self.net_down_history, self.net_down);
            push_sys(&mut self.net_up_history, self.net_up);
        }
    }

    fn sample_cpu(&mut self) {
        let Ok(stat) = std::fs::read_to_string("/proc/stat") else {
            return;
        };
        let mut core_idx = 0usize;
        for line in stat.lines() {
            let Some(rest) = line.strip_prefix("cpu") else {
                break; // cpu lines are first; stop at the first non-cpu line
            };
            let (idle_all, total) = match parse_cpu_fields(rest) {
                Some(v) => v,
                None => continue,
            };
            if rest.starts_with(' ') || rest.starts_with('\t') {
                // Aggregate "cpu" line.
                let dt = total.saturating_sub(self.prev_total);
                let di = idle_all.saturating_sub(self.prev_idle);
                self.last_total_delta = dt;
                if self.prev_total != 0 && dt > 0 {
                    self.cpu_now = (100.0 * (1.0 - di as f32 / dt as f32)).clamp(0.0, 100.0);
                }
                self.prev_total = total;
                self.prev_idle = idle_all;
            } else {
                // Per-core "cpuN" line.
                if self.prev_cores.len() <= core_idx {
                    self.prev_cores.resize(core_idx + 1, (0, 0));
                    self.cores.resize(core_idx + 1, 0.0);
                }
                let (pi, pt) = self.prev_cores[core_idx];
                let dt = total.saturating_sub(pt);
                let di = idle_all.saturating_sub(pi);
                if pt != 0 && dt > 0 {
                    self.cores[core_idx] = (100.0 * (1.0 - di as f32 / dt as f32)).clamp(0.0, 100.0);
                }
                self.prev_cores[core_idx] = (idle_all, total);
                core_idx += 1;
            }
        }
        if core_idx > 0 {
            self.ncores = core_idx;
        }
    }

    fn sample_mem(&mut self) {
        let Ok(info) = std::fs::read_to_string("/proc/meminfo") else {
            return;
        };
        let mut total = 0u64;
        let mut available = 0u64;
        for line in info.lines() {
            if let Some(v) = line.strip_prefix("MemTotal:") {
                total = parse_kb(v) * 1024;
            } else if let Some(v) = line.strip_prefix("MemAvailable:") {
                available = parse_kb(v) * 1024;
            }
        }
        if total > 0 {
            self.mem_total = total;
            self.mem_used = total.saturating_sub(available);
        }
    }

    /// Combined read+write throughput across whole disks (bytes/s).
    fn sample_disk(&mut self, dt: f64) {
        // Only count whole-block devices (those listed in /sys/block), skipping
        // partitions, loop/ram/zram and device-mapper to avoid double counting.
        let whole: std::collections::HashSet<String> = std::fs::read_dir("/sys/block")
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| {
                        !(n.starts_with("loop")
                            || n.starts_with("ram")
                            || n.starts_with("zram")
                            || n.starts_with("dm-")
                            || n.starts_with("sr"))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut bytes = 0u64;
        if let Ok(stats) = std::fs::read_to_string("/proc/diskstats") {
            for line in stats.lines() {
                let f: Vec<&str> = line.split_whitespace().collect();
                if f.len() < 10 {
                    continue;
                }
                if !whole.contains(f[2]) {
                    continue;
                }
                let read_sectors: u64 = f[5].parse().unwrap_or(0);
                let write_sectors: u64 = f[9].parse().unwrap_or(0);
                bytes = bytes.saturating_add((read_sectors + write_sectors) * 512);
            }
        }
        if self.prev_disk_bytes != 0 && dt > 0.0 {
            self.disk_rate = bytes.saturating_sub(self.prev_disk_bytes) as f64 / dt;
        }
        self.prev_disk_bytes = bytes;
    }

    /// Read the first battery's charge percentage and charging state, if any.
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

    /// Aggregate receive/transmit throughput across interfaces (bytes/s),
    /// excluding loopback.
    fn sample_net(&mut self, dt: f64) {
        let (mut rx, mut tx) = (0u64, 0u64);
        if let Ok(s) = std::fs::read_to_string("/proc/net/dev") {
            for line in s.lines() {
                let Some((iface, rest)) = line.split_once(':') else {
                    continue;
                };
                if iface.trim() == "lo" {
                    continue;
                }
                let f: Vec<&str> = rest.split_whitespace().collect();
                if f.len() < 9 {
                    continue;
                }
                rx = rx.saturating_add(f[0].parse::<u64>().unwrap_or(0));
                tx = tx.saturating_add(f[8].parse::<u64>().unwrap_or(0));
            }
        }
        if self.prev_net_rx != 0 && dt > 0.0 {
            self.net_down = rx.saturating_sub(self.prev_net_rx) as f64 / dt;
        }
        if self.prev_net_tx != 0 && dt > 0.0 {
            self.net_up = tx.saturating_sub(self.prev_net_tx) as f64 / dt;
        }
        self.prev_net_rx = rx;
        self.prev_net_tx = tx;
    }

    fn sample_procs(&mut self) {
        let Ok(rd) = std::fs::read_dir("/proc") else {
            return;
        };
        let total_delta = self.last_total_delta;
        let mut procs = Vec::with_capacity(self.procs.len().max(64));
        let mut next_prev = HashMap::with_capacity(self.prev_pid.len().max(64));

        for de in rd.flatten() {
            let fname = de.file_name();
            let name_str = fname.to_string_lossy();
            let Ok(pid) = name_str.parse::<i32>() else {
                continue;
            };
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
                continue;
            };
            let Some((name, jiffies, threads)) = parse_proc_stat(&stat) else {
                continue;
            };
            let prev = self.prev_pid.get(&pid).copied().unwrap_or(jiffies);
            let delta = jiffies.saturating_sub(prev);
            let cpu = if total_delta > 0 {
                100.0 * self.ncores as f32 * delta as f32 / total_delta as f32
            } else {
                0.0
            };
            let rss = read_rss(pid);
            let mem_pct = if self.mem_total > 0 {
                100.0 * rss as f32 / self.mem_total as f32
            } else {
                0.0
            };
            next_prev.insert(pid, jiffies);
            let cpu = cpu.clamp(0.0, 100.0 * self.ncores as f32);
            // Append to this process's CPU sparkline history.
            let h = self.proc_cpu_history.entry(pid).or_default();
            if h.len() >= PROC_CPU_HISTORY {
                h.pop_front();
            }
            h.push_back(cpu);
            procs.push(ProcInfo {
                pid,
                name,
                cpu,
                rss,
                mem_pct,
                threads,
            });
        }
        // Drop history for processes that have exited.
        self.proc_cpu_history.retain(|pid, _| next_prev.contains_key(pid));
        self.prev_pid = next_prev;
        self.procs = procs;
    }
}

#[cfg(not(target_os = "linux"))]
impl ProcView {
    pub fn refresh(&mut self) {
        // No /proc on this platform; nothing to sample.
    }
}

/// The CPU model name from `/proc/cpuinfo` (empty if unavailable).
#[cfg(target_os = "linux")]
fn read_cpu_name() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| {
                l.strip_prefix("model name")
                    .map(|r| r.trim_start_matches([' ', '\t', ':']).trim().to_string())
            })
        })
        .unwrap_or_default()
}

#[cfg(not(target_os = "linux"))]
fn read_cpu_name() -> String {
    String::new()
}

/// Parse the numeric fields following the `cpu`/`cpuN` label. Returns
/// `(idle_all, total)` in jiffies.
#[cfg(target_os = "linux")]
fn parse_cpu_fields(rest: &str) -> Option<(u64, u64)> {
    let mut nums = rest.split_whitespace().filter_map(|t| t.parse::<u64>().ok());
    let user = nums.next()?;
    let nice = nums.next()?;
    let system = nums.next()?;
    let idle = nums.next()?;
    let iowait = nums.next().unwrap_or(0);
    let irq = nums.next().unwrap_or(0);
    let softirq = nums.next().unwrap_or(0);
    let steal = nums.next().unwrap_or(0);
    let idle_all = idle + iowait;
    let total = user + nice + system + idle_all + irq + softirq + steal;
    Some((idle_all, total))
}

/// Parse a `/proc/<pid>/stat` line into `(comm, utime+stime, num_threads)`. The
/// command name can contain spaces and parentheses, so we split on the last `)`.
#[cfg(target_os = "linux")]
fn parse_proc_stat(stat: &str) -> Option<(String, u64, u32)> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let comm = stat.get(open + 1..close)?.to_string();
    let rest = stat.get(close + 1..)?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // After the ')' the tokens start at field 3 (state); utime is field 14,
    // stime field 15, num_threads field 20 → indices 11, 12 and 17 here.
    let utime = fields.get(11)?.parse::<u64>().ok()?;
    let stime = fields.get(12)?.parse::<u64>().ok()?;
    let threads = fields.get(17).and_then(|t| t.parse::<u32>().ok()).unwrap_or(1);
    Some((comm, utime + stime, threads))
}

/// Resident set size in bytes from `/proc/<pid>/statm` (resident pages).
#[cfg(target_os = "linux")]
fn read_rss(pid: i32) -> u64 {
    const PAGE: u64 = 4096;
    std::fs::read_to_string(format!("/proc/{pid}/statm"))
        .ok()
        .and_then(|s| s.split_whitespace().nth(1).and_then(|t| t.parse::<u64>().ok()))
        .map(|pages| pages * PAGE)
        .unwrap_or(0)
}

#[cfg(target_os = "linux")]
fn parse_kb(s: &str) -> u64 {
    s.split_whitespace().next().and_then(|t| t.parse().ok()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    #[test]
    fn parses_proc_stat_with_tricky_comm() {
        // comm contains spaces and parentheses; it is wrapped in one outer pair.
        let line = "1234 (weird (name) x) S 1 1234 1234 0 -1 0 0 0 0 0 \
                    111 222 0 0 20 0 1 0 999 0 0";
        let (name, jiffies, threads) = super::parse_proc_stat(line).unwrap();
        assert_eq!(name, "weird (name) x");
        assert_eq!(jiffies, 111 + 222);
        assert_eq!(threads, 1);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_cpu_fields() {
        let (idle_all, total) = super::parse_cpu_fields("  100 0 50 800 20 0 0 0").unwrap();
        assert_eq!(idle_all, 820); // idle + iowait
        assert_eq!(total, 970);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn lists_running_processes() {
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
            ProcInfo { pid: 1, name: "a".into(), cpu: 10.0, rss: 0, mem_pct: 0.0, threads: 1 },
            ProcInfo { pid: 2, name: "b".into(), cpu: 50.0, rss: 0, mem_pct: 0.0, threads: 1 },
            ProcInfo { pid: 3, name: "c".into(), cpu: 30.0, rss: 0, mem_pct: 0.0, threads: 1 },
        ];
        pv.sort = super::ProcSort::Pid;
        pv.reverse = false;
        pv.cursor = 1; // on pid 2

        // Sort by CPU (descending): order becomes 2,3,1 — cursor must stay on pid 2.
        pv.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert_eq!(pv.procs[pv.cursor].pid, 2, "cursor stays on the same process");
    }

    #[test]
    fn renders_without_panic() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut pv = super::ProcView::new();
        let theme = crate::ui::theme::Theme::mc();
        let mut t = Terminal::new(TestBackend::new(120, 30)).unwrap();
        t.draw(|f| super::render::render(f, f.area(), &mut pv, &theme))
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
            ProcInfo { pid: 1, name: "a".into(), cpu: 0.0, rss: 0, mem_pct: 0.0, threads: 4 },
            ProcInfo { pid: 2, name: "b".into(), cpu: 0.0, rss: 0, mem_pct: 0.0, threads: 32 },
            ProcInfo { pid: 3, name: "c".into(), cpu: 0.0, rss: 0, mem_pct: 0.0, threads: 9 },
        ];
        pv.cursor = 0;
        pv.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        let threads: Vec<u32> = pv.procs.iter().map(|p| p.threads).collect();
        assert_eq!(threads, vec![32, 9, 4], "threads sort is descending by default");
    }
}
