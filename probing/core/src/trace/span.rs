use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

pub use probing_proto::types::Ele;

// Global atomic counters for generating unique IDs.
static NEXT_TRACE_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_SPAN_ID: AtomicU64 = AtomicU64::new(1);

/// Obtain a numeric thread identifier using platform facilities where possible.
///
/// On macOS we use `pthread_self()` which is stable per thread lifetime.
/// On Linux we use the `gettid` syscall for the OS thread id.
/// On other platforms we hash the opaque `std::thread::ThreadId` debug output
/// to yield a reproducible u64 within process lifetime.
fn current_thread_id() -> u64 {
    #[cfg(target_os = "macos")]
    unsafe {
        return libc::pthread_self() as u64;
    }
    #[cfg(target_os = "linux")]
    unsafe {
        return libc::syscall(libc::SYS_gettid) as u64;
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let tid = thread::current().id();
        let mut h = DefaultHasher::new();
        // ThreadId only implements Debug; convert to string and hash.
        format!("{:?}", tid).hash(&mut h);
        h.finish()
    }
}

// --- Timestamp ---
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp(pub u128);

impl Timestamp {
    pub fn now() -> Self {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or_else(
                |_| Timestamp(0), // Fallback for systems where time might be before UNIX_EPOCH
                |d| Timestamp(d.as_nanos()),
            )
    }

    pub fn duration_since(&self, earlier: Timestamp) -> Duration {
        if self.0 > earlier.0 {
            Duration::from_nanos((self.0 - earlier.0) as u64)
        } else {
            Duration::from_nanos(0) // Avoid panic if earlier is not actually earlier
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attribute(pub String, pub Ele);

pub fn attr<K: Into<String>, V: Into<Ele>>(key: K, value: V) -> Attribute {
    Attribute(key.into(), value.into())
}

impl Attribute {
    pub fn key(&self) -> &str {
        &self.0
    }

    pub fn value(&self) -> &Ele {
        &self.1
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Location {
    KnownLocation(u64),
    UnknownLocation(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub name: String,
    pub location: Option<Location>,
    pub timestamp: Timestamp,
    pub attributes: Vec<Attribute>,
}

// --- Span Status ---
/// Represents the status of a span.
///
/// The status is determined by whether the span has been ended:
/// - `Active`: The span is still running (end_time is None)
/// - `Completed`: The span has been ended (end_time is Some)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpanStatus {
    Active,    // The span is currently active (end_time is None).
    Completed, // The span has been completed (end_time is Some).
}

impl SpanStatus {
    /// Returns `Active` if end_time is None, `Completed` otherwise.
    pub fn from_end_time(end_time: Option<Timestamp>) -> Self {
        if end_time.is_some() {
            SpanStatus::Completed
        } else {
            SpanStatus::Active
        }
    }
}

#[derive(Debug, Clone)]
pub struct Span {
    // === 标识符 ===
    pub trace_id: u64,
    pub span_id: u64,
    pub parent_id: Option<u64>,
    pub thread_id: u64, // stable numeric id for the originating thread

    // === 基本信息 ===
    pub name: String,

    // === 时间信息 ===
    pub start: Timestamp,
    pub end: Option<Timestamp>,

    // === 元数据 ===
    pub kind: Option<String>,
    pub loc: Option<Location>,

    // === 扩展数据 ===
    pub attrs: Vec<Attribute>,
    pub events: Vec<Event>,
}

impl Span {
    /// Creates a new root span (starts a new trace).
    pub fn new_root<N: Into<String>>(name: N, kind: Option<&str>, location: Option<&str>) -> Self {
        let trace_id = NEXT_TRACE_ID.fetch_add(1, Ordering::Relaxed);
        let span_id = NEXT_SPAN_ID.fetch_add(1, Ordering::Relaxed);
        let location = location.map(|loc_val| Location::UnknownLocation(loc_val.into()));
        let thread_id = current_thread_id();

        Span {
            trace_id,
            span_id,
            parent_id: None,
            thread_id,
            name: name.into(),
            start: Timestamp::now(),
            end: None,
            kind: kind.map(|k| k.to_string()),
            loc: location,
            attrs: vec![],
            events: vec![],
        }
    }

    /// Creates a new child span within an existing trace.
    pub fn new_child<N: Into<String>>(
        parent: &Span,
        name: N,
        kind: Option<&str>,
        location: Option<&str>,
    ) -> Self {
        let span_id = NEXT_SPAN_ID.fetch_add(1, Ordering::Relaxed);
        let location = location.map(|loc_val| Location::UnknownLocation(loc_val.into()));
        let thread_id = current_thread_id(); // child bound to the current executing thread

        Span {
            trace_id: parent.trace_id,
            span_id,
            parent_id: Some(parent.span_id),
            thread_id,
            name: name.into(),
            start: Timestamp::now(),
            end: None,
            kind: kind.map(|k| k.to_string()),
            loc: location,
            attrs: vec![],
            events: vec![],
        }
    }

    /// Adds an attribute to this span.
    ///
    /// Returns an error if the span has already been ended.
    pub fn add_attr<V: Into<Ele>>(&mut self, key: &str, value: V) -> Result<(), super::TraceError> {
        if self.end.is_some() {
            return Err(super::TraceError::SpanAlreadyClosed);
        }
        self.attrs.push(attr(key, value));
        Ok(())
    }

    /// Adds an event to this span.
    ///
    /// Returns an error if the span has already been ended.
    pub fn add_event<S: Into<String>>(
        &mut self,
        name: S,
        attributes: Option<Vec<Attribute>>,
    ) -> Result<(), super::TraceError> {
        if self.end.is_some() {
            return Err(super::TraceError::SpanAlreadyClosed);
        }

        self.events.push(Event {
            name: name.into(),
            location: None,
            timestamp: Timestamp::now(),
            attributes: attributes.unwrap_or_default(),
        });

        Ok(())
    }

    /// Ends this span.
    pub fn finish(&mut self) {
        self.end = Some(Timestamp::now());
    }

    /// Ends this span (alias for `finish()`).
    pub fn end(&mut self) {
        self.finish();
    }

    /// Ends this span with success status (alias for `end()`).
    pub fn end_success(&mut self) {
        self.end();
    }

    /// Ends this span and optionally records an error message as an attribute.
    pub fn end_error(&mut self, error_message: Option<String>) {
        if let Some(msg) = error_message {
            // Record error message as an attribute
            let _ = self.add_attr("error.message", msg);
        }
        self.finish();
    }

    /// Returns the status of this span.
    pub fn status(&self) -> SpanStatus {
        SpanStatus::from_end_time(self.end)
    }

    /// Returns the duration of this span if it has been ended.
    pub fn duration(&self) -> Option<Duration> {
        self.end.map(|et| et.duration_since(self.start))
    }

    /// Checks if this span has been ended.
    pub fn is_ended(&self) -> bool {
        self.end.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration as StdDuration;

    // --- 1. Basic Span Functionality ---

    #[test]
    fn test_new_root_span() {
        let span = Span::new_root(
            "process_incoming_request",
            Some("server_op"),
            Some("my_app::request_handler"),
        );

        assert_eq!(span.name, "process_incoming_request");
        assert_eq!(span.kind, Some("server_op".to_string()));
        assert_eq!(span.parent_id, None, "Root span has no parent");
        assert_eq!(
            span.status(),
            SpanStatus::Active,
            "New span should be active"
        );
        match &span.loc {
            Some(Location::UnknownLocation(path)) => assert_eq!(path, "my_app::request_handler"),
            _ => panic!("Expected UnknownLocation with the specified code_path"),
        }

        assert!(span.trace_id > 0, "Trace ID should be positive");
        assert!(span.span_id > 0, "Span ID should be positive");
        assert!(!span.is_ended(), "New span should not be ended");
    }

    #[test]
    fn test_new_child_span() {
        let parent = Span::new_root("root_operation", None, None);

        let child = Span::new_child(
            &parent,
            "database_query",
            Some("db_client"),
            Some("my_app::db_service::query"),
        );

        assert_eq!(child.name, "database_query");
        assert_eq!(
            child.parent_id,
            Some(parent.span_id),
            "Child's parent should be the root span"
        );
        assert_eq!(child.status(), SpanStatus::Active);
        assert_eq!(
            child.trace_id, parent.trace_id,
            "Child span must share the same trace_id as its parent"
        );
        assert!(child.attrs.is_empty(), "Initial attributes should be empty");

        // Note: Without a manager, we don't modify parent status.
        // The parent status remains unchanged.
        assert_eq!(
            parent.status(),
            SpanStatus::Active,
            "Parent span status remains unchanged without manager"
        );
    }

    #[test]
    fn test_end_span() {
        let mut span = Span::new_root("single_task", None, None);
        assert!(!span.is_ended(), "Span should not be ended initially");

        span.end();
        assert!(span.is_ended(), "Span should be ended");
        assert!(span.end.is_some(), "End time must be set");
        assert_eq!(span.status(), SpanStatus::Completed);
        assert!(span.duration().is_some(), "Duration should be available");
    }

    #[test]
    fn test_end_span_with_error() {
        let mut span = Span::new_root("error_task", None, None);
        let error_message = "Something went wrong".to_string();

        span.end_error(Some(error_message.clone()));
        assert!(span.is_ended(), "Span should be ended");
        assert_eq!(span.status(), SpanStatus::Completed);
        // Verify error message was recorded as an attribute
        assert!(
            span.attrs.iter().any(|attr| {
                attr.0 == "error.message"
                    && matches!(&attr.1, Ele::Text(msg) if msg == &error_message)
            }),
            "Error message should be recorded as an attribute"
        );
    }

    #[test]
    fn test_add_attributes_and_events() {
        let mut span = Span::new_root("user_request_processing", None, None);

        // Add attributes with various data types.
        span.add_attr("http.method", "GET").unwrap();
        span.add_attr("http.url", "/users/123".to_string()).unwrap();
        span.add_attr("user.id", 123i32).unwrap();
        span.add_attr("request.size_bytes", 1024i64).unwrap();
        span.add_attr("cache.hit_ratio", 0.75f32).unwrap();
        span.add_attr("processing.time_ms", 123.456f64).unwrap();
        span.add_attr("custom.info", Ele::Text("important_detail".to_string()))
            .unwrap();

        // Add an event.
        span.add_event(
            "cache_lookup",
            Some(vec![
                attr("cache.key", "user_123_data"),
                attr("cache.hit", true),
            ]),
        )
        .unwrap();

        // Simulate some work
        std::thread::sleep(StdDuration::from_millis(5));

        span.add_event("validation_complete", None).unwrap(); // Event without attributes

        assert_eq!(span.attrs.len(), 7, "Expected 7 attributes on the span");
        assert_eq!(span.attrs[0], attr("http.method", "GET"));
        assert_eq!(span.attrs[1], attr("http.url", "/users/123".to_string()));

        assert_eq!(span.events.len(), 2, "Expected 2 events in the span");
        assert_eq!(span.events[0].name, "cache_lookup");
        assert!(
            span.events[0].timestamp.0 > span.start.0,
            "Event timestamp should be after span start"
        );
        assert_eq!(span.events[0].attributes.len(), 2);
        assert_eq!(
            span.events[0].attributes[0],
            attr("cache.key", "user_123_data")
        );
        assert_eq!(span.events[0].attributes[1], attr("cache.hit", true));

        assert_eq!(span.events[1].name, "validation_complete");
        assert!(
            span.events[1].timestamp.0 > span.events[0].timestamp.0,
            "Second event should be later"
        );
        assert!(span.events[1].attributes.is_empty());

        span.end();

        // Behavior check: Attributes and events cannot be added to a closed span.
        let attributes_count_before = span.attrs.len();
        let events_len_before = span.events.len();

        assert!(span
            .add_attr("attempt_after_close", "should_not_be_added")
            .is_err());
        assert!(span.add_event("event_after_close", None).is_err());

        assert_eq!(
            span.attrs.len(),
            attributes_count_before,
            "Attributes should not be added to a closed span."
        );
        assert_eq!(
            span.events.len(),
            events_len_before,
            "Events should not be added to a closed span."
        );
    }

    #[test]
    fn test_trace_id_generation() {
        // First trace - should get a trace_id from atomic counter
        let span1 = Span::new_root("span1", None, None);
        let trace_id1 = span1.trace_id;
        assert!(trace_id1 > 0, "Trace ID should be positive");

        // Second trace - should get the next trace_id (incremented)
        let span2 = Span::new_root("span2", None, None);
        let trace_id2 = span2.trace_id;
        assert!(trace_id2 > trace_id1, "Trace ID should increment");

        // Third trace - should continue incrementing
        let span3 = Span::new_root("span3", None, None);
        let trace_id3 = span3.trace_id;
        assert!(
            trace_id3 > trace_id2,
            "Trace ID should continue incrementing"
        );
    }

    #[test]
    fn test_span_id_generation() {
        let span1 = Span::new_root("span1", None, None);
        let span_id1 = span1.span_id;
        assert!(span_id1 > 0, "Span ID should be positive");

        let span2 = Span::new_root("span2", None, None);
        let span_id2 = span2.span_id;
        assert!(span_id2 > span_id1, "Span ID should increment");
    }
}
