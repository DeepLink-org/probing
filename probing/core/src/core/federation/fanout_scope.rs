//! Request-scoped fan-out tier for hierarchical cluster queries.
//!
//! Coordinator → node aggregators (local0) → on-node leaf ranks.

use std::cell::Cell;

thread_local! {
    static FANOUT_SCOPE: Cell<FanoutScope> = const { Cell::new(FanoutScope::Auto) };
}

/// How remote peers are selected for federated / cluster fan-out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FanoutScope {
    #[default]
    Auto,
    /// Legacy: every alive peer except self.
    Flat,
    /// Global coordinator: one endpoint per node (``local_rank == 0`` / ``group_rank``).
    Coordinator,
    /// Node aggregator: sibling leaf ranks on the same ``group_rank``.
    Node,
    /// Local process only — no remote fan-out.
    Local,
}

impl FanoutScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Flat => "flat",
            Self::Coordinator => "coordinator",
            Self::Node => "node",
            Self::Local => "local",
        }
    }
}

/// Whether hierarchical fan-out is enabled (default on).
pub fn hierarchical_fanout_enabled() -> bool {
    match std::env::var("PROBING_CLUSTER_FANOUT_HIERARCHICAL") {
        Ok(val) => {
            let lower = val.trim().to_ascii_lowercase();
            !matches!(lower.as_str(), "0" | "false" | "no" | "off")
        }
        Err(_) => true,
    }
}

pub fn env_i32(name: &str) -> Option<i32> {
    std::env::var(name).ok().and_then(|v| v.trim().parse().ok())
}

pub fn local_rank_from_env() -> Option<i32> {
    env_i32("LOCAL_RANK")
}

pub fn is_local0_from_env() -> bool {
    local_rank_from_env().unwrap_or(0) == 0
}

pub fn resolve_fanout_scope(scope: FanoutScope) -> FanoutScope {
    match scope {
        FanoutScope::Auto => {
            if hierarchical_fanout_enabled() && is_local0_from_env() {
                FanoutScope::Coordinator
            } else if hierarchical_fanout_enabled() {
                FanoutScope::Local
            } else {
                FanoutScope::Flat
            }
        }
        other => other,
    }
}

pub fn set_fanout_scope(scope: FanoutScope) {
    FANOUT_SCOPE.set(scope);
}

pub fn current_fanout_scope() -> FanoutScope {
    FANOUT_SCOPE.get()
}

pub fn take_fanout_scope() -> FanoutScope {
    let scope = FANOUT_SCOPE.get();
    FANOUT_SCOPE.set(FanoutScope::Auto);
    scope
}

/// Run ``f`` with a scoped fan-out tier (sync).
pub fn with_fanout_scope<T>(scope: FanoutScope, f: impl FnOnce() -> T) -> T {
    let previous = FANOUT_SCOPE.get();
    FANOUT_SCOPE.set(scope);
    let out = f();
    FANOUT_SCOPE.set(previous);
    out
}
