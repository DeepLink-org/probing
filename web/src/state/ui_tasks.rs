//! Global browser-side task queue with cancellation tokens and session groups.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use dioxus::prelude::*;

const MAX_TASKS: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiTaskKind {
    Agent,
    Snapshot,
    Skill,
    #[allow(dead_code)] // reserved for Command Panel / SQL tasks
    Query,
}

impl UiTaskKind {
    pub fn label(self) -> &'static str {
        match self {
            UiTaskKind::Agent => "Agent",
            UiTaskKind::Snapshot => "Snapshot",
            UiTaskKind::Skill => "Skill",
            UiTaskKind::Query => "Query",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiTaskStatus {
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UiTask {
    pub id: u64,
    pub kind: UiTaskKind,
    pub label: String,
    pub detail: Option<String>,
    pub status: UiTaskStatus,
    pub group_id: Option<u64>,
    pub started_ms: u64,
    pub finished_ms: Option<u64>,
    pub error: Option<String>,
}

impl UiTask {
    pub fn is_running(&self) -> bool {
        self.status == UiTaskStatus::Running
    }

    pub fn elapsed_ms(&self, now_ms: u64) -> u64 {
        let end = self.finished_ms.unwrap_or(now_ms);
        end.saturating_sub(self.started_ms)
    }

    pub fn elapsed_label(&self, now_ms: u64) -> String {
        let ms = self.elapsed_ms(now_ms);
        if ms < 1_000 {
            format!("{ms}ms")
        } else {
            format!("{:.1}s", ms as f64 / 1_000.0)
        }
    }
}

pub(crate) struct UiTaskState {
    next_id: u64,
    next_group_id: u64,
    tasks: Vec<UiTask>,
    /// Per-task cancel flag (checked cooperatively by async work).
    cancel_tokens: HashMap<u64, Arc<AtomicBool>>,
    /// Session-wide cancel flag shared by all tasks in a group.
    group_cancel: HashMap<u64, Arc<AtomicBool>>,
    active_agent_group: Option<u64>,
    /// Latest in-flight page snapshot task (superseded on route change).
    snapshot_task_id: Option<u64>,
}

impl Default for UiTaskState {
    fn default() -> Self {
        Self {
            next_id: 1,
            next_group_id: 1,
            tasks: Vec::new(),
            cancel_tokens: HashMap::new(),
            group_cancel: HashMap::new(),
            active_agent_group: None,
            snapshot_task_id: None,
        }
    }
}

pub static UI_TASKS: GlobalSignal<UiTaskState> = Signal::global(UiTaskState::default);
/// Bumped every 500ms while any task is running (drives elapsed-time UI).
pub static UI_TASK_TICK: GlobalSignal<u32> = Signal::global(|| 0);

fn now_ms() -> u64 {
    js_sys::Date::now() as u64
}

fn trim_tasks(state: &mut UiTaskState) {
    if state.tasks.len() <= MAX_TASKS {
        return;
    }
    let mut running: Vec<UiTask> = state
        .tasks
        .iter()
        .filter(|t| t.is_running())
        .cloned()
        .collect();
    let mut finished: Vec<UiTask> = state
        .tasks
        .iter()
        .filter(|t| !t.is_running())
        .cloned()
        .collect();
    finished.sort_by_key(|t| t.finished_ms.unwrap_or(t.started_ms));
    finished.reverse();
    let keep_finished = MAX_TASKS.saturating_sub(running.len());
    finished.truncate(keep_finished);
    running.append(&mut finished);
    state.tasks = running;
}

fn settle_task(id: u64, status: UiTaskStatus, error: Option<String>) {
    let mut state = UI_TASKS.write();
    if let Some(task) = state
        .tasks
        .iter_mut()
        .find(|t| t.id == id && t.is_running())
    {
        task.status = status;
        task.finished_ms = Some(now_ms());
        task.error = error;
    }
    state.cancel_tokens.remove(&id);
}

/// Handle for a running UI task; check [`UiTaskHandle::is_cancelled`] between await points.
pub struct UiTaskHandle {
    id: u64,
    task_cancel: Arc<AtomicBool>,
    group_cancel: Option<Arc<AtomicBool>>,
    settled: bool,
}

impl UiTaskHandle {
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn is_cancelled(&self) -> bool {
        self.task_cancel.load(Ordering::Relaxed)
            || self
                .group_cancel
                .as_ref()
                .is_some_and(|t| t.load(Ordering::Relaxed))
    }

    pub fn finish(mut self) {
        if self.settled {
            return;
        }
        self.settled = true;
        if self.is_cancelled() {
            settle_task(self.id, UiTaskStatus::Cancelled, None);
        } else {
            settle_task(self.id, UiTaskStatus::Done, None);
        }
    }

    pub fn fail(mut self, error: impl Into<String>) {
        if self.settled {
            return;
        }
        self.settled = true;
        if self.is_cancelled() {
            settle_task(self.id, UiTaskStatus::Cancelled, None);
        } else {
            settle_task(self.id, UiTaskStatus::Failed, Some(error.into()));
        }
    }

    /// Mark as cancelled without treating as failure.
    pub fn cancel(mut self) {
        if self.settled {
            return;
        }
        self.settled = true;
        self.task_cancel.store(true, Ordering::Relaxed);
        settle_task(self.id, UiTaskStatus::Cancelled, None);
    }
}

/// Groups related tasks (e.g. one Agent investigation). Cancelling any member cancels the session.
pub struct UiTaskSession {
    group_id: u64,
    cancel: Arc<AtomicBool>,
}

impl UiTaskSession {
    /// Start a new cancellable session (Agent / skill run).
    pub fn start() -> Self {
        let mut state = UI_TASKS.write();
        let group_id = state.next_group_id;
        state.next_group_id += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        state.group_cancel.insert(group_id, cancel.clone());
        state.active_agent_group = Some(group_id);
        Self { group_id, cancel }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn open(
        &self,
        kind: UiTaskKind,
        label: impl Into<String>,
        detail: Option<String>,
    ) -> UiTaskHandle {
        open_ui_task_inner(
            kind,
            label,
            detail,
            Some(self.group_id),
            Some(self.cancel.clone()),
        )
    }
}

impl Drop for UiTaskSession {
    fn drop(&mut self) {
        let mut state = UI_TASKS.write();
        if state.active_agent_group == Some(self.group_id) {
            state.active_agent_group = None;
        }
        state.group_cancel.remove(&self.group_id);
    }
}

/// Open a standalone task (not tied to a session).
pub fn open_ui_task(
    kind: UiTaskKind,
    label: impl Into<String>,
    detail: Option<String>,
) -> UiTaskHandle {
    open_ui_task_inner(kind, label, detail, None, None)
}

fn open_ui_task_inner(
    kind: UiTaskKind,
    label: impl Into<String>,
    detail: Option<String>,
    group_id: Option<u64>,
    group_cancel: Option<Arc<AtomicBool>>,
) -> UiTaskHandle {
    let mut state = UI_TASKS.write();
    let id = state.next_id;
    state.next_id += 1;
    let token = Arc::new(AtomicBool::new(false));
    state.cancel_tokens.insert(id, token.clone());
    state.tasks.push(UiTask {
        id,
        kind,
        label: label.into(),
        detail,
        status: UiTaskStatus::Running,
        group_id,
        started_ms: now_ms(),
        finished_ms: None,
        error: None,
    });
    trim_tasks(&mut state);
    UiTaskHandle {
        id,
        task_cancel: token,
        group_cancel,
        settled: false,
    }
}

/// Cancel a task. If it belongs to a session group, the whole session is cancelled.
pub fn cancel_ui_task(id: u64) {
    let group_id = {
        let state = UI_TASKS.read();
        state
            .tasks
            .iter()
            .find(|t| t.id == id)
            .and_then(|t| t.group_id)
    };
    if let Some(gid) = group_id {
        cancel_ui_group(gid);
        return;
    }
    cancel_single_ui_task(id);
}

fn cancel_single_ui_task(id: u64) {
    {
        let state = UI_TASKS.read();
        if let Some(token) = state.cancel_tokens.get(&id) {
            token.store(true, Ordering::Relaxed);
        }
    }
    settle_task(id, UiTaskStatus::Cancelled, None);
    let mut state = UI_TASKS.write();
    if state.snapshot_task_id == Some(id) {
        state.snapshot_task_id = None;
    }
}

pub fn cancel_ui_group(group_id: u64) {
    if let Some(cancel) = UI_TASKS.read().group_cancel.get(&group_id).cloned() {
        cancel.store(true, Ordering::Relaxed);
    }
    let mut state = UI_TASKS.write();
    let ids: Vec<u64> = state
        .tasks
        .iter()
        .filter(|t| t.group_id == Some(group_id) && t.is_running())
        .map(|t| t.id)
        .collect();
    for id in ids {
        if let Some(token) = state.cancel_tokens.get(&id) {
            token.store(true, Ordering::Relaxed);
        }
        if let Some(task) = state.tasks.iter_mut().find(|t| t.id == id) {
            task.status = UiTaskStatus::Cancelled;
            task.finished_ms = Some(now_ms());
            task.error = None;
        }
        state.cancel_tokens.remove(&id);
    }
    if state.active_agent_group == Some(group_id) {
        state.active_agent_group = None;
    }
}

pub fn cancel_all_running_ui_tasks() {
    let ids: Vec<u64> = UI_TASKS
        .read()
        .tasks
        .iter()
        .filter(|t| t.is_running())
        .map(|t| t.id)
        .collect();
    for id in ids {
        cancel_ui_task(id);
    }
}

pub fn clear_finished_ui_tasks() {
    let mut state = UI_TASKS.write();
    state.tasks.retain(|t| t.is_running());
}

pub fn running_ui_task_count() -> usize {
    UI_TASKS
        .read()
        .tasks
        .iter()
        .filter(|t| t.is_running())
        .count()
}

pub fn any_ui_task_running() -> bool {
    running_ui_task_count() > 0
}

/// True while an Agent session has running work.
pub fn ui_agent_busy() -> bool {
    let state = UI_TASKS.read();
    let Some(gid) = state.active_agent_group else {
        return false;
    };
    state
        .tasks
        .iter()
        .any(|t| t.group_id == Some(gid) && t.is_running())
}

/// Register a page snapshot task; cancels any previous in-flight snapshot.
pub fn begin_snapshot_task(label: impl Into<String>, detail: Option<String>) -> UiTaskHandle {
    let prev = UI_TASKS.read().snapshot_task_id;
    if let Some(prev_id) = prev {
        cancel_single_ui_task(prev_id);
    }
    let handle = open_ui_task(UiTaskKind::Snapshot, label, detail);
    UI_TASKS.write().snapshot_task_id = Some(handle.id());
    handle
}

pub fn end_snapshot_task(id: u64) {
    let mut state = UI_TASKS.write();
    if state.snapshot_task_id == Some(id) {
        state.snapshot_task_id = None;
    }
}

pub fn ui_tasks_snapshot() -> Vec<UiTask> {
    UI_TASKS.read().tasks.clone()
}
