//! Data-fetching hooks for the web UI.
//!
//! **New code** should prefer [`use_app_resource`] (auto-fetch) and Dioxus [`use_action`](dioxus::prelude::use_action)
//! (user-triggered). Legacy [`use_api`] / [`ApiState`] remain for pages not yet migrated.

use dioxus::prelude::*;
use gloo_timers::callback::Interval;
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
        *self.loading.read() == *other.loading.read()
            && self.data.read().as_ref() == other.data.read().as_ref()
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

/// Dioxus 0.7 [`use_resource`] wrapper with unified [`AppError`] results.
pub fn use_app_resource<T, F, Fut>(fetch: F) -> Resource<Result<T, AppError>>
where
    T: Clone + 'static,
    F: FnMut() -> Fut + 'static,
    Fut: Future<Output = Result<T, AppError>> + 'static,
{
    use_resource(fetch)
}

/// Periodic tick signal for polling APIs (e.g. dashboard metrics).
pub fn use_poll_tick(interval_ms: u32) -> Signal<u32> {
    let tick = use_signal(|| 0u32);
    let _interval = use_signal(|| None::<Interval>);
    use_effect(move || {
        let mut tick = tick;
        let mut slot = _interval;
        slot.set(Some(Interval::new(interval_ms, move || {
            tick.set(tick() + 1);
        })));
    });
    tick
}
