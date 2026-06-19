use super::ApiClient;
use crate::utils::error::Result;
use probing_proto::prelude::*;

/// Performance analysis API
impl ApiClient {
    /// Get profiler configuration: returns vector of (name, value) pairs
    pub async fn get_profiler_config(&self) -> Result<Vec<(String, String)>> {
        let df = self.execute_query("select name, value from information_schema.df_settings where name like 'probing.%';").await?;
        let mut result = Vec::new();
        if !df.cols.is_empty() && df.cols.len() >= 2 {
            let names = &df.cols[0];
            let values = &df.cols[1];
            let nrows = names.len().min(values.len());
            for i in 0..nrows {
                let name = match names.get(i) {
                    Ele::Text(s) => s.to_string(),
                    _ => continue,
                };
                let value = match values.get(i) {
                    Ele::Text(s) => s.to_string(),
                    Ele::Nil => String::new(),
                    _ => continue,
                };
                result.push((name, value));
            }
        }
        Ok(result)
    }

    /// Get flamegraph JSON for native web UI rendering.
    pub async fn get_flamegraph_json(&self, profiler_type: &str) -> Result<String> {
        self.get_flamegraph_json_with_metric(profiler_type, None)
            .await
    }

    /// Get flamegraph JSON with optional torch metric (`duration`, `delta_mb`, `peak_mb`).
    pub async fn get_flamegraph_json_with_metric(
        &self,
        profiler_type: &str,
        metric: Option<&str>,
    ) -> Result<String> {
        let path = match metric {
            Some(m) if !m.is_empty() => {
                format!(
                    "/apis/flamegraph/{}?format=json&metric={}",
                    profiler_type,
                    urlencoding::encode(m)
                )
            }
            _ => format!("/apis/flamegraph/{}?format=json", profiler_type),
        };
        self.get_request(&path).await
    }
}
