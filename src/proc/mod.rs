//! Process explorer: a full-screen view listing running processes with their
//! CPU/memory usage (sortable, killable), plus an animated CPU-load line graph,
//! per-core load bars, and a memory display. Sampling is self-contained, reading
//! Linux `/proc`; on other platforms the list is simply empty.

pub mod render;

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use std::collections::{HashMap, VecDeque};

/// Number of CPU-load samples kept for the line graph.
pub const CPU_HISTORY: usize = 160;

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
}

/// Which column the list is sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcSort {
    Cpu,
    Mem,
    Name,
    Pid,
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
    /// Overall CPU-busy percentage history (0..=100), oldest first.
    pub cpu_history: VecDeque<f32>,
    /// Per-core busy percentage (0..=100).
    pub cores: Vec<f32>,
    pub mem_total: u64,
    pub mem_used: u64,
    /// Visible table rows, set by the renderer for paging math.
    pub view_rows: usize,

    // --- sampling state ---
    prev_pid: HashMap<i32, u64>,
    prev_total: u64,
    prev_idle: u64,
    prev_cores: Vec<(u64, u64)>, // (idle_all, total) per core
    last_total_delta: u64,
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
            cpu_history: VecDeque::with_capacity(CPU_HISTORY),
            cores: Vec::new(),
            mem_total: 0,
            mem_used: 0,
            view_rows: 1,
            prev_pid: HashMap::new(),
            prev_total: 0,
            prev_idle: 0,
            prev_cores: Vec::new(),
            last_total_delta: 0,
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
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.reverse = !self.reverse;
                self.sort_procs();
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
            self.reverse = matches!(key, ProcSort::Cpu | ProcSort::Mem);
        }
        self.sort_procs();
    }

    fn sort_procs(&mut self) {
        // Keep the cursor on the same process across a re-sort.
        let pid_at_cursor = self.procs.get(self.cursor).map(|p| p.pid);
        match self.sort {
            ProcSort::Cpu => self
                .procs
                .sort_by(|a, b| a.cpu.partial_cmp(&b.cpu).unwrap_or(std::cmp::Ordering::Equal)),
            ProcSort::Mem => self.procs.sort_by_key(|p| p.rss),
            ProcSort::Name => self.procs.sort_by_key(|p| p.name.to_lowercase()),
            ProcSort::Pid => self.procs.sort_by_key(|p| p.pid),
        }
        if self.reverse {
            self.procs.reverse();
        }
        if let Some(pid) = pid_at_cursor
            && let Some(i) = self.procs.iter().position(|p| p.pid == pid)
        {
            self.cursor = i;
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
        self.cpu_history.back().copied().unwrap_or(0.0)
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

// ---------------------------------------------------------------------------
// Sampling (Linux /proc)
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
impl ProcView {
    /// Re-read CPU, memory and per-process stats and recompute usage deltas.
    pub fn refresh(&mut self) {
        self.sample_cpu();
        self.sample_mem();
        self.sample_procs();
        self.sort_procs();
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
                if self.prev_total != 0 {
                    let busy = if dt > 0 {
                        100.0 * (1.0 - di as f32 / dt as f32)
                    } else {
                        0.0
                    };
                    push_hist(&mut self.cpu_history, busy.clamp(0.0, 100.0));
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
            let Some((name, jiffies)) = parse_proc_stat(&stat) else {
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
            procs.push(ProcInfo {
                pid,
                name,
                cpu: cpu.clamp(0.0, 100.0 * self.ncores as f32),
                rss,
                mem_pct,
            });
        }
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

/// Parse a `/proc/<pid>/stat` line into `(comm, utime+stime)`. The command name
/// can contain spaces and parentheses, so we split on the last `)`.
#[cfg(target_os = "linux")]
fn parse_proc_stat(stat: &str) -> Option<(String, u64)> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let comm = stat.get(open + 1..close)?.to_string();
    let rest = stat.get(close + 1..)?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // After the ')' the tokens start at field 3 (state); utime is field 14,
    // stime field 15 → indices 11 and 12 here.
    let utime = fields.get(11)?.parse::<u64>().ok()?;
    let stime = fields.get(12)?.parse::<u64>().ok()?;
    Some((comm, utime + stime))
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
        let (name, jiffies) = super::parse_proc_stat(line).unwrap();
        assert_eq!(name, "weird (name) x");
        assert_eq!(jiffies, 111 + 222);
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
    }
}
