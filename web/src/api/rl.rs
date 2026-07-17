use super::traces::{EventInfo, SpanInfo, TraceEvent};
use super::ApiClient;
use crate::utils::error::Result;
use probing_proto::prelude::{DataFrame, Ele, Process};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceProcessInfo {
    pub pid: i32,
    pub process_role: Option<String>,
    pub hostname: Option<String>,
    pub ray_worker_id: Option<String>,
    pub ray_actor_id: Option<String>,
    pub ray_actor_name: Option<String>,
}

/// RL-specific tracing API (multi-process fan-out, rollout filters).
impl ApiClient {
    /// Get all trace events for spans tagged with a rollout_id.
    pub async fn get_trace_events_for_rollout_id(&self, rollout_id: &str) -> Result<Vec<TraceEvent>> {
        let span_ids = self.get_span_ids_for_rollout_id(rollout_id).await?;
        if span_ids.is_empty() {
            return Ok(Vec::new());
        }
        let query = trace_events_for_span_ids_query(&span_ids);
        let df = self.execute_query(&query).await?;
        Ok(trace_events_from_df(df))
    }

    async fn get_span_ids_for_rollout_id(&self, rollout_id: &str) -> Result<Vec<i64>> {
        let query = span_ids_for_rollout_query(rollout_id);
        let df = self.execute_query(&query).await?;
        Ok(span_ids_from_df(df))
    }

    /// Get trace events from another local probing process.
    pub async fn get_trace_events_for_pid(
        &self,
        pid: i32,
        limit: Option<usize>,
    ) -> Result<Vec<TraceEvent>> {
        let query = trace_events_query(limit);
        let df = self.execute_query_local_pid(pid, &query).await?;
        Ok(trace_events_from_df(df))
    }

    /// Get all rollout-tagged trace events from another local probing process.
    pub async fn get_trace_events_for_pid_and_rollout_id(
        &self,
        pid: i32,
        rollout_id: &str,
    ) -> Result<Vec<TraceEvent>> {
        let span_ids = self
            .get_span_ids_for_pid_and_rollout_id(pid, rollout_id)
            .await?;
        if span_ids.is_empty() {
            return Ok(Vec::new());
        }
        let query = trace_events_for_span_ids_query(&span_ids);
        let df = self.execute_query_local_pid(pid, &query).await?;
        Ok(trace_events_from_df(df))
    }

    async fn get_span_ids_for_pid_and_rollout_id(
        &self,
        pid: i32,
        rollout_id: &str,
    ) -> Result<Vec<i64>> {
        let query = span_ids_for_rollout_query(rollout_id);
        let df = self.execute_query_local_pid(pid, &query).await?;
        Ok(span_ids_from_df(df))
    }

    /// Build span tree for a single rollout_id without applying an event limit.
    pub async fn get_span_tree_for_rollout_id(&self, rollout_id: &str) -> Result<Vec<SpanInfo>> {
        let events = self.get_trace_events_for_rollout_id(rollout_id).await?;
        Ok(build_span_tree_from_events(events))
    }

    /// Build span tree from another local probing process.
    pub async fn get_span_tree_for_pid(
        &self,
        pid: i32,
        limit: Option<usize>,
    ) -> Result<Vec<SpanInfo>> {
        let events = self.get_trace_events_for_pid(pid, limit).await?;
        Ok(build_span_tree_from_events(events))
    }

    /// Build span tree for a single rollout_id from another local probing process.
    pub async fn get_span_tree_for_pid_and_rollout_id(
        &self,
        pid: i32,
        rollout_id: &str,
    ) -> Result<Vec<SpanInfo>> {
        let events = self
            .get_trace_events_for_pid_and_rollout_id(pid, rollout_id)
            .await?;
        Ok(build_span_tree_from_events(events))
    }

    /// List Ray/probing processes known to the current driver process.
    pub async fn get_trace_processes(&self) -> Result<Vec<TraceProcessInfo>> {
        let query = r#"
            SELECT
                pid,
                hostname,
                ray_worker_id,
                ray_actor_id,
                ray_actor_name,
                process_role
            FROM python.ray_process
            WHERE pid > 0
            ORDER BY pid ASC
        "#;
        let df = self.execute_query(query).await?;
        let mut processes = trace_processes_from_df(df);
        if let Ok(local_processes) = self.get_local_probing_processes().await {
            merge_local_processes(&mut processes, local_processes);
        }
        processes.sort_by(|a, b| {
            process_sort_rank(a)
                .cmp(&process_sort_rank(b))
                .then_with(|| a.pid.cmp(&b.pid))
        });
        Ok(processes)
    }

    /// List local processes exposing probing memtables.
    pub async fn get_local_probing_processes(&self) -> Result<Vec<Process>> {
        let response = self.get_request("/apis/processes/local").await?;
        Self::parse_json(&response)
    }
}

fn trace_events_query(limit: Option<usize>) -> String {
    let limit_clause = if let Some(limit) = limit {
        format!("LIMIT {}", limit)
    } else {
        String::new()
    };

    format!(
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
    )
}

fn span_ids_for_rollout_query(rollout_id: &str) -> String {
    let compact_numeric_comma = sql_string_literal(&format!("%\"rollout_id\":{},%", rollout_id));
    let compact_numeric_end = sql_string_literal(&format!("%\"rollout_id\":{}}}%", rollout_id));
    let spaced_numeric_comma = sql_string_literal(&format!("%\"rollout_id\": {},%", rollout_id));
    let spaced_numeric_end = sql_string_literal(&format!("%\"rollout_id\": {}}}%", rollout_id));
    let compact_string_comma = sql_string_literal(&format!("%\"rollout_id\":\"{}\",%", rollout_id));
    let compact_string_end = sql_string_literal(&format!("%\"rollout_id\":\"{}\"}}%", rollout_id));
    let spaced_string_comma = sql_string_literal(&format!("%\"rollout_id\": \"{}\",%", rollout_id));
    let spaced_string_end = sql_string_literal(&format!("%\"rollout_id\": \"{}\"}}%", rollout_id));

    format!(
        r#"
            SELECT DISTINCT span_id
            FROM python.trace_event
            WHERE record_type = 'span_start'
              AND (
                attributes LIKE {compact_numeric_comma}
                OR attributes LIKE {compact_numeric_end}
                OR attributes LIKE {spaced_numeric_comma}
                OR attributes LIKE {spaced_numeric_end}
                OR attributes LIKE {compact_string_comma}
                OR attributes LIKE {compact_string_end}
                OR attributes LIKE {spaced_string_comma}
                OR attributes LIKE {spaced_string_end}
              )
            ORDER BY span_id ASC
        "#
    )
}

fn trace_events_for_span_ids_query(span_ids: &[i64]) -> String {
    let span_ids = span_ids
        .iter()
        .map(|span_id| span_id.to_string())
        .collect::<Vec<_>>()
        .join(", ");

    format!(
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
            WHERE span_id IN ({span_ids})
            ORDER BY time DESC
        "#
    )
}

fn sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn span_ids_from_df(df: DataFrame) -> Vec<i64> {
    if df.names.is_empty() || df.cols.is_empty() {
        return Vec::new();
    }

    let span_id_idx = df.names.iter().position(|c| c == "span_id").unwrap_or(0);
    let nrows = df.cols.iter().map(|col| col.len()).max().unwrap_or(0);
    (0..nrows)
        .filter_map(|row_idx| match df.cols.get(span_id_idx).map(|col| col.get(row_idx)) {
            Some(Ele::I64(value)) => Some(value),
            Some(Ele::I32(value)) => Some(value as i64),
            Some(Ele::F32(value)) => Some(value as i64),
            Some(Ele::F64(value)) => Some(value as i64),
            Some(Ele::Text(value)) | Some(Ele::Url(value)) => value.parse::<i64>().ok(),
            Some(Ele::DataTime(value)) => i64::try_from(value).ok(),
            _ => None,
        })
        .collect()
}

fn trace_events_from_df(df: DataFrame) -> Vec<TraceEvent> {
    let mut events = Vec::new();

    if df.names.is_empty() || df.cols.is_empty() {
        return events;
    }

    let record_type_idx = df.names.iter().position(|c| c == "record_type").unwrap_or(0);
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

    events
}

fn trace_processes_from_df(df: DataFrame) -> Vec<TraceProcessInfo> {
    let mut processes = Vec::new();

    if df.names.is_empty() || df.cols.is_empty() {
        return processes;
    }

    let pid_idx = df.names.iter().position(|c| c == "pid").unwrap_or(0);
    let hostname_idx = df.names.iter().position(|c| c == "hostname").unwrap_or(1);
    let worker_idx = df.names.iter().position(|c| c == "ray_worker_id").unwrap_or(2);
    let actor_idx = df.names.iter().position(|c| c == "ray_actor_id").unwrap_or(3);
    let actor_name_idx = df.names.iter().position(|c| c == "ray_actor_name").unwrap_or(4);
    let role_idx = df.names.iter().position(|c| c == "process_role").unwrap_or(5);
    let nrows = df.cols.iter().map(|col| col.len()).max().unwrap_or(0);
    let mut seen = std::collections::HashSet::<i32>::new();

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

        let pid = get_str(pid_idx).parse::<i32>().unwrap_or(0);
        if pid <= 0 || !seen.insert(pid) {
            continue;
        }

        let opt = |value: String| {
            if value.is_empty() {
                None
            } else {
                Some(value)
            }
        };

        processes.push(TraceProcessInfo {
            pid,
            process_role: opt(get_str(role_idx)),
            hostname: opt(get_str(hostname_idx)),
            ray_worker_id: opt(get_str(worker_idx)),
            ray_actor_id: opt(get_str(actor_idx)),
            ray_actor_name: opt(get_str(actor_name_idx)),
        });
    }

    processes.sort_by(|a, b| {
        process_sort_rank(a)
            .cmp(&process_sort_rank(b))
            .then_with(|| a.pid.cmp(&b.pid))
    });
    processes
}

fn merge_local_processes(processes: &mut Vec<TraceProcessInfo>, local_processes: Vec<Process>) {
    let mut known = processes
        .iter()
        .map(|process| process.pid)
        .collect::<std::collections::HashSet<_>>();
    for process in local_processes {
        if process.pid <= 0 || !known.insert(process.pid) {
            continue;
        }
        processes.push(TraceProcessInfo {
            pid: process.pid,
            process_role: Some(local_process_role(&process.cmd)),
            hostname: None,
            ray_worker_id: None,
            ray_actor_id: None,
            ray_actor_name: local_actor_name(&process.cmd),
        });
    }
}

fn local_process_role(cmd: &str) -> String {
    if cmd.contains("RolloutManager.generate") {
        "rollout_actor".to_string()
    } else if cmd.contains("MegatronTrainRayActor.train") {
        "train_actor".to_string()
    } else if cmd.contains("SGLangEngine") {
        "sglang_engine".to_string()
    } else if cmd.contains("JobSupervisor") {
        "ray_job_supervisor".to_string()
    } else if cmd.contains("Lock") {
        "ray_lock".to_string()
    } else if cmd.contains("train_async.py") {
        "driver".to_string()
    } else {
        "process".to_string()
    }
}

fn local_actor_name(cmd: &str) -> Option<String> {
    for marker in [
        "RolloutManager.generate",
        "MegatronTrainRayActor.train",
        "SGLangEngine",
        "JobSupervisor",
        "Lock",
    ] {
        if cmd.contains(marker) {
            return Some(format!("ray::{marker}"));
        }
    }
    None
}

fn process_sort_rank(process: &TraceProcessInfo) -> usize {
    match process.process_role.as_deref() {
        Some("driver") => 0,
        Some(role) if role.contains("driver") => 1,
        Some(role) if role.contains("rollout") => 2,
        Some(_) => 3,
        None => 4,
    }
}

fn build_span_tree_from_events(mut events: Vec<TraceEvent>) -> Vec<SpanInfo> {
    events.sort_by_key(|event| event.timestamp);

    let mut span_map: std::collections::HashMap<(i64, i64), SpanInfo> =
        std::collections::HashMap::new();
    let mut root_spans: Vec<(i64, i64)> = Vec::new();
    let mut span_process_lookup: std::collections::HashMap<(i64, i64, i64), i64> =
        std::collections::HashMap::new();

    for event in &events {
        if event.record_type == "span_start" {
            let process_pid = trace_event_process_pid(event);
            span_process_lookup.insert((event.span_id, event.thread_id, event.trace_id), process_pid);
            let span_key = (process_pid, event.span_id);
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
                root_spans.push(span_key);
            }

            span_map.insert(span_key, span);
        } else if event.record_type == "span_end" {
            let process_pid = trace_event_process_pid_from_start(event, &span_process_lookup);
            let span_key = (process_pid, event.span_id);
            if let Some(span) = span_map.get_mut(&span_key) {
                span.end_timestamp = Some(event.timestamp);
            }
        } else if event.record_type == "event" {
            let process_pid = trace_event_process_pid_from_start(event, &span_process_lookup);
            let span_key = (process_pid, event.span_id);
            if let Some(span) = span_map.get_mut(&span_key) {
                span.events.push(EventInfo {
                    name: event.name.clone(),
                    timestamp: event.timestamp,
                    attributes: event.event_attributes.clone(),
                });
            }
        }
    }

    let mut depth_map: std::collections::HashMap<(i64, i64), usize> =
        std::collections::HashMap::new();

    for root_id in &root_spans {
        depth_map.insert(*root_id, 0);
    }

    let mut changed = true;
    while changed {
        changed = false;
        for (span_key, span) in span_map.iter() {
            if depth_map.contains_key(span_key) {
                continue;
            }

            if let Some(parent_id) = span.parent_id {
                let parent_key = (span_key.0, parent_id);
                if parent_id != -1 && depth_map.contains_key(&parent_key) {
                    let parent_depth = depth_map[&parent_key];
                    depth_map.insert(*span_key, parent_depth + 1);
                    changed = true;
                }
            } else {
                depth_map.insert(*span_key, 0);
                changed = true;
            }
        }
    }

    let mut spans_to_process: Vec<((i64, i64), usize)> = span_map
        .keys()
        .map(|&key| (key, depth_map.get(&key).copied().unwrap_or(0)))
        .collect();
    spans_to_process.sort_by(|a, b| b.1.cmp(&a.1));

    for (span_key, _depth) in spans_to_process {
        let parent_key = span_map
            .get(&span_key)
            .and_then(|span| span.parent_id)
            .filter(|&pid| pid != -1)
            .map(|parent_id| (span_key.0, parent_id));

        if let Some(parent_key) = parent_key {
            if let Some(child) = span_map.remove(&span_key) {
                if let Some(parent) = span_map.get_mut(&parent_key) {
                    parent.children.push(child);
                } else {
                    span_map.insert(span_key, child);
                }
            }
        }
    }

    let mut result = Vec::new();
    for root_id in root_spans {
        if let Some(span) = span_map.remove(&root_id) {
            result.push(span);
        }
    }

    for (_, span) in span_map {
        result.push(span);
    }

    result.sort_by_key(|s| s.start_timestamp);

    result
}

fn trace_event_process_pid(event: &TraceEvent) -> i64 {
    attr_string(&event.attributes, "pid")
        .and_then(|pid| pid.parse::<i64>().ok())
        .filter(|pid| *pid > 0)
        .unwrap_or(event.trace_id.max(1))
}

fn trace_event_process_pid_from_start(
    event: &TraceEvent,
    span_process_lookup: &std::collections::HashMap<(i64, i64, i64), i64>,
) -> i64 {
    if event.trace_id > 0 {
        if let Some(pid) = span_process_lookup
            .get(&(event.span_id, event.thread_id, event.trace_id))
            .copied()
        {
            return pid;
        }
    }

    span_process_lookup
        .iter()
        .find(|((span_id, thread_id, _), _)| {
            *span_id == event.span_id && *thread_id == event.thread_id
        })
        .map(|(_, pid)| *pid)
        .or_else(|| {
            span_process_lookup
                .iter()
                .find(|((span_id, _, _), _)| *span_id == event.span_id)
                .map(|(_, pid)| *pid)
        })
        .unwrap_or_else(|| trace_event_process_pid(event))
}

fn attr_string(attributes: &Option<String>, key: &str) -> Option<String> {
    let attrs = attributes.as_ref()?;
    let value = serde_json::from_str::<serde_json::Value>(attrs).ok()?;
    let raw = value.get(key)?;
    match raw {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}
