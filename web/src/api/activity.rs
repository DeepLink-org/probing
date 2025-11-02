use super::ApiClient;
use crate::utils::error::Result;
use probing_proto::prelude::*;

/// 活动分析API
impl ApiClient {
    /// 带模式的调用堆栈获取：mode = py | cpp | mixed
    pub async fn get_callstack_with_mode(&self, tid: Option<String>, mode: &str) -> Result<Vec<CallFrame>> {
        let mode = match mode {
            "py" | "cpp" | "mixed" => mode,
            _ => "mixed",
        };
        let base = "/apis/pythonext/callstack";
        let path = if let Some(tid) = tid {
            format!("{}?tid={}&mode={}", base, tid, mode)
        } else {
            format!("{}?mode={}", base, mode)
        };
        let response = self.get_request(&path).await?;
        Self::parse_json(&response)
    }
}
