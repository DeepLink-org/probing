//! Data-fetching hooks for the web UI.
//!
//! **New code** should prefer [`use_app_resource`] (auto-fetch) and Dioxus [`use_action`](dioxus::prelude::use_action)
//! (user-triggered). Legacy [`use_api`] / [`ApiState`] remain for pages not yet migrated.

use dioxus::prelude::*;
use gloo_timers::callback::Interval;
use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;
use wasm_bindgen::JsCast;
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
pub fn use_api<T, F, Fut>(fetch_fn: F) -> ApiState<T>
where
    T: Clone + 'static,
    F: FnMut() -> Fut + 'static,
    Fut: Future<Output = Result<T, AppError>> + 'static,
{
    use_api_with_options(fetch_fn, ApiFetchOptions::default())
}

#[derive(Clone, Copy, Default)]
pub struct ApiFetchOptions {
    /// When true, skip the loading spinner on refetch if data is already present.
    pub keep_previous_while_refreshing: bool,
}

/// Like [`use_api`] with refresh behavior controls (for polled dashboards).
pub fn use_api_with_options<T, F, Fut>(
    mut fetch_fn: F,
    options: ApiFetchOptions,
) -> ApiState<T>
where
    T: Clone + 'static,
    F: FnMut() -> Fut + 'static,
    Fut: Future<Output = Result<T, AppError>> + 'static,
{
    let state = use_api_simple::<T>();

    use_effect(move || {
        let mut loading = state.loading;
        let mut data = state.data;
        let show_loading =
            !options.keep_previous_while_refreshing || data.read().is_none();
        let result_future = fetch_fn();
        spawn(async move {
            if show_loading {
                *loading.write() = true;
            }
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
/// Prefer [`use_poll_tick_gated`] when the page can be hidden.
#[allow(dead_code)]
pub fn use_poll_tick(interval_ms: u32) -> Signal<u32> {
    use_poll_tick_gated(interval_ms, None)
}

pub fn use_poll_tick_gated(interval_ms: u32, gate: Option<Signal<bool>>) -> Signal<u32> {
    let tick = use_signal(|| 0u32);
    let mut interval_slot = use_signal(|| None::<Interval>);

    use_effect(move || {
        let mut tick = tick;
        let gate = gate;
        interval_slot.set(Some(Interval::new(interval_ms, move || {
            let allowed = gate.map(|g| g()).unwrap_or(true);
            if allowed {
                tick.set(tick() + 1);
            }
        })));
    });

    use_drop(move || {
        interval_slot.set(None);
    });

    tick
}

/// True while the document tab is visible (not backgrounded).
pub fn use_page_visible() -> Signal<bool> {
    let visible = use_signal(|| true);
    let slot = use_hook(|| {
        Rc::new(RefCell::new(
            None::<(
                web_sys::Document,
                wasm_bindgen::closure::Closure<dyn FnMut(web_sys::Event)>,
            )>,
        ))
    });

    let slot_for_effect = slot.clone();
    use_effect(move || {
        if let Some((document, handler)) = slot_for_effect.borrow_mut().take() {
            let listener = handler.as_ref().unchecked_ref();
            let _ = document.remove_event_listener_with_callback("visibilitychange", listener);
        }

        let Some(window) = web_sys::window() else {
            return;
        };
        let Some(document) = window.document() else {
            return;
        };

        let mut visible = visible;
        visible.set(!document.hidden());
        let handler = wasm_bindgen::closure::Closure::wrap(Box::new(move |_e: web_sys::Event| {
            if let Some(window) = web_sys::window() {
                if let Some(document) = window.document() {
                    visible.set(!document.hidden());
                }
            }
        }) as Box<dyn FnMut(web_sys::Event)>);
        let listener = handler.as_ref().unchecked_ref();
        let _ = document.add_event_listener_with_callback("visibilitychange", listener);
        *slot_for_effect.borrow_mut() = Some((document, handler));
    });

    let slot_for_drop = slot.clone();
    use_drop(move || {
        if let Some((document, handler)) = slot_for_drop.borrow_mut().take() {
            let listener = handler.as_ref().unchecked_ref();
            let _ = document.remove_event_listener_with_callback("visibilitychange", listener);
        }
    });

    visible
}
