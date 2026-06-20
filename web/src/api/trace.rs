use super::ApiClient;
use crate::utils::error::Result;
use probing_proto::prelude::DataFrame;
use serde::{Deserialize, Serialize};

/// Trace API response structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceResponse {
    pub success: bool,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
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
    /// Get list of traceable items (includes variable information when available).
    pub async fn get_traceable_items(&self, prefix: Option<&str>) -> Result<Vec<TraceableItem>> {
        let base = "/apis/pythonext/trace/list";
        let path = if let Some(prefix) = prefix {
            format!("{}?prefix={}", base, prefix)
        } else {
            base.to_string()
        };

        let response = self.get_request(&path).await?;
        Self::parse_json(&response)
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
        let mut params = vec![format!("function={}", urlencoding::encode(function))];

        if let Some(watch) = watch {
            if !watch.is_empty() {
                params.push(format!("watch={}", urlencoding::encode(&watch.join(","))));
            }
        }

        if print_to_terminal {
            params.push("print_to_terminal=true".to_string());
        }

        let path = format!("{}?{}", base, params.join("&"));

        let response = self.get_request(&path).await?;
        let result: TraceResponse = Self::parse_json(&response)?;
        Ok(result)
    }

    /// Stop tracing a function
    pub async fn stop_trace(&self, function: &str) -> Result<TraceResponse> {
        let path = format!(
            "/apis/pythonext/trace/stop?function={}",
            urlencoding::encode(function)
        );
        let response = self.get_request(&path).await?;
        let result: TraceResponse = Self::parse_json(&response)?;
        Ok(result)
    }

    /// Get variable change records via the trace/variables HTTP handler.
    pub async fn get_trace_variables(
        &self,
        function: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<VariableRecord>> {
        let limit = limit.unwrap_or(100);
        let mut path = format!("/apis/pythonext/trace/variables?limit={limit}");
        if let Some(func) = function {
            path.push_str(&format!("&function={}", urlencoding::encode(func)));
        }

        let response = self.get_request(&path).await?;
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&response) {
            if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
                return Err(crate::utils::error::AppError::Api(err.to_string()));
            }
        }
        Self::parse_json(&response)
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

        // Use snake_case column names (DataFusion lowercases unquoted aliases).
        let queries = [
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
            crate::utils::error::AppError::Api(
                "Failed to query python.trace_variables table".to_string(),
            )
        }))
    }
}
