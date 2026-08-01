//! Central failure policy for server bootstrap and supervised components.

pub(crate) fn engine_initialization_failed(message: String) {
    log::error!("probing engine initialization failed: {message}");
    crate::engine_lifecycle::mark_engine_failed(message);
    log::debug!(
        "server runtime after engine failure: {}",
        crate::runtime_state::snapshot()
    );
}

pub(crate) fn component_failed(component: &str, error: anyhow::Error) -> String {
    let message = format!("{error:#}");
    log::error!("{component} exited: {message}");
    message
}
