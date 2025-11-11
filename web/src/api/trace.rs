use super::ApiClient;
use crate::utils::error::Result;
use serde::{Deserialize, Serialize};
use probing_proto::prelude::{DataFrame, Ele};

/// Trace API 响应结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceResponse {
    pub success: bool,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Trace 状态信息（简化版，因为 show_trace 只返回函数名列表）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceInfo {
    pub function: String,
}

/// 变量变化记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableRecord {
    pub function_name: String,
    pub filename: String,
    pub lineno: i64,
    pub variable_name: String,
    pub value: String,
    pub value_type: String,
    pub timestamp: f64,
}

/// 可追踪项（函数或模块）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceableItem {
    pub name: String,
    #[serde(rename = "type")]
    pub item_type: String,
    #[serde(default)]
    pub variables: Vec<String>,
}

/// Trace API
impl ApiClient {
    /// 获取可追踪的函数列表（返回函数名列表，兼容旧格式）
    pub async fn get_traceable_functions(&self, prefix: Option<&str>) -> Result<Vec<String>> {
        let items = self.get_traceable_items(prefix).await?;
        // Convert to old format for backward compatibility
        Ok(items.iter().map(|item| format!("[{}] {}", item.item_type, item.name)).collect())
    }

    /// 获取可追踪的项列表（始终包含变量信息）
    pub async fn get_traceable_items(&self, prefix: Option<&str>) -> Result<Vec<TraceableItem>> {
        let base = "/apis/pythonext/trace/list";
        let path = if let Some(prefix) = prefix {
            format!("{}?prefix={}", base, prefix)
        } else {
            base.to_string()
        };
        
        let response = self.get_request(&path).await?;
        
        // Try to parse as new format (list of objects)
        if let Ok(items) = serde_json::from_str::<Vec<TraceableItem>>(&response) {
            return Ok(items);
        }
        
        // Fallback to old format (list of strings)
        let strings: Vec<String> = Self::parse_json(&response)?;
        Ok(strings.iter().map(|s| {
            // Parse "[TYPE] name" format
            if let Some(bracket_end) = s.find(']') {
                let item_type = s[1..bracket_end].to_string();
                let name = s[bracket_end + 2..].to_string();
                TraceableItem {
                    name,
                    item_type,
                    variables: vec![],
                }
            } else {
                TraceableItem {
                    name: s.clone(),
                    item_type: "".to_string(),
                    variables: vec![],
                }
            }
        }).collect())
    }

    /// 获取当前 trace 状态（返回已追踪的函数名列表）
    pub async fn get_trace_info(&self) -> Result<Vec<String>> {
        let path = "/apis/pythonext/trace/show";
        let response = self.get_request(path).await?;
        let info: Vec<String> = Self::parse_json(&response)?;
        Ok(info)
    }

    /// 开始追踪函数
    pub async fn start_trace(
        &self,
        function: &str,
        watch: Option<Vec<String>>,
        depth: Option<i32>,
    ) -> Result<TraceResponse> {
        let base = "/apis/pythonext/trace/start";
        let mut params = vec![format!("function={}", function)];
        
        if let Some(watch) = watch {
            if !watch.is_empty() {
                params.push(format!("watch={}", watch.join(",")));
            }
        }
        
        if let Some(depth) = depth {
            params.push(format!("depth={}", depth));
        }
        
        let path = if params.len() > 1 {
            format!("{}?{}", base, params.join("&"))
        } else {
            format!("{}?function={}", base, function)
        };
        
        let response = self.get_request(&path).await?;
        let result: TraceResponse = Self::parse_json(&response)?;
        Ok(result)
    }

    /// 停止追踪函数
    pub async fn stop_trace(&self, function: &str) -> Result<TraceResponse> {
        let path = format!("/apis/pythonext/trace/stop?function={}", function);
        let response = self.get_request(&path).await?;
        let result: TraceResponse = Self::parse_json(&response)?;
        Ok(result)
    }

    /// 获取变量变化记录（通过 SQL 查询）
    pub async fn get_variable_records(
        &self,
        function: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<VariableRecord>> {
        // Build SQL query
        let limit_clause = limit.map(|l| format!(" LIMIT {}", l)).unwrap_or_default();
        let where_clause = if let Some(func) = function {
            // Escape single quotes in function name
            let escaped_func = func.replace("'", "''");
            format!(" WHERE function_name = '{}'", escaped_func)
        } else {
            String::new()
        };
        
        // Try with python namespace first, fallback to direct table name
        let queries = vec![
            format!(
                "SELECT function_name, filename, lineno, variable_name, value, value_type, timestamp FROM python.trace_variables{} ORDER BY timestamp DESC{}",
                where_clause, limit_clause
            ),
            format!(
                "SELECT function_name, filename, lineno, variable_name, value, value_type, timestamp FROM python.trace_variables{} ORDER BY timestamp DESC{}",
                where_clause, limit_clause
            ),
        ];
        
        // Try each query until one succeeds
        let mut last_err: Option<crate::utils::error::AppError> = None;
        for query in queries {
            match self.execute_query(&query).await {
                Ok(df) => {
                    return Ok(Self::dataframe_to_variable_records(df));
                }
                Err(e) => {
                    last_err = Some(e);
                    continue;
                }
            }
        }
        
        // If all queries failed, return empty vector or error
        Err(last_err.unwrap_or_else(|| {
            crate::utils::error::AppError::Api("Failed to query python.trace_variables table".to_string())
        }))
    }
    
    /// 将 DataFrame 转换为 Vec<VariableRecord>
    fn dataframe_to_variable_records(df: DataFrame) -> Vec<VariableRecord> {
        let mut records = Vec::new();
        
        if df.names.is_empty() || df.cols.is_empty() {
            return records;
        }
        
        // Find column indices
        let function_name_idx = df.names.iter().position(|c| c == "function_name").unwrap_or(0);
        let filename_idx = df.names.iter().position(|c| c == "filename").unwrap_or(1);
        let lineno_idx = df.names.iter().position(|c| c == "lineno").unwrap_or(2);
        let variable_name_idx = df.names.iter().position(|c| c == "variable_name").unwrap_or(3);
        let value_idx = df.names.iter().position(|c| c == "value").unwrap_or(4);
        let value_type_idx = df.names.iter().position(|c| c == "value_type").unwrap_or(5);
        let timestamp_idx = df.names.iter().position(|c| c == "timestamp").unwrap_or(6);
        
        // Get number of rows
        let nrows = df.cols.iter().map(|col| col.len()).max().unwrap_or(0);
        
        // Extract data from each row
        for i in 0..nrows {
            let get_str = |idx: usize| -> String {
                match df.cols.get(idx).map(|col| col.get(i)) {
                    Some(Ele::Text(s)) => s.clone(),
                    Some(Ele::I32(x)) => x.to_string(),
                    Some(Ele::I64(x)) => x.to_string(),
                    Some(Ele::F32(x)) => x.to_string(),
                    Some(Ele::F64(x)) => x.to_string(),
                    _ => String::new(),
                }
            };
            
            let get_i64 = |idx: usize| -> i64 {
                match df.cols.get(idx).map(|col| col.get(i)) {
                    Some(Ele::I32(x)) => x as i64,
                    Some(Ele::I64(x)) => x,
                    Some(Ele::F32(x)) => x as i64,
                    Some(Ele::F64(x)) => x as i64,
                    Some(Ele::Text(s)) => s.parse().unwrap_or(0),
                    _ => 0,
                }
            };
            
            let get_f64 = |idx: usize| -> f64 {
                match df.cols.get(idx).map(|col| col.get(i)) {
                    Some(Ele::F64(x)) => x,
                    Some(Ele::F32(x)) => x as f64,
                    Some(Ele::I64(x)) => x as f64,
                    Some(Ele::I32(x)) => x as f64,
                    Some(Ele::Text(s)) => s.parse().unwrap_or(0.0),
                    _ => 0.0,
                }
            };
            
            let function_name = get_str(function_name_idx);
            if function_name.is_empty() {
                continue;
            }
            
            let variable_name = get_str(variable_name_idx);
            if variable_name.is_empty() {
                continue;
            }
            
            records.push(VariableRecord {
                function_name,
                filename: get_str(filename_idx),
                lineno: get_i64(lineno_idx),
                variable_name,
                value: get_str(value_idx),
                value_type: get_str(value_type_idx),
                timestamp: get_f64(timestamp_idx),
            });
        }
        
        records
    }
}

