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
    /// 获取 trace events，支持限制数量
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
    
    /// 构建 span 树结构，支持限制数量
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
    
    /// 获取 Chrome tracing 格式的 JSON 数据
    /// 返回格式符合 Chrome DevTools tracing viewer 的要求
    pub async fn get_chrome_tracing_json(&self, limit: Option<usize>) -> Result<String> {
        let events = self.get_trace_events(limit).await?;
        
        // 找到最小时间戳作为基准
        let min_timestamp = events.iter()
            .map(|e| e.timestamp)
            .min()
            .unwrap_or(0);
        
        // 转换为 Chrome tracing 格式
        let mut trace_events: Vec<serde_json::Value> = Vec::new();
        
        // 使用 HashMap 跟踪 span 的开始时间
        let mut span_starts: std::collections::HashMap<i64, (i64, String, i64)> = std::collections::HashMap::new();
        
        for event in &events {
            // 将纳秒转换为微秒（Chrome tracing 使用微秒）
            let ts_micros = (event.timestamp - min_timestamp) / 1000;
            let pid = event.trace_id as u32;
            let tid = event.thread_id as u32;
            
            match event.record_type.as_str() {
                "span_start" => {
                    // 记录 span 开始
                    span_starts.insert(event.span_id, (ts_micros, event.name.clone(), event.thread_id));
                    
                    // 创建 'B' (Begin) 事件
                    let mut chrome_event = serde_json::json!({
                        "name": event.name,
                        "cat": event.kind.as_ref().unwrap_or(&"span".to_string()),
                        "ph": "B",
                        "ts": ts_micros,
                        "pid": pid,
                        "tid": tid,
                    });
                    
                    // 添加可选参数
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
                    // 创建 'E' (End) 事件
                    let (start_ts, name, _) = span_starts.get(&event.span_id)
                        .map(|(ts, n, _)| (*ts, n.clone(), 0))
                        .unwrap_or((ts_micros, "unknown".to_string(), 0));
                    
                    let mut chrome_event = serde_json::json!({
                        "name": name,
                        "cat": "span",
                        "ph": "E",
                        "ts": ts_micros,
                        "pid": pid,
                        "tid": tid,
                    });
                    
                    // 计算持续时间（微秒）
                    let dur = ts_micros - start_ts;
                    if dur > 0 {
                        chrome_event["dur"] = serde_json::Value::Number(dur.into());
                    }
                    
                    trace_events.push(chrome_event);
                }
                "event" => {
                    // 创建 'i' (Instant) 事件
                    let mut chrome_event = serde_json::json!({
                        "name": event.name,
                        "cat": "event",
                        "ph": "i",
                        "ts": ts_micros,
                        "pid": pid,
                        "tid": tid,
                        "s": "t", // scope: thread
                    });
                    
                    // 添加事件属性
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
        
        // 构建完整的 Chrome tracing 格式 JSON
        let chrome_trace = serde_json::json!({
            "traceEvents": trace_events,
            "displayTimeUnit": "ms",
        });
        
        Ok(serde_json::to_string_pretty(&chrome_trace)?)
    }
}

