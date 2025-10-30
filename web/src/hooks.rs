use dioxus::prelude::*;
use crate::utils::error::AppError;

/// API 调用状态
#[derive(Clone)]
pub struct ApiState<T: Clone + 'static> {
    pub loading: Signal<bool>,
    pub data: Signal<Option<Result<T, AppError>>>,
}

impl<T: Clone + 'static> ApiState<T> {
    /// 检查是否正在加载
    pub fn is_loading(&self) -> bool {
        *self.loading.read()
    }

    /// 获取错误信息
    pub fn error(&self) -> Option<AppError> {
        self.data.read().as_ref()?.as_ref().err().cloned()
    }

    /// 获取成功数据
    pub fn value(&self) -> Option<T> {
        self.data.read().as_ref()?.as_ref().ok().cloned()
    }

    /// 检查是否有数据（成功或失败）
    pub fn has_data(&self) -> bool {
        self.data.read().is_some()
    }
}

/// 创建 API 状态
fn create_api_state<T: Clone + 'static>() -> ApiState<T> {
    ApiState {
        loading: use_signal(|| true),
        data: use_signal(|| None),
    }
}

/// 简单的 API 调用 hook
pub fn use_api_simple<T: Clone + 'static>() -> ApiState<T> {
    create_api_state::<T>()
}