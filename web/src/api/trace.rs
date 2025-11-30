use super::ApiClient;
use crate::utils::error::Result;
use serde::{Deserialize, Serialize};
use probing_proto::prelude::DataFrame;

/// Trace API response structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceResponse {
    pub success: bool,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Trace status information (simplified version, as show_trace only returns function name list)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceInfo {
    pub function: String,
}

/// Variable change record
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

/// Traceable item (function or module)
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
    /// Get list of traceable functions (returns function name list, compatible with old format)
    pub async fn get_traceable_functions(&self, prefix: Option<&str>) -> Result<Vec<String>> {
        let items = self.get_traceable_items(prefix).await?;
        // Convert to old format for backward compatibility
        Ok(items.iter().map(|item| format!("[{}] {}", item.item_type, item.name)).collect())
    }

    /// Get list of traceable items (always includes variable information)
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

    /// Get current trace status (returns list of traced function names)
    pub async fn get_trace_info(&self) -> Result<Vec<String>> {
        let path = "/apis/pythonext/trace/show";
        let response = self.get_request(path).await?;
        let info: Vec<String> = Self::parse_json(&response)?;
        Ok(info)
    }

    /// Start tracing a function
    pub async fn start_trace(
        &self,
        function: &str,
        watch: Option<Vec<String>>,
        print_to_terminal: bool,
    ) -> Result<TraceResponse> {
        let base = "/apis/pythonext/trace/start";
        let mut params = vec![format!("function={}", function)];

        if let Some(watch) = watch {
            if !watch.is_empty() {
                params.push(format!("watch={}", watch.join(",")));
            }
        }

        if print_to_terminal {
            params.push("print_to_terminal=true".to_string());
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

    /// Stop tracing a function
    pub async fn stop_trace(&self, function: &str) -> Result<TraceResponse> {
        let path = format!("/apis/pythonext/trace/stop?function={}", function);
        let response = self.get_request(&path).await?;
        let result: TraceResponse = Self::parse_json(&response)?;
        Ok(result)
    }

    /// Get variable change records (via SQL query)
    /// Returns DataFrame directly, uses SQL AS to control column name display
    pub async fn get_variable_records(
        &self,
        function: Option<&str>,
        limit: Option<usize>,
    ) -> Result<DataFrame> {
        // Build SQL query with column renaming via AS (SQL controls column names)
        let limit_clause = limit.map(|l| format!(" LIMIT {}", l)).unwrap_or_default();
        let where_clause = if let Some(func) = function {
            // Escape single quotes in function name
            let escaped_func = func.replace("'", "''");
            format!(" WHERE function_name = '{}'", escaped_func)
        } else {
            String::new()
        };

        // SQL query uses AS to rename columns for display
        let queries = vec![
            format!(
                "SELECT filename AS File, lineno AS Line, variable_name AS Variable, value AS Value, value_type AS Type, timestamp AS Time FROM python.trace_variables{} ORDER BY timestamp DESC{}",
                where_clause, limit_clause
            ),
            format!(
                "SELECT filename AS File, lineno AS Line, variable_name AS Variable, value AS Value, value_type AS Type, timestamp AS Time FROM trace_variables{} ORDER BY timestamp DESC{}",
                where_clause, limit_clause
            ),
        ];

        // Try each query until one succeeds
        let mut last_err: Option<crate::utils::error::AppError> = None;
        for query in queries.iter() {
            match self.execute_query(query).await {
                Ok(df) => {
                    return Ok(df);
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

}
