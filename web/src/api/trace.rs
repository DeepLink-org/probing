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
    /// SQL 查询必须按 VariableRecord 结构体的字段顺序返回：function_name, filename, lineno, variable_name, value, value_type, timestamp
    pub async fn get_variable_records(
        &self,
        function: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<VariableRecord>> {
        // Build SQL query with fields in VariableRecord order
        let limit_clause = limit.map(|l| format!(" LIMIT {}", l)).unwrap_or_default();
        let where_clause = if let Some(func) = function {
            // Escape single quotes in function name
            let escaped_func = func.replace("'", "''");
            format!(" WHERE function_name = '{}'", escaped_func)
        } else {
            String::new()
        };
        
        // SQL query ensures field order matches VariableRecord struct
        let queries = vec![
            format!(
                "SELECT function_name, filename, lineno, variable_name, value, value_type, timestamp FROM python.trace_variables{} ORDER BY timestamp DESC{}",
                where_clause, limit_clause
            ),
            format!(
                "SELECT function_name, filename, lineno, variable_name, value, value_type, timestamp FROM trace_variables{} ORDER BY timestamp DESC{}",
                where_clause, limit_clause
            ),
        ];
        
        // Try each query until one succeeds
        let mut last_err: Option<crate::utils::error::AppError> = None;
        for query in queries.iter() {
            match self.execute_query(query).await {
                Ok(df) => {
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        Self::dataframe_to_variable_records(df)
                    })) {
                        Ok(records) => {
                            return Ok(records);
                        }
                        Err(e) => {
                            web_sys::console::error_1(&format!("[trace] Panic during DataFrame parsing: {:?}", e).into());
                            last_err = Some(crate::utils::error::AppError::Api(format!("Panic during DataFrame parsing: {:?}", e)));
                            continue;
                        }
                    }
                }
                Err(e) => {
                    last_err = Some(e);
                    continue;
                }
            }
        }
        
        // If all queries failed, return error
        Err(last_err.unwrap_or_else(|| {
            crate::utils::error::AppError::Api("Failed to query python.trace_variables table".to_string())
        }))
    }
    
    /// 将 DataFrame 转换为 Vec<VariableRecord>
    /// 假设 SQL 查询返回的字段顺序与 VariableRecord 结构体一致：function_name, filename, lineno, variable_name, value, value_type, timestamp
    fn dataframe_to_variable_records(df: DataFrame) -> Vec<VariableRecord> {
        let mut records = Vec::new();
        
        // Expected 7 columns in order: function_name, filename, lineno, variable_name, value, value_type, timestamp
        if df.cols.len() < 7 {
            return records;
        }
        
        // Get number of rows
        let nrows = df.cols.iter()
            .map(|col| col.len())
            .min()
            .unwrap_or(0);
        
        // Extract data from each row by index (SQL ensures correct order)
        for i in 0..nrows {
            let get_str = |col_idx: usize| -> String {
                match df.cols.get(col_idx).map(|col| col.get(i)) {
                    Some(Ele::Text(s)) => s.clone(),
                    Some(Ele::I32(x)) => x.to_string(),
                    Some(Ele::I64(x)) => x.to_string(),
                    Some(Ele::F32(x)) => x.to_string(),
                    Some(Ele::F64(x)) => x.to_string(),
                    _ => String::new(),
                }
            };
            
            let get_i64 = |col_idx: usize| -> i64 {
                match df.cols.get(col_idx).map(|col| col.get(i)) {
                    Some(Ele::I32(x)) => x as i64,
                    Some(Ele::I64(x)) => x,
                    Some(Ele::F32(x)) => x as i64,
                    Some(Ele::F64(x)) => x as i64,
                    Some(Ele::Text(s)) => s.parse().unwrap_or(0),
                    _ => 0,
                }
            };
            
            let get_f64 = |col_idx: usize| -> f64 {
                match df.cols.get(col_idx).map(|col| col.get(i)) {
                    Some(Ele::F64(x)) => x,
                    Some(Ele::F32(x)) => x as f64,
                    Some(Ele::I64(x)) => x as f64,
                    Some(Ele::I32(x)) => x as f64,
                    Some(Ele::Text(s)) => s.parse().unwrap_or(0.0),
                    _ => 0.0,
                }
            };
            
            let function_name = get_str(0);
            if function_name.is_empty() {
                continue;
            }
            
            let variable_name = get_str(3);
            if variable_name.is_empty() {
                continue;
            }
            
            // Create record directly by index (SQL ensures correct order)
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                VariableRecord {
                    function_name: function_name.clone(),
                    filename: get_str(1),
                    lineno: get_i64(2),
                    variable_name: variable_name.clone(),
                    value: get_str(4),
                    value_type: get_str(5),
                    timestamp: get_f64(6),
                }
            })) {
                Ok(record) => {
                    records.push(record);
                }
                Err(e) => {
                    web_sys::console::error_1(&format!("[trace] Error creating record at row {}: {:?}", i, e).into());
                }
            }
        }
        
        records
    }
}

