//! Container resource usage. From the container cgroup when there is one,
//! otherwise by walking /proc for every process in the container's pid
//! namespace and summing RSS and CPU time.

use hullr_core::Stats;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Cumulative counters at one instant.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sample {
    pub cpu_usec: u64,
    pub memory: u64,
    pub limit: Option<u64>,
    pub pids: u32,
}

fn clk_tck() -> u64 {
    // SAFETY: sysconf has no preconditions.
    let t = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if t > 0 { t as u64 } else { 100 }
}

fn page_size() -> u64 {
    // SAFETY: sysconf has no preconditions.
    let p = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if p > 0 { p as u64 } else { 4096 }
}

/// (utime + stime in ticks) from a /proc/<pid>/stat line.
pub fn parse_stat_cpu(stat: &str) -> Option<u64> {
    let rest = stat.rsplit_once(')')?.1;
    let f: Vec<&str> = rest.split_whitespace().collect();
    // After the comm field: state is f[0]; utime is field 14 overall -> f[11].
    Some(f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?)
}

/// Sum rx/tx bytes over non-loopback interfaces in /proc/net/dev text.
pub fn parse_net_dev(text: &str) -> (u64, u64) {
    let mut rx = 0;
    let mut tx = 0;
    for l in text.lines().skip(2) {
        let Some((name, rest)) = l.split_once(':') else { continue };
        if name.trim() == "lo" {
            continue;
        }
        let f: Vec<u64> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
        if f.len() >= 9 {
            rx += f[0];
            tx += f[8];
        }
    }
    (rx, tx)
}

/// Pids sharing the pid namespace of `pid1` (seen from the host).
pub fn container_pids(pid1: u32) -> Vec<u32> {
    let Some(want) = crate::ns::ns_id(&pid1.to_string(), "pid") else { return vec![] };
    let Ok(rd) = std::fs::read_dir("/proc") else { return vec![] };
    rd.flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter(|p| crate::ns::ns_id(&p.to_string(), "pid").as_deref() == Some(want.as_str()))
        .collect()
}

pub fn sample_proc(pid1: u32) -> Sample {
    let pids = container_pids(pid1);
    let tick_us = 1_000_000 / clk_tck();
    let page = page_size();
    let mut s = Sample { pids: pids.len() as u32, ..Default::default() };
    for p in pids {
        if let Some(t) = std::fs::read_to_string(format!("/proc/{p}/stat")).ok().as_deref().and_then(parse_stat_cpu) {
            s.cpu_usec += t * tick_us;
        }
        if let Some(rss) = std::fs::read_to_string(format!("/proc/{p}/statm"))
            .ok()
            .and_then(|m| m.split_whitespace().nth(1)?.parse::<u64>().ok())
        {
            s.memory += rss * page;
        }
    }
    s
}

pub fn sample(id: &str, pid1: u32) -> Sample {
    if let Some(cg) = crate::cgroup::path_for(id) {
        if let Some((memory, limit, pids, cpu_usec)) = crate::cgroup::read_stats(&cg) {
            return Sample { cpu_usec, memory, limit, pids };
        }
    }
    sample_proc(pid1)
}

/// Remembers the previous sample per container so repeated `stats` calls
/// (a streaming `hullr stats`) compute CPU% without sleeping.
#[derive(Default)]
pub struct Tracker {
    last: Mutex<HashMap<String, (Instant, u64)>>,
}

impl Tracker {
    pub fn stats(&self, id: &str, pid1: u32, memory_limit: Option<u64>) -> Stats {
        let prev = self.last.lock().unwrap().get(id).copied().filter(|(t, _)| t.elapsed() < Duration::from_secs(10));
        let (t0, c0) = match prev {
            Some(p) => p,
            None => {
                let s = sample(id, pid1);
                let t = Instant::now();
                std::thread::sleep(Duration::from_millis(150));
                (t, s.cpu_usec)
            }
        };
        let s = sample(id, pid1);
        let now = Instant::now();
        let wall = now.duration_since(t0).as_micros().max(1) as f64;
        let cpu_percent = (s.cpu_usec.saturating_sub(c0) as f64 / wall) * 100.0;
        self.last.lock().unwrap().insert(id.to_string(), (now, s.cpu_usec));
        let (rx, tx) = std::fs::read_to_string(format!("/proc/{pid1}/net/dev"))
            .map(|t| parse_net_dev(&t))
            .unwrap_or((0, 0));
        Stats {
            cpu_percent,
            memory_bytes: s.memory,
            memory_limit: s.limit.or(memory_limit),
            pids: s.pids,
            net_rx_bytes: rx,
            net_tx_bytes: tx,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_line() {
        let l = "1234 (my (odd) cmd) S 1 1234 1234 0 -1 4194560 100 0 0 0 7 3 0 0 20 0 1 0 100 1000 50";
        assert_eq!(parse_stat_cpu(l), Some(10));
    }

    #[test]
    fn net_dev() {
        let t = "Inter-|   Receive |  Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes\n\
            lo: 100 1 0 0 0 0 0 0 100 1 0 0 0 0 0 0\n\
            eth0: 2000 10 0 0 0 0 0 0 3000 12 0 0 0 0 0 0\n";
        assert_eq!(parse_net_dev(t), (2000, 3000));
    }

    #[test]
    fn self_sample() {
        let me = std::process::id();
        let s = sample_proc(me);
        assert!(s.pids >= 1 && s.memory > 0);
    }
}
