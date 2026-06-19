//! Stack page filter/refresh and latest frame counts (sidebar + main content).

use dioxus::prelude::*;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct StackSnapshot {
    pub tid_label: String,
    pub total: usize,
    pub py: usize,
    pub rust: usize,
    pub cpp: usize,
    pub shown: usize,
    pub loaded: bool,
}

pub static STACK_MODE: GlobalSignal<String> = Signal::global(|| String::from("mixed"));
pub static STACK_REFRESH: GlobalSignal<u32> = Signal::global(|| 0);
pub static STACK_SNAPSHOT: GlobalSignal<StackSnapshot> = Signal::global(StackSnapshot::default);

pub fn stack_tid_label(tid: Option<&str>) -> String {
    match tid {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => String::from("default"),
    }
}

pub fn bump_stack_refresh() {
    *STACK_REFRESH.write() = STACK_REFRESH().wrapping_add(1);
}
