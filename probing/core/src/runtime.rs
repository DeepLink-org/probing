use std::future::Future;
use std::sync::OnceLock;
use std::thread::{self, ThreadId};

use once_cell::sync::Lazy;

/// Shared Tokio runtime for all sync→async bridges (Python bindings, local server, etc.).
///
/// ENGINE and CONFIG_STORE must only be accessed from this runtime. Creating ad-hoc
/// runtimes (especially when Python already has an asyncio loop) can cause SIGSEGV.
pub static CORE_RUNTIME: Lazy<tokio::runtime::Runtime> = Lazy::new(|| {
    let worker_threads = std::env::var("PROBING_SERVER_WORKER_THREADS")
        .unwrap_or_else(|_| "4".to_string())
        .parse::<usize>()
        .unwrap_or(4);
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(worker_threads)
        .thread_name("probing-runtime")
        .build()
        .unwrap_or_else(|e| panic!("Failed to create probing runtime: {e}"))
});

/// Python main thread id, registered when `probing._core` loads.
static PYTHON_MAIN_THREAD: OnceLock<ThreadId> = OnceLock::new();

/// Record the Python main thread (call from `probing._core` module init).
pub fn register_python_main_thread() {
    let _ = PYTHON_MAIN_THREAD.set(thread::current().id());
}

fn is_python_main_thread() -> bool {
    PYTHON_MAIN_THREAD
        .get()
        .is_some_and(|id| thread::current().id() == *id)
}

fn is_inside_core_runtime() -> bool {
    tokio::runtime::Handle::try_current().is_ok()
}

fn spawn_block_on_thread<F, T>(future: F) -> T
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    thread::Builder::new()
        .name("probing-block-on".into())
        .spawn(move || CORE_RUNTIME.block_on(future))
        .expect("failed to spawn block_on thread")
        .join()
        .expect("block_on thread panicked")
}

/// Run synchronous Rust work that must not execute on the Python main thread once
/// extension modules such as PyArrow have initialized (macOS SIGSEGV without this).
pub fn run_on_native_thread<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
    if !is_python_main_thread() || is_inside_core_runtime() {
        return f();
    }
    // Use a fresh thread per call so nested main-thread calls cannot deadlock a
    // single-worker queue (e.g. config.set during probing import).
    thread::Builder::new()
        .name("probing-native".into())
        .spawn(f)
        .expect("failed to spawn probing-native thread")
        .join()
        .expect("probing-native thread panicked")
}

/// Run an async future on [`CORE_RUNTIME`] from a synchronous context.
pub fn block_on<F, T>(future: F) -> T
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    // Never call Runtime::block_on from a probing-runtime worker (panics).
    if is_inside_core_runtime() {
        return spawn_block_on_thread(future);
    }
    run_on_native_thread(move || CORE_RUNTIME.block_on(future))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_on_runs_on_core_runtime() {
        register_python_main_thread();
        let thread_name = block_on(async {
            thread::current()
                .name()
                .unwrap_or_default()
                .to_string()
        });
        assert!(thread_name.starts_with("probing-runtime"));
    }

    #[test]
    fn native_thread_routes_from_main() {
        register_python_main_thread();
        let ran_on = run_on_native_thread(|| {
            thread::current()
                .name()
                .unwrap_or_default()
                .to_string()
        });
        assert_eq!(ran_on, "probing-native");
    }

    #[test]
    fn block_on_from_runtime_worker_does_not_panic() {
        register_python_main_thread();
        let value = block_on(async {
            block_on(async { 40 + 2 })
        });
        assert_eq!(value, 42);
    }
}
