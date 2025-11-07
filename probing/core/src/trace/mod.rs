mod span;

pub use span::{attr, Attribute, Ele, Event, Location, Span, SpanStatus, Timestamp};

// --- Custom Error Type ---

/// Represents errors that can occur during tracing operations.
#[derive(Debug)]
pub enum TraceError {
    /// Indicates that an operation was attempted on a span that has already been closed.
    SpanAlreadyClosed,
}
