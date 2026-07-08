#![cfg_attr(test, allow(clippy::approx_constant, clippy::await_holding_lock))]

pub mod config;
pub mod core;
pub mod diagnostics;
pub mod runtime;
pub mod signal;
pub mod storage;
pub mod sync;
pub mod trace;
mod tracing;

pub use diagnostics::install_panic_hook;
pub use runtime::{
    block_on, is_python_main_thread, on_native_bridge_thread, register_python_main_thread,
    run_on_native_thread, runtime_operational, BlockOnFallback, RuntimeError, CORE_RUNTIME,
};

use self::core::Engine;
use self::core::EngineBuilder;

pub fn create_engine() -> EngineBuilder {
    Engine::builder().with_default_namespace("probe")
}

use once_cell::sync::Lazy;
use tokio::sync::RwLock;

use self::core::Result;

pub static ENGINE: Lazy<RwLock<Engine>> = Lazy::new(|| RwLock::new(Engine::default()));

pub async fn initialize_engine(builder: EngineBuilder) -> Result<()> {
    let engine = builder
        .build()
        .await
        .inspect_err(|e| log::error!("Error creating engine: {e}"))?;

    *ENGINE.write().await = engine;
    Ok(())
}
