use dioxus::prelude::*;
use std::future::Future;
use crate::utils::error::AppError;

/// API call state
#[derive(Clone)]
pub struct ApiState<T: Clone + 'static> {
    pub loading: Signal<bool>,
    pub data: Signal<Option<Result<T, AppError>>>,
}

impl<T: Clone + 'static> ApiState<T> {
    /// Check if currently loading
    #[inline]
    pub fn is_loading(&self) -> bool {
        *self.loading.read()
    }
}

impl<T: Clone + 'static + PartialEq> PartialEq for ApiState<T> {
    fn eq(&self, other: &Self) -> bool {
        // Compare internal values of Signal
        *self.loading.read() == *other.loading.read() &&
        self.data.read().as_ref() == other.data.read().as_ref()
    }
}

/// Simple API call hook (does not auto-execute)
pub fn use_api_simple<T: Clone + 'static>() -> ApiState<T> {
    ApiState {
        loading: use_signal(|| false),
        data: use_signal(|| None),
    }
}

/// Generic API call hook (auto-executes)
/// 
/// Automatically executes API call when component mounts, and re-executes when dependencies change.
/// Uses cached ApiClient instance for better performance.
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