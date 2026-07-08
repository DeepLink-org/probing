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
    pub phase: Option<String>,
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
    pub phase: Option<String>,
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

        // Use logical event time (`time`, ns) — not memtable ingestion `timestamp` (µs).
        // Matches training step_matrix / SPANS_SQL in probing.tracing.
        let query = format!(
            r#"
            SELECT
                record_type,
                trace_id,
                span_id,
                COALESCE(parent_id, -1) as parent_id,
                name,
                time AS timestamp,
                COALESCE(thread_id, 0) as thread_id,
                phase,
                location,
                attributes,
                event_attributes
            FROM python.trace_event
            ORDER BY time DESC
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
        let record_type_idx = df
            .names
            .iter()
            .position(|c| c == "record_type")
            .unwrap_or(0);
        let trace_id_idx = df.names.iter().position(|c| c == "trace_id").unwrap_or(1);
        let span_id_idx = df.names.iter().position(|c| c == "span_id").unwrap_or(2);
        let parent_id_idx = df.names.iter().position(|c| c == "parent_id").unwrap_or(3);
        let name_idx = df.names.iter().position(|c| c == "name").unwrap_or(4);
        let timestamp_idx = df.names.iter().position(|c| c == "timestamp").unwrap_or(5);
        let thread_id_idx = df.names.iter().position(|c| c == "thread_id").unwrap_or(6);
        let phase_idx = df.names.iter().position(|c| c == "phase").unwrap_or(7);
        let location_idx = df.names.iter().position(|c| c == "location").unwrap_or(8);
        let attributes_idx = df.names.iter().position(|c| c == "attributes").unwrap_or(9);
        let event_attributes_idx = df
            .names
            .iter()
            .position(|c| c == "event_attributes")
            .unwrap_or(10);

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
                phase: get_opt_str(phase_idx),
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
        let mut span_map: std::collections::HashMap<i64, SpanInfo> =
            std::collections::HashMap::new();
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
                    phase: event.phase.clone(),
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
        let mut spans_to_process: Vec<(i64, usize)> = span_map
            .keys()
            .map(|&id| (id, depth_map.get(&id).copied().unwrap_or(0)))
            .collect();
        spans_to_process.sort_by_key(|b| std::cmp::Reverse(b.1)); // Sort by depth descending

        // Process spans from deepest to shallowest
        // This ensures that when we add a child to its parent, the child's children
        // have already been added to the child
        for (span_id, _depth) in spans_to_process {
            let parent_id = span_map
                .get(&span_id)
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

    /// Get JSON data in Chrome tracing format via the Python extension API.
    pub async fn get_chrome_tracing_json(&self, limit: Option<usize>) -> Result<String> {
        let limit = limit.unwrap_or(1000);
        let path = format!("/apis/pythonext/trace/chrome-tracing?limit={limit}");
        let response = self.get_request(&path).await?;

        let json_value: serde_json::Value = serde_json::from_str(&response)?;
        if let Some(error_obj) = json_value.get("error") {
            return Err(crate::utils::error::AppError::Api(format!(
                "Backend error: {}",
                error_obj
            )));
        }

        Ok(response)
    }

    /// Get Ray task execution timeline
    #[allow(dead_code)]
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

        let path = format!("/apis/pythonext/ray/timeline{}", query_string);
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

        let path = format!("/apis/pythonext/ray/timeline/chrome{}", query_string);
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

#[allow(dead_code)]
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
    pub phase: Option<String>,
    pub thread_id: i64,
    pub attributes: Option<serde_json::Value>,
}
