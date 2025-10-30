use super::ApiClient;
use crate::utils::error::Result;
use probing_proto::prelude::*;

/// 活动分析API
impl ApiClient {
    /// 获取调用堆栈信息
    pub async fn get_callstack(&self, tid: Option<String>) -> Result<Vec<CallFrame>> {
        let path = if let Some(tid) = tid {
            format!("/apis/pythonext/callstack?tid={}", tid)
        } else {
            "/apis/pythonext/callstack".to_string()
        };
        let response = self.get_request(&path).await?;
        Self::parse_json(&response)
    }
}
