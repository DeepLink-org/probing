use std::io;

use super::sample::{ProcessSample, ThreadSample};
use super::sampler::CpuHostSampler;

pub struct UnsupportedSampler;

impl CpuHostSampler for UnsupportedSampler {
    fn platform(&self) -> &'static str {
        "unsupported"
    }

    fn sample_process(&self) -> io::Result<ProcessSample> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "CPU host sampling is not supported on this platform",
        ))
    }

    fn sample_threads(&self, _top_n: usize) -> io::Result<Vec<ThreadSample>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "CPU host sampling is not supported on this platform",
        ))
    }
}
