use std::io;

use super::sample::{ProcessSample, ThreadSample};

/// Cross-platform CPU host sampler for the current process only (Tier 0).
pub trait CpuHostSampler: Send + Sync {
    fn platform(&self) -> &'static str;

    fn sample_process(&self) -> io::Result<ProcessSample>;

    /// Thread samples; backends may return an empty vec when unsupported.
    fn sample_threads(&self, top_n: usize) -> io::Result<Vec<ThreadSample>>;
}

/// Platform backend for the current process.
pub fn host_sampler() -> Box<dyn CpuHostSampler> {
    #[cfg(target_os = "linux")]
    {
        return Box::new(super::linux::LinuxSampler::new());
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(super::macos::MacSampler)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        return Box::new(super::unsupported::UnsupportedSampler);
    }
}
