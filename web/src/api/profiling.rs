use super::ApiClient;
use crate::utils::error::Result;
use probing_proto::prelude::*;

/// 性能分析API
impl ApiClient {
    /// 获取profiler配置：返回 (name, value) 对的向量
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

    /// 获取火焰图数据
    pub async fn get_flamegraph(&self, profiler_type: &str) -> Result<String> {
        self.get_request(&format!("/apis/flamegraph/{}", profiler_type)).await
    }
}
