//! Lightweight system stats sampler for the menu-bar status widget. Sampled
//! through the `sysinfo` crate (as the process explorer is), so it reads the
//! same on Linux, macOS and Windows rather than only where `/proc` exists.

use std::collections::VecDeque;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, System};

/// Number of CPU-load samples kept for the histogram.
pub const HISTORY: usize = 32;

pub struct SysSampler {
    sys: System,
    /// Whether a CPU baseline has been taken. Usage is a delta between two
    /// refreshes, so the first one only primes the next.
    primed: bool,
    /// Recent CPU busy percentages (0..=100), oldest first.
    pub cpu_history: VecDeque<u64>,
    pub mem_used_kb: u64,
    pub mem_total_kb: u64,
}

impl SysSampler {
    pub fn new() -> Self {
        SysSampler {
            sys: System::new(),
            primed: false,
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
        self.sys.refresh_cpu_specifics(CpuRefreshKind::nothing().with_cpu_usage());
        if !std::mem::replace(&mut self.primed, true) {
            return None;
        }
        let usage = self.sys.global_cpu_usage();
        usage.is_finite().then(|| (usage.round().max(0.0) as u64).min(100))
    }

    fn sample_mem(&mut self) {
        self.sys.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
        let total = self.sys.total_memory() / 1024;
        if total > 0 {
            // Used as "not available", which counts reclaimable cache as free —
            // the figure `free`'s "available" column and btop agree on.
            let available = self.sys.available_memory() / 1024;
            self.mem_total_kb = total;
            self.mem_used_kb = total.saturating_sub(available);
        }
    }

    pub fn mem_percent(&self) -> u64 {
        (self.mem_used_kb * 100).checked_div(self.mem_total_kb).unwrap_or(0).min(100)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sample_is_a_baseline_and_memory_reads_everywhere() {
        let mut s = SysSampler::new();
        s.sample();
        assert!(s.cpu_history.is_empty(), "the first CPU refresh only primes the delta");
        assert!(s.mem_total_kb > 0, "memory is read on every platform");
        s.sample();
        assert_eq!(s.cpu_history.len(), 1);
        assert!(s.cpu_last() <= 100);
        assert!(s.mem_percent() <= 100);
    }
}
