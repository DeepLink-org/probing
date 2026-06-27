use std::cell::Cell;
use std::future::Future;
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread::{self, ThreadId};

use log;
use once_cell::sync::Lazy;

fn build_core_runtime() -> tokio::runtime::Runtime {
    let worker_threads = std::env::var("PROBING_SERVER_WORKER_THREADS")
        .unwrap_or_else(|_| "4".to_string())
        .parse::<usize>()
        .unwrap_or(4);
    match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(worker_threads)
        .thread_name("probing-runtime")
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            log::error!("Failed to create probing multi-thread runtime: {e}; trying current-thread fallback");
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .thread_name("probing-runtime")
                .build()
                .unwrap_or_else(|e2| {
                    log::error!(
                        "Failed to create probing fallback runtime: {e2}; using minimal runtime"
                    );
                    tokio::runtime::Builder::new_current_thread()
                        .build()
                        .expect("unable to create minimal tokio runtime")
                })
        }
    }
}

/// Shared Tokio runtime for all sync→async bridges (Python bindings, local server, etc.).
///
/// ENGINE and CONFIG_STORE must only be accessed from this runtime. Creating ad-hoc
/// runtimes (especially when Python already has an asyncio loop) can cause SIGSEGV.
pub static CORE_RUNTIME: Lazy<tokio::runtime::Runtime> = Lazy::new(build_core_runtime);

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

fn block_on_ephemeral<F, T>(future: F) -> T
where
    F: Future<Output = T>,
{
    match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt.block_on(future),
        Err(e) => {
            log::error!("failed to create ephemeral block_on runtime: {e}; using futures executor");
            futures::executor::block_on(future)
        }
    }
}

fn recover_block_on_from_cell<F, T>(future_cell: &Arc<Mutex<Option<F>>>) -> T
where
    F: Future<Output = T>,
{
    let fut = future_cell
        .lock()
        .ok()
        .and_then(|mut guard| guard.take())
        .expect("block_on future missing");
    block_on_ephemeral(fut)
}

fn spawn_block_on_thread<F, T>(future: F) -> T
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let future_cell = Arc::new(Mutex::new(Some(future)));
    let worker_cell = Arc::clone(&future_cell);
    match thread::Builder::new()
        .name("probing-block-on".into())
        .spawn(move || {
            let fut = worker_cell
                .lock()
                .ok()
                .and_then(|mut guard| guard.take())
                .expect("block_on future missing");
            CORE_RUNTIME.block_on(fut)
        }) {
        Ok(handle) => match handle.join() {
            Ok(v) => v,
            Err(_) => {
                log::error!("block_on thread panicked; using ephemeral runtime");
                recover_block_on_from_cell(&future_cell)
            }
        },
        Err(e) => {
            log::error!("failed to spawn block_on thread: {e}; using ephemeral runtime");
            recover_block_on_from_cell(&future_cell)
        }
    }
}

/// Single worker for Python↔Rust calls that must not run on the Python main thread
/// (macOS/PyArrow) or on Tokio workers (nested Python callbacks).
struct NativeBridge {
    tx: Option<mpsc::Sender<BridgeJob>>,
}

struct BridgeJob {
    func: Box<dyn FnOnce() + Send>,
    done: mpsc::Sender<()>,
}

impl NativeBridge {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel::<BridgeJob>();
        match thread::Builder::new()
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
            }) {
            Ok(_) => Self { tx: Some(tx) },
            Err(e) => {
                log::error!("failed to spawn probing-native bridge: {e}; using direct calls");
                Self { tx: None }
            }
        }
    }

    fn call<R: Send + 'static>(&self, f: impl FnOnce() -> R + Send + 'static) -> R {
        let Some(tx) = &self.tx else {
            return f();
        };
        let (result_tx, result_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let func_cell = Arc::new(Mutex::new(Some(f)));
        let worker_cell = Arc::clone(&func_cell);
        if tx
            .send(BridgeJob {
                func: Box::new(move || {
                    let r = worker_cell
                        .lock()
                        .ok()
                        .and_then(|mut guard| guard.take())
                        .expect("bridge func missing")();
                    let _ = result_tx.send(r);
                }),
                done: done_tx,
            })
            .is_err()
        {
            log::error!("probing-native bridge thread exited; using direct call");
            return func_cell
                .lock()
                .ok()
                .and_then(|mut guard| guard.take())
                .expect("bridge func missing")();
        }
        if done_rx.recv().is_err() {
            log::error!("probing-native bridge worker dropped completion");
        }
        match result_rx.recv() {
            Ok(r) => r,
            Err(_) => {
                log::error!("probing-native bridge worker returned no value; using direct call");
                func_cell
                    .lock()
                    .ok()
                    .and_then(|mut guard| guard.take())
                    .expect("bridge func missing")()
            }
        }
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
