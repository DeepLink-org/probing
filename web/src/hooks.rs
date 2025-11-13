use dioxus::prelude::*;
use std::future::Future;
use crate::utils::error::AppError;

/// API 调用状态
#[derive(Clone)]
pub struct ApiState<T: Clone + 'static> {
    pub loading: Signal<bool>,
    pub data: Signal<Option<Result<T, AppError>>>,
}

impl<T: Clone + 'static> ApiState<T> {
    /// 检查是否正在加载
    #[inline]
    pub fn is_loading(&self) -> bool {
        *self.loading.read()
    }
}

impl<T: Clone + 'static + PartialEq> PartialEq for ApiState<T> {
    fn eq(&self, other: &Self) -> bool {
        // 比较 Signal 的内部值
        *self.loading.read() == *other.loading.read() &&
        self.data.read().as_ref() == other.data.read().as_ref()
    }
}

/// 简单的 API 调用 hook（不自动执行）
pub fn use_api_simple<T: Clone + 'static>() -> ApiState<T> {
    ApiState {
        loading: use_signal(|| false),
        data: use_signal(|| None),
    }
}

/// 通用的 API 调用 hook（自动执行）
/// 
/// 自动在组件挂载时执行 API 调用，并在依赖变化时重新执行。
/// 使用缓存的 ApiClient 实例以提高性能。
/// 
/// # 示例
/// ```rust
/// let state = use_api(move || {
///     let client = ApiClient::new();
///     async move { client.get_overview().await }
/// });
/// ```
pub fn use_api<T, F, Fut>(mut fetch_fn: F) -> ApiState<T>
where
    T: Clone + 'static,
    F: FnMut() -> Fut + 'static,
    Fut: Future<Output = Result<T, AppError>> + 'static,
{
    let state = use_api_simple::<T>();
    
    use_effect(move || {
        let mut loading = state.loading;
        let mut data = state.data;
        let result_future = fetch_fn();
        spawn(async move {
            *loading.write() = true;
            let result = result_future.await;
            *data.write() = Some(result);
            *loading.write() = false;
        });
    });
    
    state
}