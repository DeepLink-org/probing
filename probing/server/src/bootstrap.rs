//! Process bootstrap orchestration for the in-process probing server.

use crate::engine::initialize_engine;
use crate::engine_lifecycle::EngineInitClaim;

/// Initialize the SQL engine and ensure the local control listener is running.
///
/// The listener still starts when engine initialization fails so `/health` and `/ready` can
/// expose the failure instead of making the process silently disappear.
pub(crate) fn start_local() {
    if initialize_engine_once() {
        crate::server::start_local_listener();
    }
}

fn initialize_engine_once() -> bool {
    match crate::engine_lifecycle::begin_engine_initialization() {
        EngineInitClaim::Ready => {
            log::debug!("probing engine already initialized");
            return true;
        }
        EngineInitClaim::InProgress => {
            log::debug!("probing engine initialization already in progress");
            return false;
        }
        EngineInitClaim::Claimed => {}
    }

    match probing_core::runtime::block_on(initialize_engine()) {
        Ok(Ok(())) => log::info!("probing engine initialized"),
        Ok(Err(error)) => crate::failure::engine_initialization_failed(format!("{error:#}")),
        Err(error) => crate::failure::engine_initialization_failed(format!(
            "probing runtime unavailable during engine initialization: {error}"
        )),
    }
    true
}
