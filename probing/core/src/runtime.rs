use std::cell::Cell;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread::{self, ThreadId};

use log;
use once_cell::sync::Lazy;
use thiserror::Error;

/// Async bridge failure — probing continues but callers should treat results as unavailable.
#[derive(Debug, Clone, Error)]
pub enum RuntimeError {
    #[error("probing runtime unavailable")]
    Unavailable,
    #[error("probing runtime internal error: {0}")]
    Internal(String),
    #[error("probing runtime panicked")]
    Panicked,
}

impl From<RuntimeError> for datafusion::error::DataFusionError {
    fn from(e: RuntimeError) -> Self {
        datafusion::error::DataFusionError::External(Box::new(e))
    }
}

/// Fallback value when the native bridge cannot recover a stored closure (internal only).
pub trait BlockOnFallback: Send + 'static {
    fn on_block_on_failure(err: RuntimeError) -> Self;
}

impl BlockOnFallback for () {
    fn on_block_on_failure(_: RuntimeError) -> Self {}
}

impl<T, E> BlockOnFallback for Result<T, E>
where
    T: Send + 'static,
    E: From<RuntimeError> + Send + 'static,
{
    fn on_block_on_failure(err: RuntimeError) -> Self {
        Err(err.into())
    }
}

impl<T: Send + 'static> BlockOnFallback for Result<T, String> {
    fn on_block_on_failure(err: RuntimeError) -> Self {
        Err(err.to_string())
    }
}

#[cfg(feature = "python-bridge")]
impl<T: Send + 'static> BlockOnFallback for Result<T, pyo3::PyErr> {
    fn on_block_on_failure(err: RuntimeError) -> Self {
        Err(pyo3::exceptions::PyRuntimeError::new_err(err.to_string()))
    }
}

fn try_build_core_runtime() -> Option<tokio::runtime::Runtime> {
    let worker_threads = std::env::var("PROBING_SERVER_WORKER_THREADS")
        .unwrap_or_else(|_| "4".to_string())
        .parse::<usize>()
        .unwrap_or(4);

    if let Ok(rt) = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(worker_threads)
        .thread_name("probing-runtime")
        .build()
    {
        return Some(rt);
    }

    log::error!("Failed to create probing multi-thread runtime; trying current-thread fallback");

    match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .thread_name("probing-runtime")
        .build()
    {
        Ok(rt) => Some(rt),
        Err(e) => {
            log::error!(
                "Failed to create probing current-thread runtime: {e}; \
                 async bridge will use ephemeral executors only"
            );
            None
        }
    }
}

/// Shared Tokio runtime for sync→async bridges (Python bindings, local server, etc.).
pub struct CoreRuntime {
    inner: Option<tokio::runtime::Runtime>,
    degraded: AtomicBool,
}

static FALLBACK_RUNTIME: OnceLock<Option<&'static tokio::runtime::Runtime>> = OnceLock::new();

/// Last-resort runtime for the server-side `block_on`/`spawn` methods, which —
/// unlike the free [`block_on`] function — cannot return a `Result`. Tries a
/// bounded number of times rather than spinning forever, so a catastrophic
/// environment fails loudly instead of hanging a thread.
fn build_emergency_runtime() -> Option<tokio::runtime::Runtime> {
    const MAX_ATTEMPTS: u32 = 8;
    for attempt in 1..=MAX_ATTEMPTS {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .thread_name("probing-runtime-fallback")
            .build()
        {
            Ok(rt) => return Some(rt),
            Err(e) => {
                log::error!(
                    "probing: emergency runtime creation failed (attempt {attempt}/{MAX_ATTEMPTS}): {e}"
                );
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
    log::error!(
        "probing: unable to create any tokio runtime after {MAX_ATTEMPTS} attempts; \
         async bridge will use ephemeral executors"
    );
    None
}

fn try_ephemeral_runtime() -> Option<&'static tokio::runtime::Runtime> {
    static EPHEMERAL: OnceLock<Option<tokio::runtime::Runtime>> = OnceLock::new();
    EPHEMERAL
        .get_or_init(|| {
            match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .thread_name("probing-runtime-ephemeral")
                .build()
            {
                Ok(rt) => Some(rt),
                Err(e) => {
                    log::error!("probing: ephemeral runtime build failed: {e}; retrying minimal");
                    tokio::runtime::Builder::new_current_thread()
                        .build()
                        .map(Some)
                        .unwrap_or_else(|e2| {
                            log::error!("probing: minimal ephemeral runtime build failed: {e2}");
                            None
                        })
                }
            }
        })
        .as_ref()
}

fn fallback_runtime() -> Option<&'static tokio::runtime::Runtime> {
    FALLBACK_RUNTIME
        .get_or_init(|| build_emergency_runtime().map(|rt| Box::leak(Box::new(rt)) as &'static _))
        .as_ref()
        .copied()
}

impl CoreRuntime {
    fn new() -> Self {
        let inner = try_build_core_runtime();
        let degraded = inner.is_none();
        if degraded {
            log::error!(
                "probing: no tokio runtime available; marking async bridge degraded \
                 (queries and config may return empty/error results)"
            );
        }
        Self {
            inner,
            degraded: AtomicBool::new(degraded),
        }
    }

    fn resolve_runtime(&self) -> Option<&tokio::runtime::Runtime> {
        if let Some(rt) = &self.inner {
            return Some(rt);
        }
        self.mark_degraded();
        fallback_runtime().or_else(try_ephemeral_runtime)
    }

    fn ensure_runtime(&self) -> Option<&tokio::runtime::Runtime> {
        self.resolve_runtime()
    }

    /// Whether the shared runtime is healthy enough for probing async work.
    pub fn is_operational(&self) -> bool {
        !self.degraded.load(Ordering::Relaxed)
    }

    pub fn mark_degraded(&self) {
        if !self.degraded.swap(true, Ordering::Relaxed) {
            log::error!(
                "probing: runtime marked degraded; async/query features may return \
                 empty or error results until process restart"
            );
        }
    }

    pub fn spawn<F>(&self, future: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        match self.ensure_runtime() {
            Some(rt) => rt.spawn(future),
            None => {
                self.mark_degraded();
                log::error!("probing: no tokio runtime for spawn; creating per-call ephemeral");
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt.spawn(future),
                    Err(e) => {
                        log::error!("probing: per-call spawn runtime build failed: {e}");
                        try_ephemeral_runtime()
                            .expect("probing: no tokio runtime for spawn")
                            .spawn(future)
                    }
                }
            }
        }
    }

    pub fn handle(&self) -> Option<tokio::runtime::Handle> {
        self.ensure_runtime().map(|rt| rt.handle().clone())
    }

    /// Run a future on this runtime; returns `Err` when no executor is available.
    pub fn try_block_on<F, T>(&self, future: F) -> Result<T, RuntimeError>
    where
        F: Future<Output = T>,
    {
        if let Some(rt) = &self.inner {
            return Ok(rt.block_on(future));
        }
        if let Some(rt) = fallback_runtime() {
            self.mark_degraded();
            return Ok(rt.block_on(future));
        }
        if let Some(rt) = try_ephemeral_runtime() {
            self.mark_degraded();
            log::error!(
                "probing: no core/fallback runtime for try_block_on; using static ephemeral"
            );
            return Ok(rt.block_on(future));
        }
        self.mark_degraded();
        log::error!(
            "probing: no tokio runtime for try_block_on; using per-call ephemeral executor"
        );
        block_on_ephemeral(future)
    }

    /// Prefer [`try_block_on`] when the caller can surface bridge failures.
    pub fn block_on<F, T>(&self, future: F) -> T
    where
        F: Future<Output = T>,
    {
        if let Some(rt) = &self.inner {
            return rt.block_on(future);
        }
        if let Some(rt) = fallback_runtime() {
            self.mark_degraded();
            return rt.block_on(future);
        }
        if let Some(rt) = try_ephemeral_runtime() {
            self.mark_degraded();
            return rt.block_on(future);
        }
        self.mark_degraded();
        log::error!("probing: CoreRuntime::block_on using per-call ephemeral executor");
        block_on_ephemeral(future).unwrap_or_else(|err| {
            log::error!("probing: CoreRuntime::block_on failed: {err}");
            panic!("probing: async bridge unavailable: {err}");
        })
    }
}

/// Shared Tokio runtime for all sync→async bridges (Python bindings, local server, etc.).
///
/// ENGINE and CONFIG_STORE must only be accessed from this runtime. Creating ad-hoc
/// runtimes (especially when Python already has an asyncio loop) can cause SIGSEGV.
pub static CORE_RUNTIME: Lazy<CoreRuntime> = Lazy::new(CoreRuntime::new);

/// Whether probing's async bridge is still operational.
pub fn runtime_operational() -> bool {
    CORE_RUNTIME.is_operational()
}

static PYTHON_MAIN_THREAD: OnceLock<ThreadId> = OnceLock::new();

pub fn register_python_main_thread() {
    let _ = PYTHON_MAIN_THREAD.set(thread::current().id());
}

pub fn is_python_main_thread() -> bool {
    PYTHON_MAIN_THREAD
        .get()
        .is_some_and(|id| thread::current().id() == *id)
}

fn is_inside_core_runtime() -> bool {
    tokio::runtime::Handle::try_current().is_ok()
}

fn take_from_mutex_cell<T>(cell: &Mutex<Option<T>>, context: &str) -> Option<T> {
    match cell.lock() {
        Ok(mut guard) => guard.take(),
        Err(poison) => {
            log::warn!("probing {context}: mutex poisoned; recovering stored value if any");
            poison.into_inner().take()
        }
    }
}

fn block_on_ephemeral<F, T>(future: F) -> Result<T, RuntimeError>
where
    F: Future<Output = T>,
{
    match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => Ok(rt.block_on(future)),
        Err(e) => block_on_failed(&format!("ephemeral runtime build failed: {e}")),
    }
}

fn block_on_failed<T>(context: &str) -> Result<T, RuntimeError> {
    log::error!("probing block_on: {context}; async bridge degraded");
    CORE_RUNTIME.mark_degraded();
    Err(RuntimeError::Internal(context.to_string()))
}

fn recover_block_on_from_cell<F, T>(future_cell: &Arc<Mutex<Option<F>>>) -> Result<T, RuntimeError>
where
    F: Future<Output = T>,
{
    match take_from_mutex_cell(future_cell, "block_on recovery") {
        Some(fut) => block_on_ephemeral(fut),
        None => block_on_failed("future missing during block_on recovery"),
    }
}

fn spawn_block_on_thread<F, T>(future: F) -> Result<T, RuntimeError>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = mpsc::sync_channel::<T>(1);
    let future_cell = Arc::new(Mutex::new(Some(future)));
    let worker_cell = Arc::clone(&future_cell);

    let worker = move || {
        let Some(fut) = take_from_mutex_cell(&worker_cell, "block_on worker") else {
            CORE_RUNTIME.mark_degraded();
            log::error!("probing block_on worker: future missing from cell");
            return;
        };
        let out = match CORE_RUNTIME.try_block_on(fut) {
            Ok(v) => v,
            Err(e) => {
                log::error!("probing block_on worker: runtime unavailable: {e}");
                return;
            }
        };
        let _ = tx.send(out);
    };

    match thread::Builder::new()
        .name("probing-block-on".into())
        .spawn(worker)
    {
        Ok(handle) => match handle.join() {
            Ok(()) => match rx.recv() {
                Ok(v) => Ok(v),
                Err(_) => recover_block_on_from_cell(&future_cell),
            },
            Err(_) => {
                log::error!("block_on thread panicked; attempting recovery");
                recover_block_on_from_cell(&future_cell)
            }
        },
        Err(e) => {
            log::error!("failed to spawn block_on thread: {e}; using ephemeral runtime");
            recover_block_on_from_cell(&future_cell)
        }
    }
}

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
                        CORE_RUNTIME.mark_degraded();
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

    fn call<R: Send + BlockOnFallback + 'static>(
        &self,
        f: impl FnOnce() -> R + Send + 'static,
    ) -> R {
        let Some(tx) = &self.tx else {
            return f();
        };
        let (result_tx, result_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let func_cell = Arc::new(Mutex::new(Some(f)));
        let worker_cell = Arc::clone(&func_cell);

        let run_direct = |context: &str| -> R {
            log::error!("probing-native bridge: {context}; using direct call");
            CORE_RUNTIME.mark_degraded();
            match take_from_mutex_cell(&func_cell, "native bridge direct") {
                Some(func) => func(),
                None => R::on_block_on_failure(RuntimeError::Internal(context.to_string())),
            }
        };

        if tx
            .send(BridgeJob {
                func: Box::new(move || {
                    let out = match take_from_mutex_cell(&worker_cell, "native bridge worker") {
                        Some(func) => func(),
                        None => {
                            log::error!("probing-native bridge worker: func missing from cell");
                            CORE_RUNTIME.mark_degraded();
                            return;
                        }
                    };
                    let _ = result_tx.send(out);
                }),
                done: done_tx,
            })
            .is_err()
        {
            return run_direct("bridge thread exited before job was queued");
        }
        if done_rx.recv().is_err() {
            log::error!("probing-native bridge worker dropped completion");
        }
        match result_rx.recv() {
            Ok(r) => r,
            Err(_) => run_direct("bridge worker returned no value"),
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

/// True when the current thread is the dedicated ``probing-native`` bridge worker.
pub fn on_native_bridge_thread() -> bool {
    on_native_bridge()
}

fn run_on_native_bridge<R: Send + BlockOnFallback + 'static>(
    f: impl FnOnce() -> R + Send + 'static,
) -> R {
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

pub fn run_on_native_thread<R: Send + BlockOnFallback + 'static>(
    f: impl FnOnce() -> R + Send + 'static,
) -> R {
    if needs_native_bridge() {
        return run_on_native_bridge(f);
    }
    f()
}

/// Run an async future on [`CORE_RUNTIME`] from a synchronous context.
///
/// Returns `Err(RuntimeError)` when the async bridge cannot run the future
/// (degraded runtime, panic, …). Callers must decide how to surface that —
/// the bridge never fabricates a "successful-looking" empty/default value,
/// which for a diagnostics tool would silently turn a failure into "no data".
pub fn block_on<F, T>(future: F) -> Result<T, RuntimeError>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    if is_inside_core_runtime() {
        return spawn_block_on_thread(future);
    }
    run_on_native_thread(move || {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            CORE_RUNTIME.try_block_on(future)
        })) {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(e),
            Err(_) => {
                log::error!("probing block_on panicked on native thread");
                CORE_RUNTIME.mark_degraded();
                Err(RuntimeError::Panicked)
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_block_on_completes_on_current_runtime() {
        let value = CORE_RUNTIME
            .try_block_on(async { 21 + 21 })
            .expect("runtime available in tests");
        assert_eq!(value, 42);
    }

    #[test]
    fn block_on_completes_on_current_runtime() {
        let value = block_on(async { 21 + 21 }).expect("runtime available in tests");
        assert_eq!(value, 42);
    }

    #[test]
    fn block_on_from_runtime_worker_does_not_panic() {
        let value = block_on(async { block_on(async { 40 + 2 }) })
            .expect("outer bridge")
            .expect("inner bridge");
        assert_eq!(value, 42);
    }

    #[test]
    fn native_bridge_serializes_calls() {
        run_on_native_bridge(|| ());
        run_on_native_bridge(|| ());
    }

    #[test]
    fn core_runtime_starts_operational_in_tests() {
        assert!(runtime_operational());
    }

    #[test]
    fn block_on_fallback_result_is_err_not_ok() {
        let out: Result<i32, RuntimeError> = Result::on_block_on_failure(RuntimeError::Unavailable);
        assert!(matches!(out, Err(RuntimeError::Unavailable)));
    }

    #[test]
    fn block_on_fallback_never_masks_runtime_error_in_display() {
        let err = RuntimeError::Internal("bridge broken".into());
        let wrapped: Result<(), RuntimeError> = Result::on_block_on_failure(err);
        assert!(wrapped.unwrap_err().to_string().contains("bridge broken"));
    }
}
