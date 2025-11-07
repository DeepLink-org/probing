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
    pub code_path: Option<String>,
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
    pub code_path: Option<String>,
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
    /// 获取所有 trace events
    pub async fn get_trace_events(&self) -> Result<Vec<TraceEvent>> {
        let query = r#"
            SELECT 
                record_type,
                trace_id,
                span_id,
                COALESCE(parent_id, -1) as parent_id,
                name,
                timestamp,
                COALESCE(thread_id, 0) as thread_id,
                kind,
                code_path,
                attributes,
                event_attributes
            FROM python.trace_event
            ORDER BY timestamp ASC
        "#;
        
        let df = self.execute_query(query).await?;
        
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
        let code_path_idx = df.names.iter().position(|c| c == "code_path").unwrap_or(8);
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
                code_path: get_opt_str(code_path_idx),
                attributes: get_opt_str(attributes_idx),
                event_attributes: get_opt_str(event_attributes_idx),
            });
        }
        
        Ok(events)
    }
    
    /// 构建 span 树结构
    pub async fn get_span_tree(&self) -> Result<Vec<SpanInfo>> {
        let events = self.get_trace_events().await?;
        
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
                    code_path: event.code_path.clone(),
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
}

