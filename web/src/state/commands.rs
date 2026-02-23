//! Global state for Command Panel and REPL execution.

use dioxus::prelude::*;

/// Output of one eval
#[derive(Clone)]
pub struct EvalState {
    pub output: String,
    pub is_error: bool,
}

/// One completed cell: input command + output
#[derive(Clone)]
pub struct Cell {
    pub input: String,
    pub output: EvalState,
}

/// Floating result shown after executing a command
#[derive(Clone)]
pub struct FloatingResult {
    pub command: String,
    pub output: String,
    pub is_error: bool,
}

pub static COMMAND_PANEL_OPEN: GlobalSignal<bool> = Signal::global(|| false);
pub static COMMAND_INPUT: GlobalSignal<String> = Signal::global(|| String::new());
pub static EVAL_HISTORY: GlobalSignal<Vec<Cell>> = Signal::global(Vec::new);
