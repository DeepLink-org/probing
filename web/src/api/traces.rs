use super::ApiClient;
use crate::utils::error::Result;
use probing_proto::prelude::Ele;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEvent {
    pub record_type: String,
    pub trace_id: i64,
    pub span_id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub timestamp: i64,
    pub thread_id: i64,
    pub kind: Option<String>,
    pub location: Option<String>,
    pub attributes: Option<String>,
    pub event_attributes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpanInfo {
    pub span_id: i64,
    pub trace_id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub start_timestamp: i64,
    pub end_timestamp: Option<i64>,
    pub thread_id: i64,
    pub kind: Option<String>,
    pub location: Option<String>,
    pub attributes: Option<String>,
    pub children: Vec<SpanInfo>,
    pub events: Vec<EventInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventInfo {
    pub name: String,
    pub timestamp: i64,
    pub attributes: Option<String>,
}

/// Tracing API
impl ApiClient {
    /// Get trace events, supports limiting count
    pub async fn get_trace_events(&self, limit: Option<usize>) -> Result<Vec<TraceEvent>> {
        let limit_clause = if let Some(limit) = limit {
            format!("LIMIT {}", limit)
        } else {
            String::new()
        };

        let query = format!(
            r#"
            SELECT
                record_type,
                trace_id,
                span_id,
                COALESCE(parent_id, -1) as parent_id,
                name,
                timestamp,
                COALESCE(thread_id, 0) as thread_id,
                kind,
                location,
                attributes,
                event_attributes
            FROM python.trace_event
            ORDER BY timestamp DESC
            {}
        "#,
            limit_clause
        );

        let df = self.execute_query(&query).await?;

        // Convert DataFrame to Vec<TraceEvent>
        let mut events = Vec::new();

        if df.names.is_empty() || df.cols.is_empty() {
            return Ok(events);
        }

        // Find column indices
        let record_type_idx = df.names.iter().position(|c| c == "record_type").unwrap_or(0);
        let trace_id_idx = df.names.iter().position(|c| c == "trace_id").unwrap_or(1);
        let span_id_idx = df.names.iter().position(|c| c == "span_id").unwrap_or(2);
        let parent_id_idx = df.names.iter().position(|c| c == "parent_id").unwrap_or(3);
        let name_idx = df.names.iter().position(|c| c == "name").unwrap_or(4);
        let timestamp_idx = df.names.iter().position(|c| c == "timestamp").unwrap_or(5);
        let thread_id_idx = df.names.iter().position(|c| c == "thread_id").unwrap_or(6);
        let kind_idx = df.names.iter().position(|c| c == "kind").unwrap_or(7);
        let location_idx = df.names.iter().position(|c| c == "location").unwrap_or(8);
        let attributes_idx = df.names.iter().position(|c| c == "attributes").unwrap_or(9);
        let event_attributes_idx = df.names.iter().position(|c| c == "event_attributes").unwrap_or(10);

        // Get number of rows
        let nrows = df.cols.iter().map(|col| col.len()).max().unwrap_or(0);

        for row_idx in 0..nrows {
            let get_str = |idx: usize| -> String {
                match df.cols.get(idx).map(|col| col.get(row_idx)) {
                    Some(Ele::Text(s)) => s.clone(),
                    Some(Ele::I32(x)) => x.to_string(),
                    Some(Ele::I64(x)) => x.to_string(),
                    Some(Ele::F32(x)) => x.to_string(),
                    Some(Ele::F64(x)) => x.to_string(),
                    _ => "".to_string(),
                }
            };

            let get_i64 = |idx: usize| -> i64 {
                match df.cols.get(idx).map(|col| col.get(row_idx)) {
                    Some(Ele::I32(x)) => x as i64,
                    Some(Ele::I64(x)) => x,
                    Some(Ele::F32(x)) => x as i64,
                    Some(Ele::F64(x)) => x as i64,
                    Some(Ele::Text(s)) => s.parse().unwrap_or(0),
                    _ => 0,
                }
            };

            let get_opt_str = |idx: usize| -> Option<String> {
                match df.cols.get(idx).map(|col| col.get(row_idx)) {
                    Some(Ele::Text(s)) if !s.is_empty() => Some(s.clone()),
                    _ => None,
                }
            };

            let get_opt_i64 = |idx: usize| -> Option<i64> {
                let val = get_i64(idx);
                if val == -1 {
                    None
                } else {
                    Some(val)
                }
            };

            events.push(TraceEvent {
                record_type: get_str(record_type_idx),
                trace_id: get_i64(trace_id_idx),
                span_id: get_i64(span_id_idx),
                parent_id: get_opt_i64(parent_id_idx),
                name: get_str(name_idx),
                timestamp: get_i64(timestamp_idx),
                thread_id: get_i64(thread_id_idx),
                kind: get_opt_str(kind_idx),
                location: get_opt_str(location_idx),
                attributes: get_opt_str(attributes_idx),
                event_attributes: get_opt_str(event_attributes_idx),
            });
        }

        Ok(events)
    }

    /// Build span tree structure, supports limiting count
    pub async fn get_span_tree(&self, limit: Option<usize>) -> Result<Vec<SpanInfo>> {
        let events = self.get_trace_events(limit).await?;

        // Build span map from span_start events
        let mut span_map: std::collections::HashMap<i64, SpanInfo> = std::collections::HashMap::new();
        let mut root_spans: Vec<i64> = Vec::new();

        for event in &events {
            if event.record_type == "span_start" {
                let span = SpanInfo {
                    span_id: event.span_id,
                    trace_id: event.trace_id,
                    parent_id: event.parent_id,
                    name: event.name.clone(),
                    start_timestamp: event.timestamp,
                    end_timestamp: None,
                    thread_id: event.thread_id,
                    kind: event.kind.clone(),
                    location: event.location.clone(),
                    attributes: event.attributes.clone(),
                    children: Vec::new(),
                    events: Vec::new(),
                };

                if event.parent_id.is_none() || event.parent_id == Some(-1) {
                    root_spans.push(event.span_id);
                }

                span_map.insert(event.span_id, span);
            } else if event.record_type == "span_end" {
                if let Some(span) = span_map.get_mut(&event.span_id) {
                    span.end_timestamp = Some(event.timestamp);
                }
            } else if event.record_type == "event" {
                if let Some(span) = span_map.get_mut(&event.span_id) {
                    span.events.push(EventInfo {
                        name: event.name.clone(),
                        timestamp: event.timestamp,
                        attributes: event.event_attributes.clone(),
                    });
                }
            }
        }

        // Build tree structure - process from deepest to shallowest
        // Calculate depth for each span using iterative approach
        let mut depth_map: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();

        // Initialize all root spans to depth 0
        for root_id in &root_spans {
            depth_map.insert(*root_id, 0);
        }

        // Iteratively calculate depths until no changes
        let mut changed = true;
        while changed {
            changed = false;
            for (span_id, span) in span_map.iter() {
                if depth_map.contains_key(span_id) {
                    continue; // Already calculated
                }

                if let Some(parent_id) = span.parent_id {
                    if parent_id != -1 && depth_map.contains_key(&parent_id) {
                        let parent_depth = depth_map[&parent_id];
                        depth_map.insert(*span_id, parent_depth + 1);
                        changed = true;
                    }
                } else {
                    // Root span (should have been added already, but handle it)
                    depth_map.insert(*span_id, 0);
                    changed = true;
                }
            }
        }

        // Sort spans by depth (deepest first) so we process children before parents
        let mut spans_to_process: Vec<(i64, usize)> = span_map.keys()
            .map(|&id| (id, depth_map.get(&id).copied().unwrap_or(0)))
            .collect();
        spans_to_process.sort_by(|a, b| b.1.cmp(&a.1)); // Sort by depth descending

        // Process spans from deepest to shallowest
        // This ensures that when we add a child to its parent, the child's children
        // have already been added to the child
        for (span_id, _depth) in spans_to_process {
            let parent_id = span_map.get(&span_id)
                .and_then(|span| span.parent_id)
                .filter(|&pid| pid != -1);

            if let Some(parent_id) = parent_id {
                // Remove child from map and add to parent
                if let Some(child) = span_map.remove(&span_id) {
                    if let Some(parent) = span_map.get_mut(&parent_id) {
                        parent.children.push(child);
                    } else {
                        // Parent not found (shouldn't happen if depth calculation is correct)
                        // Put child back as orphan
                        span_map.insert(span_id, child);
                    }
                }
            }
        }

        // Collect root spans
        let mut result = Vec::new();
        for root_id in root_spans {
            if let Some(span) = span_map.remove(&root_id) {
                result.push(span);
            }
        }

        // Add any remaining spans (orphans)
        for (_, span) in span_map {
            result.push(span);
        }

        // Sort by start timestamp
        result.sort_by_key(|s| s.start_timestamp);

        Ok(result)
    }

    /// Get JSON data in Chrome tracing format
    /// Returns format compatible with Chrome DevTools tracing viewer
    pub async fn get_chrome_tracing_json(&self, limit: Option<usize>) -> Result<String> {
        let mut events = self.get_trace_events(limit).await?;

        // Sort by timestamp ascending, ensure span_start is processed before span_end
        // This way when processing span_end, the corresponding span_start is already in span_starts
        events.sort_by_key(|e| e.timestamp);

        // Find minimum timestamp as baseline
        let min_timestamp = events.iter()
            .map(|e| e.timestamp)
            .min()
            .unwrap_or(0);

        // Convert to Chrome tracing format
        let mut trace_events: Vec<serde_json::Value> = Vec::new();

        // Use (span_id, thread_id) as key to track span start time, supports multi-threaded scenarios
        // Value contains: (start timestamp in microseconds, span name, kind, trace_id)
        let mut span_starts: std::collections::HashMap<(i64, i64), (i64, String, Option<String>, i64)> = std::collections::HashMap::new();

        // First pass: collect all span_start events, build lookup table
        // This helps match span_end events, even if trace_id in span_end is 0
        let mut span_start_lookup: std::collections::HashMap<(i64, i64), (i64, String, Option<String>, i64)> = std::collections::HashMap::new();
        // Build span_id to parent_id mapping, used to find top-level spans
        let mut span_to_parent: std::collections::HashMap<i64, Option<i64>> = std::collections::HashMap::new();
        // Find first (earliest) top-level span's trace_id, use as unified pid
        let mut unified_pid: Option<i64> = None;

        for event in &events {
            if event.record_type == "span_start" {
                let key = (event.span_id, event.thread_id);
                span_start_lookup.insert(key, (
                    event.timestamp,
                    event.name.clone(),
                    event.kind.clone(),
                    event.trace_id,
                ));

                // Record parent_id mapping
                span_to_parent.insert(event.span_id, event.parent_id);

                // If it's a top-level span (no parent_id or parent_id = -1), and unified_pid not set yet
                // Use first top-level span's trace_id as unified pid
                if unified_pid.is_none() && (event.parent_id.is_none() || event.parent_id == Some(-1)) {
                    unified_pid = Some(event.trace_id);
                }
            }
        }

        // If no top-level span found, use first span_start's trace_id
        let unified_pid = unified_pid.unwrap_or_else(|| {
            events.iter()
                .find(|e| e.record_type == "span_start")
                .map(|e| e.trace_id)
                .unwrap_or(1)
        });

        // Second pass: convert events to Chrome tracing format
        for event in &events {
            // Convert nanoseconds to microseconds (Chrome tracing uses microseconds)
            let ts_micros = (event.timestamp - min_timestamp) / 1000;
            // All top-level spans use unified pid, child spans also use unified pid (they belong to same logical trace)
            // This ensures all related spans are displayed in the same process
            let pid = unified_pid as u32;
            let tid = event.thread_id as u32;

            match event.record_type.as_str() {
                "span_start" => {
                    // Use (span_id, thread_id) as key
                    let key = (event.span_id, event.thread_id);
                    // Store using unified pid
                    span_starts.insert(key, (ts_micros, event.name.clone(), event.kind.clone(), unified_pid));

                    // Create 'B' (Begin) event
                    let mut chrome_event = serde_json::json!({
                        "name": event.name,
                        "cat": event.kind.as_ref().unwrap_or(&"span".to_string()),
                        "ph": "B",
                        "ts": ts_micros,
                        "pid": pid,
                        "tid": tid,
                    });

                    // Add optional parameters
                    let mut args = serde_json::Map::new();
                    if let Some(ref location) = event.location {
                        if !location.is_empty() {
                            args.insert("location".to_string(), serde_json::Value::String(location.clone()));
                        }
                    }
                    if let Some(ref attrs) = event.attributes {
                        if !attrs.is_empty() {
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(attrs) {
                                args.insert("attributes".to_string(), parsed);
                            }
                        }
                    }
                    if !args.is_empty() {
                        chrome_event["args"] = serde_json::Value::Object(args);
                    }

                    trace_events.push(chrome_event);
                }
                "span_end" => {
                    // Use (span_id, thread_id) as key to find corresponding span_start
                    let key = (event.span_id, event.thread_id);

                    // First try to find from already processed events
                    if let Some((start_ts, start_name, start_kind, start_pid)) = span_starts.get(&key) {
                        // Found matching span_start, create 'E' (End) event
                        let mut chrome_event = serde_json::json!({
                            "name": start_name,
                            "cat": start_kind.as_ref().unwrap_or(&"span".to_string()),
                            "ph": "E",
                            "ts": ts_micros,
                            "pid": *start_pid as u32,
                            "tid": tid,
                        });

                        // Calculate duration (in microseconds)
                        let dur = ts_micros - start_ts;
                        if dur > 0 {
                            chrome_event["dur"] = serde_json::Value::Number(dur.into());
                        }

                        trace_events.push(chrome_event);
                        // Remove from span_starts to avoid duplicate matching
                        span_starts.remove(&key);
                    } else if let Some((start_timestamp, start_name, start_kind, _)) = span_start_lookup.get(&key) {
                        // Find span_start information from lookup table
                        let start_ts_micros = (start_timestamp - min_timestamp) / 1000;
                        // Use unified pid to ensure all spans are in the same process
                        let mut chrome_event = serde_json::json!({
                            "name": start_name,
                            "cat": start_kind.as_ref().unwrap_or(&"span".to_string()),
                            "ph": "E",
                            "ts": ts_micros,
                            "pid": unified_pid as u32,
                            "tid": tid,
                        });

                        // Calculate duration (in microseconds)
                        let dur = ts_micros - start_ts_micros;
                        if dur > 0 {
                            chrome_event["dur"] = serde_json::Value::Number(dur.into());
                        }

                        trace_events.push(chrome_event);
                    } else {
                        // No matching span_start found (may have been filtered by limit)
                        // Use unified pid to ensure all spans are in the same process
                        let mut chrome_event = serde_json::json!({
                            "name": if event.name.is_empty() { "unknown_span" } else { &event.name },
                            "cat": "span",
                            "ph": "E",
                            "ts": ts_micros,
                            "pid": unified_pid as u32,
                            "tid": tid,
                        });
                        trace_events.push(chrome_event);
                    }
                }
                "event" => {
                    // Create 'i' (Instant) event
                    let mut chrome_event = serde_json::json!({
                        "name": event.name,
                        "cat": "event",
                        "ph": "i",
                        "ts": ts_micros,
                        "pid": pid,
                        "tid": tid,
                        "s": "t", // scope: thread
                    });

                    // Add event attributes
                    if let Some(ref attrs) = event.event_attributes {
                        if !attrs.is_empty() {
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(attrs) {
                                chrome_event["args"] = parsed;
                            }
                        }
                    }

                    trace_events.push(chrome_event);
                }
                _ => {}
            }
        }

        // Build complete Chrome tracing format JSON
        let chrome_trace = serde_json::json!({
            "traceEvents": trace_events,
            "displayTimeUnit": "ms",
        });

        Ok(serde_json::to_string_pretty(&chrome_trace)?)
    }

    /// Get Ray task execution timeline
    pub async fn get_ray_timeline(
        &self,
        task_filter: Option<&str>,
        actor_filter: Option<&str>,
        start_time: Option<i64>,
        end_time: Option<i64>,
    ) -> Result<Vec<RayTimelineEntry>> {
        let mut query_params = Vec::new();

        if let Some(filter) = task_filter {
            query_params.push(format!("task_filter={}", urlencoding::encode(filter)));
        }
        if let Some(filter) = actor_filter {
            query_params.push(format!("actor_filter={}", urlencoding::encode(filter)));
        }
        if let Some(time) = start_time {
            query_params.push(format!("start_time={}", time));
        }
        if let Some(time) = end_time {
            query_params.push(format!("end_time={}", time));
        }

        let query_string = if query_params.is_empty() {
            String::new()
        } else {
            format!("?{}", query_params.join("&"))
        };

        let path = format!("/apis/python/ray/timeline{}", query_string);
        let response = self.get_request(&path).await?;
        Self::parse_json(&response)
    }

    /// Get Ray timeline in Chrome tracing format (for Perfetto UI)
    pub async fn get_ray_timeline_chrome_format(
        &self,
        task_filter: Option<&str>,
        actor_filter: Option<&str>,
        start_time: Option<i64>,
        end_time: Option<i64>,
    ) -> Result<String> {
        let mut query_params = Vec::new();

        if let Some(filter) = task_filter {
            query_params.push(format!("task_filter={}", urlencoding::encode(filter)));
        }
        if let Some(filter) = actor_filter {
            query_params.push(format!("actor_filter={}", urlencoding::encode(filter)));
        }
        if let Some(time) = start_time {
            query_params.push(format!("start_time={}", time));
        }
        if let Some(time) = end_time {
            query_params.push(format!("end_time={}", time));
        }

        let query_string = if query_params.is_empty() {
            String::new()
        } else {
            format!("?{}", query_params.join("&"))
        };

        let path = format!("/apis/python/ray/timeline/chrome{}", query_string);
        let response = self.get_request(&path).await?;

        // Check for error in the response JSON
        let json_value: serde_json::Value = serde_json::from_str(&response)?;
        if let Some(error_obj) = json_value.get("error") {
            return Err(crate::utils::error::AppError::Api(format!(
                "Backend error: {}",
                error_obj
            )));
        }

        Ok(response)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RayTimelineEntry {
    pub name: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    pub start_time: i64,
    pub end_time: Option<i64>,
    pub duration: Option<i64>,
    pub trace_id: i64,
    pub span_id: i64,
    pub parent_id: Option<i64>,
    pub kind: Option<String>,
    pub thread_id: i64,
    pub attributes: Option<serde_json::Value>,
}
