use pyo3::prelude::*;

use probing_core::run_on_native_thread;

/// Run Rust/Python bridge work off the Python main thread and Tokio workers.
pub fn with_detached_native<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
    Python::attach(|py| py.detach(|| run_on_native_thread(f)))
}
