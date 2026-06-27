//! Lightweight system stats sampler (Linux `/proc`). Self-contained — no
//! external crates. Used by the menu-bar status widget.

use std::collections::VecDeque;

/// Number of CPU-load samples kept for the histogram.
pub const HISTORY: usize = 32;

pub struct SysSampler {
    prev: Option<(u64, u64)>, // (total, idle) jiffies
    /// Recent CPU busy percentages (0..=100), oldest first.
    pub cpu_history: VecDeque<u64>,
    pub mem_used_kb: u64,
    pub mem_total_kb: u64,
}

impl SysSampler {
    pub fn new() -> Self {
        SysSampler {
            prev: None,
            cpu_history: VecDeque::with_capacity(HISTORY),
            mem_used_kb: 0,
            mem_total_kb: 0,
        }
    }

    /// Take one sample (CPU delta since last call + current memory).
    pub fn sample(&mut self) {
        if let Some(pct) = self.cpu_percent() {
            if self.cpu_history.len() >= HISTORY {
                self.cpu_history.pop_front();
            }
            self.cpu_history.push_back(pct);
        }
        self.sample_mem();
    }

    fn cpu_percent(&mut self) -> Option<u64> {
        let stat = std::fs::read_to_string("/proc/stat").ok()?;
        let line = stat.lines().next()?; // "cpu  u n s idle iowait irq softirq steal ..."
        let mut nums = line.split_whitespace().skip(1).filter_map(|t| t.parse::<u64>().ok());
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

        let pct = self.prev.map(|(pt, pi)| {
            let dt = total.saturating_sub(pt);
            let di = idle_all.saturating_sub(pi);
            ((dt.saturating_sub(di)) * 100).checked_div(dt).unwrap_or(0).min(100)
        });
        self.prev = Some((total, idle_all));
        pct
    }

    fn sample_mem(&mut self) {
        let Ok(info) = std::fs::read_to_string("/proc/meminfo") else {
            return;
        };
        let mut total = 0u64;
        let mut available = 0u64;
        for line in info.lines() {
            if let Some(v) = line.strip_prefix("MemTotal:") {
                total = parse_kb(v);
            } else if let Some(v) = line.strip_prefix("MemAvailable:") {
                available = parse_kb(v);
            }
        }
        if total > 0 {
            self.mem_total_kb = total;
            self.mem_used_kb = total.saturating_sub(available);
        }
    }

    pub fn mem_percent(&self) -> u64 {
        (self.mem_used_kb * 100)
            .checked_div(self.mem_total_kb)
            .unwrap_or(0)
            .min(100)
    }

    pub fn cpu_last(&self) -> u64 {
        self.cpu_history.back().copied().unwrap_or(0)
    }
}

impl Default for SysSampler {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_kb(s: &str) -> u64 {
    s.split_whitespace().next().and_then(|t| t.parse().ok()).unwrap_or(0)
}
