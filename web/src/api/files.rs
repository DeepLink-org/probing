use super::ApiClient;
use crate::utils::error::Result;

impl ApiClient {
    /// Read a workspace file via `GET /apis/files?path=…`.
    pub async fn read_file(&self, path: &str) -> Result<String> {
        let query = format!("/apis/files?path={}", urlencoding::encode(path));
        self.get_request(&query).await
    }
}
