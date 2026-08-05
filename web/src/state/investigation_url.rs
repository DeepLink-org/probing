//! Sync investigation context with URL query parameters (`?pid=&tid=&trace_id=`).

use dioxus::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;

use crate::state::investigation::{
    update_investigation_context, InvestigationContext, INVESTIGATION_CONTEXT,
};

const QUERY_PID: &str = "pid";
const QUERY_TID: &str = "tid";
const QUERY_RANK: &str = "rank";
const QUERY_HOST: &str = "host";
const QUERY_DEVICE: &str = "gpu";
const QUERY_STEP: &str = "step";
const QUERY_TRACE_ID: &str = "trace_id";
const QUERY_SPAN: &str = "span";

pub fn parse_context_from_search(search: &str) -> InvestigationContext {
    let search = search.trim_start_matches('?');
    if search.is_empty() {
        return InvestigationContext::default();
    }

    let mut ctx = InvestigationContext::default();
    for pair in search.split('&') {
        if pair.is_empty() {
            continue;
        }
        let Some((key, raw)) = pair.split_once('=') else {
            continue;
        };
        let value = urlencoding::decode(raw)
            .map(|cow| cow.into_owned())
            .unwrap_or_else(|_| raw.to_string());
        match key {
            QUERY_PID => ctx.pid = value.parse().ok(),
            QUERY_TID => ctx.tid = value.parse().ok(),
            QUERY_RANK => ctx.rank = value.parse().ok(),
            QUERY_HOST if !value.is_empty() => ctx.host = Some(value),
            QUERY_DEVICE => ctx.device_id = value.parse().ok(),
            QUERY_STEP => ctx.local_step = value.parse().ok(),
            QUERY_TRACE_ID => ctx.trace_id = value.parse().ok(),
            QUERY_SPAN if !value.is_empty() => ctx.span_name = Some(value),
            _ => {}
        }
    }

    if !ctx.is_empty() {
        ctx.label = None;
        ctx.label = Some(ctx.summary());
    }
    ctx
}

pub fn context_to_search(ctx: &InvestigationContext) -> String {
    if ctx.is_empty() {
        return String::new();
    }

    let mut parts = Vec::new();
    if let Some(pid) = ctx.pid {
        parts.push(format!("{QUERY_PID}={pid}"));
    }
    if let Some(tid) = ctx.tid {
        parts.push(format!("{QUERY_TID}={tid}"));
    }
    if let Some(rank) = ctx.rank {
        parts.push(format!("{QUERY_RANK}={rank}"));
    }
    if let Some(host) = &ctx.host {
        if !host.is_empty() {
            parts.push(format!("{QUERY_HOST}={}", urlencoding::encode(host)));
        }
    }
    if let Some(device_id) = ctx.device_id {
        parts.push(format!("{QUERY_DEVICE}={device_id}"));
    }
    if let Some(step) = ctx.local_step {
        parts.push(format!("{QUERY_STEP}={step}"));
    }
    if let Some(trace_id) = ctx.trace_id {
        parts.push(format!("{QUERY_TRACE_ID}={trace_id}"));
    }
    if let Some(span) = &ctx.span_name {
        if !span.is_empty() {
            parts.push(format!("{QUERY_SPAN}={}", urlencoding::encode(span)));
        }
    }
    parts.join("&")
}

pub fn current_url_search() -> String {
    web_sys::window()
        .and_then(|w| w.location().search().ok())
        .unwrap_or_default()
}

pub fn apply_investigation_context_from_url() {
    let url_ctx = parse_context_from_search(&current_url_search());
    if url_ctx.is_empty() {
        return;
    }
    update_investigation_context(|ctx| {
        if url_ctx.pid.is_some() {
            ctx.pid = url_ctx.pid;
        }
        if url_ctx.tid.is_some() {
            ctx.tid = url_ctx.tid;
        }
        if url_ctx.rank.is_some() {
            ctx.rank = url_ctx.rank;
        }
        if url_ctx.host.is_some() {
            ctx.host = url_ctx.host.clone();
        }
        if url_ctx.device_id.is_some() {
            ctx.device_id = url_ctx.device_id;
        }
        if url_ctx.local_step.is_some() {
            ctx.local_step = url_ctx.local_step;
        }
        if url_ctx.trace_id.is_some() {
            ctx.trace_id = url_ctx.trace_id;
        }
        if url_ctx.span_name.is_some() {
            ctx.span_name = url_ctx.span_name.clone();
        }
        ctx.label = Some(ctx.summary());
    });
}

pub fn sync_investigation_context_to_url() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let location = window.location();
    let Ok(pathname) = location.pathname() else {
        return;
    };
    let hash = location.hash().unwrap_or_default();
    let query = context_to_search(&INVESTIGATION_CONTEXT.read());
    let new_url = if query.is_empty() {
        format!("{pathname}{hash}")
    } else {
        format!("{pathname}?{query}{hash}")
    };

    if let Ok(history) = window.history() {
        let _ = history.replace_state_with_url(&JsValue::NULL, "", Some(&new_url));
    }
}

/// Keep URL query in sync with global context; re-apply on browser back/forward.
#[component]
pub fn InvestigationUrlSync(#[props(optional)] route_key: Option<String>) -> Element {
    let ctx = INVESTIGATION_CONTEXT.read().clone();
    let ctx_key = format!(
        "{}:{}:{}:{}:{}:{}:{}:{}:{}",
        ctx.pid.unwrap_or(-1),
        ctx.tid.unwrap_or(-1),
        ctx.rank.unwrap_or(-1),
        ctx.host.as_deref().unwrap_or(""),
        ctx.device_id.unwrap_or(-1),
        ctx.trace_id.unwrap_or(-1),
        ctx.span_name.as_deref().unwrap_or(""),
        ctx.local_step.unwrap_or(-1),
        route_key.as_deref().unwrap_or("")
    );

    use_effect(move || {
        let _ = ctx_key;
        sync_investigation_context_to_url();
    });

    let slot = use_hook(|| {
        Rc::new(RefCell::new(
            None::<(
                web_sys::Window,
                wasm_bindgen::closure::Closure<dyn FnMut(web_sys::Event)>,
            )>,
        ))
    });

    let slot_for_effect = slot.clone();
    use_effect(move || {
        if let Some((window, handler)) = slot_for_effect.borrow_mut().take() {
            let listener = handler.as_ref().unchecked_ref();
            let _ = window.remove_event_listener_with_callback("popstate", listener);
        }

        let Some(window) = web_sys::window() else {
            return;
        };

        let handler = wasm_bindgen::closure::Closure::wrap(Box::new(move |_e: web_sys::Event| {
            apply_investigation_context_from_url();
        })
            as Box<dyn FnMut(web_sys::Event)>);
        let listener = handler.as_ref().unchecked_ref();
        let _ = window.add_event_listener_with_callback("popstate", listener);
        *slot_for_effect.borrow_mut() = Some((window, handler));
    });

    use_drop(move || {
        if let Some((window, handler)) = slot.borrow_mut().take() {
            let listener = handler.as_ref().unchecked_ref();
            let _ = window.remove_event_listener_with_callback("popstate", listener);
        }
    });

    rsx! {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn investigation_context_round_trips_training_and_trace_coordinates() {
        let context = InvestigationContext {
            pid: Some(42),
            tid: Some(7),
            rank: Some(19),
            host: Some("trainer-02".to_string()),
            device_id: Some(3),
            trace_id: Some(88),
            span_name: Some("train.step".to_string()),
            local_step: Some(119),
            label: None,
        };

        let parsed = parse_context_from_search(&context_to_search(&context));
        assert_eq!(parsed.pid, context.pid);
        assert_eq!(parsed.tid, context.tid);
        assert_eq!(parsed.rank, context.rank);
        assert_eq!(parsed.host, context.host);
        assert_eq!(parsed.device_id, context.device_id);
        assert_eq!(parsed.trace_id, context.trace_id);
        assert_eq!(parsed.span_name, context.span_name);
        assert_eq!(parsed.local_step, context.local_step);
    }
}
