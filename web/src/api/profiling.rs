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
        let path = match profiler_type {
            "torch" => match metric {
                Some(m) if !m.is_empty() => format!(
                    "/apis/torchextension/flamegraph/json?metric={}",
                    urlencoding::encode(m)
                ),
                _ => "/apis/torchextension/flamegraph/json".to_string(),
            },
            "pprof" => "/apis/pprofextension/flamegraph/json".to_string(),
            other => {
                return Err(crate::utils::error::AppError::Api(format!(
                    "unknown flamegraph profiler: {other}"
                )))
            }
        };
        self.get_request(&path).await
    }

    /// Distributed SPMD torch flamegraph at one ``local_step`` (cluster fan-out by default).
    pub async fn get_distributed_flamegraph_json(
        &self,
        step: Option<i64>,
        metric: Option<&str>,
        cluster: bool,
    ) -> Result<String> {
        let mut parts = Vec::new();
        if let Some(s) = step {
            parts.push(format!("step={s}"));
        }
        if let Some(m) = metric {
            if !m.is_empty() {
                parts.push(format!("metric={}", urlencoding::encode(m)));
            }
        }
        parts.push(format!("cluster={cluster}"));
        let path = format!(
            "/apis/training/distributed_flamegraph/json?{}",
            parts.join("&")
        );
        self.get_request(&path).await
    }
}
