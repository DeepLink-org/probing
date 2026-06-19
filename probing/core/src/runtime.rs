use std::cell::Cell;
use std::future::Future;
use std::sync::{mpsc, OnceLock};
use std::thread::{self, ThreadId};

use log;
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

/// Whether the current thread is the Python main thread registered at `_core` load.
pub fn is_python_main_thread() -> bool {
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

/// Single worker for Python↔Rust calls that must not run on the Python main thread
/// (macOS/PyArrow) or on Tokio workers (nested Python callbacks).
struct NativeBridge {
    tx: mpsc::Sender<BridgeJob>,
}

struct BridgeJob {
    func: Box<dyn FnOnce() + Send>,
    done: mpsc::Sender<()>,
}

impl NativeBridge {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel::<BridgeJob>();
        thread::Builder::new()
            .name("probing-native".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    let finished = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        (job.func)();
                    }));
                    if finished.is_err() {
                        log::error!("probing-native bridge worker panicked");
                    }
                    let _ = job.done.send(());
                }
            })
            .expect("failed to spawn probing-native bridge");
        Self { tx }
    }

    fn call<R: Send + 'static>(&self, f: impl FnOnce() -> R + Send + 'static) -> R {
        let (result_tx, result_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        self.tx
            .send(BridgeJob {
                func: Box::new(move || {
                    let r = f();
                    let _ = result_tx.send(r);
                }),
                done: done_tx,
            })
            .expect("probing-native bridge thread exited");
        done_rx
            .recv()
            .expect("probing-native bridge worker dropped completion");
        result_rx
            .recv()
            .expect("probing-native bridge worker returned no value")
    }
}

static NATIVE_BRIDGE: Lazy<NativeBridge> = Lazy::new(NativeBridge::new);

thread_local! {
    static ON_NATIVE_BRIDGE: Cell<bool> = const { Cell::new(false) };
}

fn on_native_bridge() -> bool {
    ON_NATIVE_BRIDGE.with(|v| v.get())
}

fn run_on_native_bridge<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
    if on_native_bridge() {
        return f();
    }
    NATIVE_BRIDGE.call(|| {
        ON_NATIVE_BRIDGE.with(|flag| {
            flag.set(true);
            let out = f();
            flag.set(false);
            out
        })
    })
}

fn needs_native_bridge() -> bool {
    (is_python_main_thread() && !on_native_bridge()) || is_inside_core_runtime()
}

/// Run synchronous Rust/Python bridge work off the Python main thread and Tokio workers.
pub fn run_on_native_thread<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
    if needs_native_bridge() {
        return run_on_native_bridge(f);
    }
    f()
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
    fn block_on_completes_on_current_runtime() {
        let value = block_on(async { 21 + 21 });
        assert_eq!(value, 42);
    }

    #[test]
    fn block_on_from_runtime_worker_does_not_panic() {
        let value = block_on(async { block_on(async { 40 + 2 }) });
        assert_eq!(value, 42);
    }

    #[test]
    fn native_bridge_serializes_calls() {
        let a = run_on_native_bridge(|| 1);
        let b = run_on_native_bridge(|| 2);
        assert_eq!(a + b, 3);
    }
}
