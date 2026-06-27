mod collector;
mod extension;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
mod sample;
mod sampler;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod unsupported;

pub use collector::{autostart_interval_ms, start_cpu_sampling, start_cpu_sampling_from_env};
pub use extension::CpuProbeExtension;
#[cfg(target_os = "macos")]
pub use probing_core::signal::send_sigusr2_to_thread_id;
pub use sample::{ProcessSample, ThreadSample};
pub use sampler::{host_sampler, CpuHostSampler};

#[cfg(test)]
mod tests {
    use super::sampler::host_sampler;

    #[test]
    fn host_sampler_process_sample() {
        let sampler = host_sampler();
        let sample = sampler.sample_process().expect("process sample");
        assert!(sample.rss_bytes > 0 || sample.cputime_user_ns > 0);
    }

    #[test]
    fn host_sampler_thread_sample() {
        let sampler = host_sampler();
        let threads = sampler.sample_threads(8).expect("thread sample");
        // Linux/macOS should enumerate at least the main thread.
        assert!(
            !threads.is_empty(),
            "expected thread-level CPU samples from {:?}",
            sampler.platform()
        );
    }
}
