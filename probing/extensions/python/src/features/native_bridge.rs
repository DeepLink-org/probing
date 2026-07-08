use pyo3::prelude::*;

use probing_core::{on_native_bridge_thread, run_on_native_thread};

/// Run Rust/Python bridge work off the Python main thread and Tokio workers.
///
/// When already on the ``probing-native`` bridge worker (e.g. loading a Python
/// plugin from ``execute_python_code``), avoid ``py.detach`` — an extra thread
/// plus nested GIL handoffs can SIGABRT on Linux CI (pytest-cov + torch).
pub fn with_detached_native<R: Send + probing_core::runtime::BlockOnFallback + 'static>(
    f: impl FnOnce() -> R + Send + 'static,
) -> R {
    if on_native_bridge_thread() {
        return run_on_native_thread(f);
    }
    Python::attach(|py| py.detach(|| run_on_native_thread(f)))
}
