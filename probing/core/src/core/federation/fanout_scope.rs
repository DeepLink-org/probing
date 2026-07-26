//! Request-scoped fan-out context for hierarchical cluster queries.
//!
//! Coordinator → node aggregators (local0) → on-node leaf ranks.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};

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

/// Outcome of a federated query's remote work.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FanoutStats {
    pub nodes_succeeded: usize,
    pub nodes_failed: Vec<String>,
    /// Peer partial DataFrames dropped during coordinator merge (conversion failure).
    pub peer_batches_dropped: usize,
}

/// Shareable stats sink captured by physical execution plans.
#[derive(Debug, Clone, Default)]
pub(crate) struct FanoutStatsHandle(Arc<Mutex<FanoutStats>>);

impl FanoutStatsHandle {
    fn lock(&self) -> MutexGuard<'_, FanoutStats> {
        self.0.lock().unwrap_or_else(|poisoned| {
            log::error!("fan-out stats lock poisoned; recovering inner state");
            poisoned.into_inner()
        })
    }

    pub(crate) fn reset(&self) {
        *self.lock() = FanoutStats::default();
    }

    pub(crate) fn set(&self, stats: FanoutStats) {
        *self.lock() = stats;
    }

    pub(crate) fn record_success(&self) {
        self.lock().nodes_succeeded += 1;
    }

    pub(crate) fn record_failure(&self, addr: &str) {
        self.lock().nodes_failed.push(addr.to_string());
    }

    pub(crate) fn take(&self) -> FanoutStats {
        std::mem::take(&mut *self.lock())
    }

    pub(crate) fn snapshot(&self) -> FanoutStats {
        self.lock().clone()
    }
}

#[derive(Clone)]
struct FanoutContext {
    scope: Cell<FanoutScope>,
    stats: FanoutStatsHandle,
}

impl FanoutContext {
    fn new(scope: FanoutScope) -> Self {
        Self {
            scope: Cell::new(scope),
            stats: FanoutStatsHandle::default(),
        }
    }

    fn child(&self, scope: FanoutScope) -> Self {
        Self {
            scope: Cell::new(scope),
            stats: self.stats.clone(),
        }
    }
}

tokio::task_local! {
    static FANOUT_CONTEXT: FanoutContext;
}

thread_local! {
    // Synchronous core callers and tests do not necessarily run inside a Tokio task.
    // Async server entry points install `FANOUT_CONTEXT`.
    static SYNC_FANOUT_CONTEXT: RefCell<FanoutContext> =
        RefCell::new(FanoutContext::new(FanoutScope::Auto));
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
    if FANOUT_CONTEXT
        .try_with(|context| context.scope.set(scope))
        .is_err()
    {
        SYNC_FANOUT_CONTEXT.with(|context| context.borrow().scope.set(scope));
    }
}

pub fn current_fanout_scope() -> FanoutScope {
    FANOUT_CONTEXT
        .try_with(|context| context.scope.get())
        .unwrap_or_else(|_| SYNC_FANOUT_CONTEXT.with(|context| context.borrow().scope.get()))
}

pub fn take_fanout_scope() -> FanoutScope {
    let scope = current_fanout_scope();
    set_fanout_scope(FanoutScope::Auto);
    scope
}

/// Run ``f`` with a scoped fan-out tier (sync).
pub fn with_fanout_scope<T>(scope: FanoutScope, f: impl FnOnce() -> T) -> T {
    let previous = current_fanout_scope();
    set_fanout_scope(scope);
    let out = f();
    set_fanout_scope(previous);
    out
}

/// Run a future with task-scoped fan-out state.
///
/// Nested scopes share the same stats sink while overriding only the routing
/// tier. Tokio task migration is safe because the context moves with the
/// future instead of being attached to an executor thread.
pub async fn with_fanout_scope_async<F>(scope: FanoutScope, future: F) -> F::Output
where
    F: Future,
{
    let context = FANOUT_CONTEXT
        .try_with(|parent| parent.child(scope))
        .unwrap_or_else(|_| FanoutContext::new(scope));
    FANOUT_CONTEXT.scope(context, future).await
}

pub(crate) fn current_fanout_stats_handle() -> FanoutStatsHandle {
    FANOUT_CONTEXT
        .try_with(|context| context.stats.clone())
        .unwrap_or_else(|_| SYNC_FANOUT_CONTEXT.with(|context| context.borrow().stats.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Barrier;

    #[tokio::test(flavor = "current_thread")]
    async fn async_context_isolated_across_interleaved_tasks() {
        let barrier = Arc::new(Barrier::new(2));
        let first_barrier = barrier.clone();
        let first = tokio::spawn(with_fanout_scope_async(
            FanoutScope::Coordinator,
            async move {
                let stats = current_fanout_stats_handle();
                stats.record_success();
                first_barrier.wait().await;
                tokio::task::yield_now().await;
                (current_fanout_scope(), stats.take())
            },
        ));

        let second = tokio::spawn(with_fanout_scope_async(FanoutScope::Node, async move {
            let stats = current_fanout_stats_handle();
            stats.record_failure("rank-1");
            barrier.wait().await;
            tokio::task::yield_now().await;
            (current_fanout_scope(), stats.take())
        }));

        let (first_scope, first_stats) = first.await.expect("first task");
        let (second_scope, second_stats) = second.await.expect("second task");

        assert_eq!(first_scope, FanoutScope::Coordinator);
        assert_eq!(first_stats.nodes_succeeded, 1);
        assert!(first_stats.nodes_failed.is_empty());
        assert_eq!(second_scope, FanoutScope::Node);
        assert_eq!(second_stats.nodes_succeeded, 0);
        assert_eq!(second_stats.nodes_failed, vec!["rank-1"]);
    }

    #[tokio::test]
    async fn nested_scope_shares_request_stats() {
        with_fanout_scope_async(FanoutScope::Coordinator, async {
            let stats = current_fanout_stats_handle();
            stats.record_success();
            with_fanout_scope_async(FanoutScope::Node, async {
                current_fanout_stats_handle().record_failure("rank-2");
                assert_eq!(current_fanout_scope(), FanoutScope::Node);
            })
            .await;

            assert_eq!(current_fanout_scope(), FanoutScope::Coordinator);
            let stats = stats.take();
            assert_eq!(stats.nodes_succeeded, 1);
            assert_eq!(stats.nodes_failed, vec!["rank-2"]);
        })
        .await;
    }
}
