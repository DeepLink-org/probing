use std::io;

use procfs::process::Process;
use procfs::WithCurrentSystemInfo;

use super::sample::{ProcessSample, ThreadSample};
use super::sampler::CpuHostSampler;

pub struct LinuxSampler {
    clk_tck: u64,
}

#[derive(Debug, Default, Clone, Copy)]
struct MemoryBreakdown {
    uss_bytes: u64,
    pss_bytes: u64,
    anon_bytes: u64,
    locked_bytes: u64,
}

/// Parse the single-file summary interface.
///
/// `smaps_rollup` is much cheaper than walking every VMA in `smaps` and
/// exposes the private/shared split needed to distinguish process-private
/// memory from Ray object-store mappings.
fn read_smaps_rollup() -> MemoryBreakdown {
    let Ok(text) = std::fs::read_to_string("/proc/self/smaps_rollup") else {
        return MemoryBreakdown::default();
    };
    let mut out = MemoryBreakdown::default();
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let Some(kb) = rest
            .split_whitespace()
            .next()
            .and_then(|value| value.parse::<u64>().ok())
        else {
            continue;
        };
        let bytes = kb.saturating_mul(1024);
        match key {
            "Pss" => out.pss_bytes = bytes,
            "Private_Clean" | "Private_Dirty" | "Private_Hugetlb" => {
                out.uss_bytes = out.uss_bytes.saturating_add(bytes);
            }
            "Anonymous" => out.anon_bytes = bytes,
            "Locked" => out.locked_bytes = bytes,
            _ => {}
        }
    }
    out
}

impl LinuxSampler {
    pub fn new() -> Self {
        let clk_tck = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        Self {
            clk_tck: if clk_tck > 0 { clk_tck as u64 } else { 100 },
        }
    }

    fn ticks_to_ns(&self, ticks: u64) -> u64 {
        ticks.saturating_mul(1_000_000_000) / self.clk_tck
    }
}

impl CpuHostSampler for LinuxSampler {
    fn platform(&self) -> &'static str {
        "linux"
    }

    fn sample_process(&self) -> io::Result<ProcessSample> {
        let proc = Process::myself().map_err(io::Error::other)?;
        let stat = proc.stat().map_err(io::Error::other)?;
        let (vol_ctxt, invol_ctxt) = proc
            .status()
            .ok()
            .map(|status| {
                (
                    status.voluntary_ctxt_switches.unwrap_or(0),
                    status.nonvoluntary_ctxt_switches.unwrap_or(0),
                )
            })
            .unwrap_or((0, 0));
        let thread_count = proc.tasks().map(|tasks| tasks.count() as u32).unwrap_or(0);
        let memory = read_smaps_rollup();
        let rss_bytes = stat.rss_bytes().get();

        Ok(ProcessSample {
            pid: stat.pid,
            comm: stat.comm,
            cputime_user_ns: self.ticks_to_ns(stat.utime),
            cputime_sys_ns: self.ticks_to_ns(stat.stime),
            rss_bytes,
            uss_bytes: memory.uss_bytes,
            pss_bytes: memory.pss_bytes,
            anon_bytes: memory.anon_bytes,
            locked_bytes: memory.locked_bytes,
            thread_count,
            vol_ctxt,
            invol_ctxt,
        })
    }

    fn sample_threads(&self, top_n: usize) -> io::Result<Vec<ThreadSample>> {
        if top_n == 0 {
            return Ok(Vec::new());
        }
        let proc = Process::myself().map_err(io::Error::other)?;
        let mut threads = Vec::new();

        for task in proc.tasks().map_err(io::Error::other)? {
            let task = task.map_err(io::Error::other)?;
            let stat = task.stat().map_err(io::Error::other)?;
            let wchan = std::fs::read_to_string(format!("/proc/self/task/{}/wchan", stat.pid))
                .map(|s| s.trim().to_string())
                .ok()
                .filter(|s| !s.is_empty());

            threads.push(ThreadSample {
                tid: stat.pid,
                comm: stat.comm,
                state: Some(stat.state.to_string()),
                wchan,
                cputime_user_ns: self.ticks_to_ns(stat.utime),
                cputime_sys_ns: self.ticks_to_ns(stat.stime),
            });
        }

        if top_n > 0 && threads.len() > top_n {
            threads.sort_by_key(|t| std::cmp::Reverse(t.total_cputime_ns()));
            threads.truncate(top_n);
        }

        Ok(threads)
    }
}
