mod span;
mod step;

pub use span::{attr, Attribute, Ele, Event, Location, Span, SpanStatus, Timestamp};
pub use step::{
    advance_micro_step, crash_atomic_step, crash_step_snapshot, current_micro_step,
    set_micro_batches, step_snapshot, sync_micro_step, StepSnapshot,
};

// --- Custom Error Type ---

/// Represents errors that can occur during tracing operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TraceError {
    /// Indicates that an operation was attempted on a span that has already been closed.
    #[error("span already closed")]
    SpanAlreadyClosed,
}
