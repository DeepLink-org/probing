mod span;
mod step;

pub use span::{attr, Attribute, Ele, Event, Location, Span, SpanStatus, Timestamp};
pub use step::{
    advance_local_step, current_local_step, set_step_bucket_size, step_snapshot, sync_local_step,
    StepSnapshot,
};

// --- Custom Error Type ---

/// Represents errors that can occur during tracing operations.
#[derive(Debug)]
pub enum TraceError {
    /// Indicates that an operation was attempted on a span that has already been closed.
    SpanAlreadyClosed,
}
