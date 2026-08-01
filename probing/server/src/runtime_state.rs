//! Composition root for mutable server process state.

use std::fmt;
use std::sync::LazyLock;

struct ServerRuntime {
    supervisor: crate::supervisor::ServerSupervisor,
    config: crate::runtime_config::ServerRuntimeConfig,
}

pub(crate) struct RuntimeSnapshot {
    engine: crate::engine_lifecycle::EngineInitState,
    supervisor: crate::supervisor::SupervisorSnapshot,
    config: crate::runtime_config::RuntimeConfigSnapshot,
}

impl fmt::Display for RuntimeSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "engine={:?}; components=[{}]; config=[{}]",
            self.engine, self.supervisor, self.config
        )
    }
}

static SERVER_RUNTIME: LazyLock<ServerRuntime> = LazyLock::new(|| ServerRuntime {
    supervisor: crate::supervisor::ServerSupervisor::new(),
    config: crate::runtime_config::ServerRuntimeConfig::from_env(),
});

pub(crate) fn supervisor() -> &'static crate::supervisor::ServerSupervisor {
    &SERVER_RUNTIME.supervisor
}

pub(crate) fn config() -> &'static crate::runtime_config::ServerRuntimeConfig {
    &SERVER_RUNTIME.config
}

pub(crate) fn snapshot() -> RuntimeSnapshot {
    RuntimeSnapshot {
        engine: crate::engine_lifecycle::engine_init_state(),
        supervisor: SERVER_RUNTIME.supervisor.snapshot(),
        config: SERVER_RUNTIME.config.snapshot(),
    }
}

pub(crate) fn shutdown() {
    log::debug!("shutting down server runtime: {}", snapshot());
    SERVER_RUNTIME.supervisor.shutdown();
}
