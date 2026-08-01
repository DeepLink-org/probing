//! Ownership and lifecycle for long-running server components.

use std::fmt;
use std::future::Future;
use std::sync::{Mutex, MutexGuard};

use tokio::sync::oneshot;
use tokio::task::JoinHandle;

#[derive(Debug, Clone, PartialEq, Eq)]
enum ComponentState {
    Stopped,
    Starting {
        generation: u64,
        key: String,
    },
    Running {
        generation: u64,
        key: String,
    },
    Failed {
        generation: u64,
        key: String,
        error: String,
    },
}

impl fmt::Display for ComponentState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stopped => formatter.write_str("stopped"),
            Self::Starting { generation, key } => {
                write!(formatter, "starting(generation={generation}, key={key})")
            }
            Self::Running { generation, key } => {
                write!(formatter, "running(generation={generation}, key={key})")
            }
            Self::Failed {
                generation,
                key,
                error,
            } => write!(
                formatter,
                "failed(generation={generation}, key={key}, error={error})"
            ),
        }
    }
}

struct ManagedTask {
    generation: u64,
    key: String,
    handle: JoinHandle<()>,
}

struct RemoteListenerSlot {
    next_generation: u64,
    active: Option<ManagedTask>,
    candidate: Option<ManagedTask>,
    state: ComponentState,
}

impl Default for RemoteListenerSlot {
    fn default() -> Self {
        Self {
            next_generation: 0,
            active: None,
            candidate: None,
            state: ComponentState::Stopped,
        }
    }
}

struct ReplaceableTaskSlot {
    next_generation: u64,
    active: Option<ManagedTask>,
    state: ComponentState,
}

#[derive(Clone, Copy)]
enum ManagedComponent {
    LocalListener,
    ReportWorker,
    TorchrunCluster,
}

impl ManagedComponent {
    fn name(self) -> &'static str {
        match self {
            Self::LocalListener => "local_listener",
            Self::ReportWorker => "report_worker",
            Self::TorchrunCluster => "torchrun_cluster",
        }
    }
}

pub(crate) struct SupervisorSnapshot {
    remote_listener: ComponentState,
    local_listener: ComponentState,
    report_worker: ComponentState,
    torchrun_cluster: ComponentState,
}

impl fmt::Display for SupervisorSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "remote_listener={}, local_listener={}, report_worker={}, torchrun_cluster={}",
            self.remote_listener, self.local_listener, self.report_worker, self.torchrun_cluster
        )
    }
}

impl Default for ReplaceableTaskSlot {
    fn default() -> Self {
        Self {
            next_generation: 0,
            active: None,
            state: ComponentState::Stopped,
        }
    }
}

pub(crate) struct ServerSupervisor {
    remote_listener: Mutex<RemoteListenerSlot>,
    local_listener: Mutex<ReplaceableTaskSlot>,
    report_worker: Mutex<ReplaceableTaskSlot>,
    torchrun_cluster: Mutex<ReplaceableTaskSlot>,
}

impl ServerSupervisor {
    pub(crate) fn new() -> Self {
        Self {
            remote_listener: Mutex::new(RemoteListenerSlot::default()),
            local_listener: Mutex::new(ReplaceableTaskSlot::default()),
            report_worker: Mutex::new(ReplaceableTaskSlot::default()),
            torchrun_cluster: Mutex::new(ReplaceableTaskSlot::default()),
        }
    }

    fn lock_remote(&self) -> MutexGuard<'_, RemoteListenerSlot> {
        self.remote_listener
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_component(&self, component: ManagedComponent) -> MutexGuard<'_, ReplaceableTaskSlot> {
        let slot = match component {
            ManagedComponent::LocalListener => &self.local_listener,
            ManagedComponent::ReportWorker => &self.report_worker,
            ManagedComponent::TorchrunCluster => &self.torchrun_cluster,
        };
        slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn snapshot(&self) -> SupervisorSnapshot {
        SupervisorSnapshot {
            remote_listener: self.lock_remote().state.clone(),
            local_listener: self
                .lock_component(ManagedComponent::LocalListener)
                .state
                .clone(),
            report_worker: self
                .lock_component(ManagedComponent::ReportWorker)
                .state
                .clone(),
            torchrun_cluster: self
                .lock_component(ManagedComponent::TorchrunCluster)
                .state
                .clone(),
        }
    }

    pub(crate) fn torchrun_cluster_active(&self) -> bool {
        let slot = self.lock_component(ManagedComponent::TorchrunCluster);
        matches!(
            &slot.state,
            ComponentState::Starting { .. } | ComponentState::Running { .. }
        )
    }

    /// Start a candidate listener. The old active listener remains alive until the candidate
    /// calls [`Self::promote_remote_listener`] after binding successfully.
    pub(crate) fn start_remote_listener<F, Fut>(&'static self, key: String, factory: F)
    where
        F: FnOnce(u64) -> Fut,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        let mut slot = self.lock_remote();
        if slot
            .active
            .as_ref()
            .is_some_and(|task| task.key == key && !task.handle.is_finished())
            || slot
                .candidate
                .as_ref()
                .is_some_and(|task| task.key == key && !task.handle.is_finished())
        {
            log::debug!("remote listener already active or starting for {key}");
            return;
        }

        if let Some(candidate) = slot.candidate.take() {
            candidate.handle.abort();
        }
        slot.next_generation = slot.next_generation.wrapping_add(1).max(1);
        let generation = slot.next_generation;
        transition(
            "remote_listener",
            &mut slot.state,
            ComponentState::Starting {
                generation,
                key: key.clone(),
            },
        );

        let (start_tx, start_rx) = oneshot::channel();
        let future = factory(generation);
        let supervisor = self;
        let handle = crate::server::SERVER_RUNTIME.spawn(async move {
            if start_rx.await.is_err() {
                return;
            }
            let result = future.await;
            supervisor.remote_listener_finished(generation, result.err());
        });
        slot.candidate = Some(ManagedTask {
            generation,
            key,
            handle,
        });
        let _ = start_tx.send(());
    }

    /// Atomically promote a bound candidate and stop the previously active listener.
    pub(crate) fn promote_remote_listener(&self, generation: u64) -> bool {
        let mut slot = self.lock_remote();
        let Some(candidate) = slot.candidate.take() else {
            return false;
        };
        if candidate.generation != generation {
            slot.candidate = Some(candidate);
            return false;
        }
        if let Some(active) = slot.active.take() {
            active.handle.abort();
        }
        transition(
            "remote_listener",
            &mut slot.state,
            ComponentState::Running {
                generation,
                key: candidate.key.clone(),
            },
        );
        slot.active = Some(candidate);
        true
    }

    fn remote_listener_finished(&self, generation: u64, error: Option<String>) {
        let mut slot = self.lock_remote();
        if slot
            .candidate
            .as_ref()
            .is_some_and(|task| task.generation == generation)
        {
            let Some(candidate) = slot.candidate.take() else {
                return;
            };
            if let Some(active) = slot.active.as_ref() {
                let next = ComponentState::Running {
                    generation: active.generation,
                    key: active.key.clone(),
                };
                transition("remote_listener", &mut slot.state, next);
            } else {
                let next = completion_state(candidate, error);
                transition("remote_listener", &mut slot.state, next);
            }
            return;
        }
        if slot
            .active
            .as_ref()
            .is_some_and(|task| task.generation == generation)
        {
            let Some(active) = slot.active.take() else {
                return;
            };
            let next = completion_state(active, error);
            transition("remote_listener", &mut slot.state, next);
        }
    }

    pub(crate) fn replace_report_worker<F, Fut>(&'static self, key: String, factory: F)
    where
        F: FnOnce(u64) -> Fut,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        self.start_component(ManagedComponent::ReportWorker, key, factory);
    }

    pub(crate) fn start_local_listener<F, Fut>(&'static self, key: String, factory: F)
    where
        F: FnOnce(u64) -> Fut,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        self.start_component(ManagedComponent::LocalListener, key, factory);
    }

    pub(crate) fn start_torchrun_cluster<F, Fut>(&'static self, key: String, factory: F)
    where
        F: FnOnce(u64) -> Fut,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        self.start_component(ManagedComponent::TorchrunCluster, key, factory);
    }

    fn start_component<F, Fut>(&'static self, component: ManagedComponent, key: String, factory: F)
    where
        F: FnOnce(u64) -> Fut,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        let mut slot = self.lock_component(component);
        if slot
            .active
            .as_ref()
            .is_some_and(|task| task.key == key && !task.handle.is_finished())
        {
            log::debug!("managed server component already running for {key}");
            return;
        }
        if let Some(active) = slot.active.take() {
            active.handle.abort();
        }
        slot.next_generation = slot.next_generation.wrapping_add(1).max(1);
        let generation = slot.next_generation;
        transition(
            component.name(),
            &mut slot.state,
            ComponentState::Starting {
                generation,
                key: key.clone(),
            },
        );

        let (start_tx, start_rx) = oneshot::channel();
        let future = factory(generation);
        let supervisor = self;
        let handle = crate::server::SERVER_RUNTIME.spawn(async move {
            if start_rx.await.is_err() {
                return;
            }
            supervisor.mark_component_running(component, generation);
            let result = future.await;
            supervisor.component_finished(component, generation, result.err());
        });
        slot.active = Some(ManagedTask {
            generation,
            key,
            handle,
        });
        let _ = start_tx.send(());
    }

    pub(crate) fn stop_report_worker(&self) {
        self.stop_component(ManagedComponent::ReportWorker);
    }

    pub(crate) fn shutdown(&self) {
        {
            let mut slot = self.lock_remote();
            slot.next_generation = slot.next_generation.wrapping_add(1).max(1);
            if let Some(candidate) = slot.candidate.take() {
                candidate.handle.abort();
            }
            if let Some(active) = slot.active.take() {
                active.handle.abort();
            }
            transition("remote_listener", &mut slot.state, ComponentState::Stopped);
        }
        self.stop_component(ManagedComponent::LocalListener);
        self.stop_component(ManagedComponent::ReportWorker);
        self.stop_component(ManagedComponent::TorchrunCluster);
    }

    fn stop_component(&self, component: ManagedComponent) {
        let mut slot = self.lock_component(component);
        slot.next_generation = slot.next_generation.wrapping_add(1).max(1);
        if let Some(active) = slot.active.take() {
            active.handle.abort();
        }
        transition(component.name(), &mut slot.state, ComponentState::Stopped);
    }

    fn mark_component_running(&self, component: ManagedComponent, generation: u64) {
        let mut slot = self.lock_component(component);
        let key = slot
            .active
            .as_ref()
            .filter(|task| task.generation == generation)
            .map(|task| task.key.clone());
        if let Some(key) = key {
            transition(
                component.name(),
                &mut slot.state,
                ComponentState::Running { generation, key },
            );
        }
    }

    fn component_finished(
        &self,
        component: ManagedComponent,
        generation: u64,
        error: Option<String>,
    ) {
        let mut slot = self.lock_component(component);
        if slot
            .active
            .as_ref()
            .is_some_and(|task| task.generation == generation)
        {
            let Some(active) = slot.active.take() else {
                return;
            };
            let next = completion_state(active, error);
            transition(component.name(), &mut slot.state, next);
        }
    }
}

fn transition(component: &str, state: &mut ComponentState, next: ComponentState) {
    if *state != next {
        log::debug!("server component {component}: {state} -> {next}");
        *state = next;
    }
}

fn completion_state(task: ManagedTask, error: Option<String>) -> ComponentState {
    match error {
        Some(error) => ComponentState::Failed {
            generation: task.generation,
            key: task.key,
            error,
        },
        None => ComponentState::Stopped,
    }
}
