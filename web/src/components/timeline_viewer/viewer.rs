use dioxus::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::colors::colors;
use crate::components::common::EmptyState;
use crate::components::icon::Icon;
use crate::utils::tracing_viewer;

use super::model::{SliceKey, TimelineModel, TimelineSlice, TimelineTrack};
use super::overview::{
    build_track_bars, find_slice_in_model, zoom_fracs, TimeRange, TimelineBarItem,
    TimelineViewState,
};
use super::parse::parse_chrome_trace;

const MIN_VIEW_SPAN: f64 = 0.02;
const BUTTON_ZOOM: f64 = 1.35;
const KEY_ZOOM: f64 = 1.14;
const WHEEL_ZOOM: f64 = 1.12;
const PAN_SCREEN_FRAC: f64 = 0.12;

#[derive(Clone, Copy, PartialEq)]
struct Viewport {
    lo: f64,
    hi: f64,
}

impl Viewport {
    fn full() -> Self {
        Self { lo: 0.0, hi: 1.0 }
    }

    fn span(&self) -> f64 {
        (self.hi - self.lo).max(MIN_VIEW_SPAN)
    }

    fn view_min(&self, min_ts: f64, full_range: f64) -> f64 {
        min_ts + self.lo * full_range
    }

    fn view_range(&self, full_range: f64) -> f64 {
        self.span() * full_range
    }

    fn zoom_in(self, anchor: f64) -> Self {
        self.zoom_in_by(anchor, BUTTON_ZOOM)
    }

    fn zoom_out(self) -> Self {
        self.zoom_out_by(BUTTON_ZOOM)
    }

    fn zoom_in_by(mut self, anchor: f64, factor: f64) -> Self {
        let span = self.span();
        let new_span = (span / factor).max(MIN_VIEW_SPAN);
        let center = self.lo + span * anchor.clamp(0.0, 1.0);
        self.lo = (center - new_span * anchor).clamp(0.0, 1.0 - new_span);
        self.hi = self.lo + new_span;
        self
    }

    fn zoom_out_by(mut self, factor: f64) -> Self {
        let span = self.span();
        let center = (self.lo + self.hi) * 0.5;
        let new_span = (span * factor).min(1.0);
        self.lo = (center - new_span * 0.5).clamp(0.0, 1.0 - new_span);
        self.hi = self.lo + new_span;
        if (self.hi - self.lo - 1.0).abs() < 0.001 {
            return Self::full();
        }
        self
    }

    fn pan_screen(self, direction: f64) -> Self {
        self.pan(self.span() * PAN_SCREEN_FRAC * direction)
    }

    fn pan(mut self, delta_frac: f64) -> Self {
        let span = self.span();
        self.lo = (self.lo + delta_frac).clamp(0.0, 1.0 - span);
        self.hi = self.lo + span;
        self
    }

    fn zoom_to_interval(mut self, start_frac: f64, end_frac: f64) -> Self {
        let lo = start_frac.min(end_frac);
        let hi = start_frac.max(end_frac);
        let span = (hi - lo).max(MIN_VIEW_SPAN);
        let pad = span * 0.15;
        self.lo = (lo - pad).clamp(0.0, 1.0 - span);
        self.hi = (self.lo + span + pad * 2.0).min(1.0);
        if self.hi - self.lo < MIN_VIEW_SPAN {
            self.hi = (self.lo + MIN_VIEW_SPAN).min(1.0);
            self.lo = self.hi - MIN_VIEW_SPAN;
        }
        self
    }
}

#[derive(Clone, PartialEq)]
struct Selection {
    key: SliceKey,
    track_label: String,
}

#[component]
pub fn TimelineViewer(
    trace_json: String,
    #[props(optional)] empty_message: Option<String>,
) -> Element {
    let filter = use_signal(String::new);
    let mut viewport = use_signal(Viewport::full);
    let mut selected = use_signal(|| None::<Selection>);
    let mut view_state = use_signal(TimelineViewState::default);
    let trace_for_shortcuts = trace_json.clone();
    let key_listener = use_hook(|| {
        Rc::new(RefCell::new(
            None::<(web_sys::Window, Closure<dyn FnMut(web_sys::KeyboardEvent)>)>,
        ))
    });

    let listener_for_effect = key_listener.clone();
    use_effect(move || {
        if let Some((window, handler)) = listener_for_effect.borrow_mut().take() {
            let listener = handler.as_ref().unchecked_ref();
            let _ = window.remove_event_listener_with_callback("keydown", listener);
        }

        let Ok(parsed) = parse_chrome_trace(&trace_for_shortcuts) else {
            return;
        };
        if parsed.is_empty() {
            return;
        }

        let Some(window) = web_sys::window() else {
            return;
        };

        let model = parsed;
        let handler = Closure::wrap(Box::new(move |e: web_sys::KeyboardEvent| {
            if timeline_shortcuts_blocked() {
                return;
            }
            let Some(action) = timeline_shortcut_from_key(&e.key()) else {
                return;
            };
            if apply_timeline_shortcut(action, viewport, selected, view_state, &model) {
                e.prevent_default();
            }
        }) as Box<dyn FnMut(web_sys::KeyboardEvent)>);

        let listener = handler.as_ref().unchecked_ref();
        let _ = window.add_event_listener_with_callback("keydown", listener);
        *listener_for_effect.borrow_mut() = Some((window, handler));
    });

    let listener_for_drop = key_listener.clone();
    use_drop(move || {
        if let Some((window, handler)) = listener_for_drop.borrow_mut().take() {
            let listener = handler.as_ref().unchecked_ref();
            let _ = window.remove_event_listener_with_callback("keydown", listener);
        }
    });

    match parse_chrome_trace(&trace_json) {
        Err(err) => rsx! {
            div { class: "p-6 text-sm text-red-700 bg-red-50 border border-red-200 rounded-lg m-4",
                "Failed to parse trace: {err}"
            }
        },
        Ok(parsed) if parsed.is_empty() => rsx! {
            div { class: "p-8",
                EmptyState {
                    message: empty_message
                        .unwrap_or_else(|| "Timeline data is empty.".to_string()),
                }
            }
        },
        Ok(parsed) => {
            let filtered_tracks = filter_tracks(&parsed, &filter());
            let full_range = parsed.range_us();
            let view = viewport();
            let view_min = view.view_min(parsed.min_ts_us, full_range);
            let view_range = view.view_range(full_range);
            let export_json = trace_json.clone();
            let zoom_pct = (1.0 / view.span() * 100.0).round() as i32;
            let in_overview = view_state().is_overview() && filter().trim().is_empty();
            let force_detail = !filter().trim().is_empty();
            rsx! {
                div { class: "flex flex-col h-full min-h-[600px]",
                    TimelineToolbar {
                        model: parsed.clone(),
                        filter,
                        viewport,
                        view_state,
                        selected,
                        track_count: filtered_tracks.len(),
                        zoom_pct,
                        in_overview,
                        on_export: move |_| {
                            if let Err(err) = tracing_viewer::open_perfetto_window(&export_json) {
                                log::warn!("Perfetto export failed: {err}");
                            }
                        },
                    }
                    if filtered_tracks.is_empty() {
                        div { class: "flex-1 flex items-center justify-center p-8",
                            EmptyState {
                                message: format!("No slices match \"{}\"", filter()),
                            }
                        }
                    } else {
                        div { class: "relative flex-1 min-h-0 border-t border-gray-200",
                            div {
                                class: "absolute inset-0 overflow-auto outline-none",
                                tabindex: "-1",
                                onwheel: move |e| {
                                    e.prevent_default();
                                    let delta = e.delta().strip_units().y;
                                    if delta < 0.0 {
                                        viewport.set(viewport().zoom_in_by(0.5, WHEEL_ZOOM));
                                    } else if delta > 0.0 {
                                        viewport.set(viewport().zoom_out_by(WHEEL_ZOOM));
                                    }
                                },
                                div { class: "min-w-full min-h-full",
                                    div { class: "min-w-[640px]",
                                        TimelineRuler {
                                            view_min,
                                            view_range,
                                        }
                                        for track in filtered_tracks {
                                            TimelineTrackRow {
                                                key: "{track.pid}-{track.tid}",
                                                track: track.clone(),
                                                model: parsed.clone(),
                                                view_state,
                                                force_detail,
                                                view_min,
                                                view_range,
                                                viewport,
                                                selected,
                                            }
                                        }
                                    }
                                }
                                div {
                                    class: "pointer-events-none absolute bottom-3 left-3 z-10 px-2 py-1 rounded-md bg-white/90 border border-gray-200/80 text-[10px] text-gray-500 shadow-sm backdrop-blur-sm",
                                    if in_overview {
                                        "Overview · click a region to drill · WASD · F fit · Esc back"
                                    } else {
                                        "Detail · click slice to expand · WASD · F fit · Esc back"
                                    }
                                }
                            }
                            if let Some(sel) = selected() {
                                if let Some(slice) = find_slice_owned(&parsed, sel.key) {
                                    {
                                        let child_count = slice.children.len();
                                        let zoom_lo =
                                            (slice.start_us - parsed.min_ts_us) / full_range;
                                        let zoom_hi =
                                            (slice.end_us() - parsed.min_ts_us) / full_range;
                                        let drill_key = sel.key;
                                        rsx! {
                                            SliceInspector {
                                                slice: slice.clone(),
                                                track_label: sel.track_label.clone(),
                                                full_range,
                                                min_ts: parsed.min_ts_us,
                                                child_count,
                                                on_close: move |_| selected.set(None),
                                                on_zoom: move |_| {
                                                    viewport.set(
                                                        viewport().zoom_to_interval(zoom_lo, zoom_hi),
                                                    );
                                                },
                                                on_drill: move |_| {
                                                    if child_count > 0 {
                                                        view_state.write().drill_path.push(drill_key);
                                                        viewport.set(
                                                            viewport().zoom_to_interval(zoom_lo, zoom_hi),
                                                        );
                                                        selected.set(None);
                                                    }
                                                },
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn find_slice_owned(model: &TimelineModel, key: SliceKey) -> Option<TimelineSlice> {
    find_slice_in_model(model, key).cloned()
}

fn filter_tracks(model: &TimelineModel, query: &str) -> Vec<TimelineTrack> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return model.tracks.clone();
    }
    model
        .tracks
        .iter()
        .filter_map(|track| {
            let slices = filter_slice_tree(&track.slices, &q);
            if slices.is_empty() {
                None
            } else {
                Some(TimelineTrack {
                    pid: track.pid,
                    tid: track.tid,
                    label: track.label.clone(),
                    slices,
                })
            }
        })
        .collect()
}

fn filter_slice_tree(slices: &[TimelineSlice], q: &str) -> Vec<TimelineSlice> {
    let mut out = Vec::new();
    for slice in slices {
        let child_matches = filter_slice_tree(&slice.children, q);
        let self_matches =
            slice.name.to_lowercase().contains(q) || slice.cat.to_lowercase().contains(q);
        if self_matches || !child_matches.is_empty() {
            out.push(TimelineSlice {
                name: slice.name.clone(),
                cat: slice.cat.clone(),
                start_us: slice.start_us,
                dur_us: slice.dur_us,
                pid: slice.pid,
                tid: slice.tid,
                args: slice.args.clone(),
                children: if child_matches.is_empty() {
                    slice.children.clone()
                } else {
                    child_matches
                },
            });
        }
    }
    out
}

#[component]
fn TimelineToolbar(
    model: TimelineModel,
    filter: Signal<String>,
    viewport: Signal<Viewport>,
    view_state: Signal<TimelineViewState>,
    selected: Signal<Option<Selection>>,
    track_count: usize,
    zoom_pct: i32,
    in_overview: bool,
    on_export: EventHandler<()>,
) -> Element {
    let drill_labels: Vec<String> = view_state()
        .drill_path
        .iter()
        .filter_map(|key| find_slice_in_model(&model, *key).map(|s| s.name.clone()))
        .collect();

    rsx! {
        div { class: "flex flex-col gap-2 px-4 py-2.5 bg-gray-50/80 border-b border-gray-200",
            div { class: "flex flex-wrap items-center gap-2",
            div { class: "relative min-w-[140px] flex-1 max-w-xs",
                span { class: "absolute left-2 top-1/2 -translate-y-1/2 text-gray-400 pointer-events-none",
                    Icon { icon: &icondata::AiSearchOutlined, class: "w-3.5 h-3.5" }
                }
                input {
                    r#type: "text",
                    class: "w-full pl-7 pr-2 py-1.5 text-xs rounded-md border border-gray-300 bg-white focus:outline-none focus:ring-2 focus:ring-blue-500/30",
                    placeholder: "Filter slices…",
                    value: "{filter}",
                    oninput: move |ev| filter.set(ev.value()),
                }
            }
            div { class: "flex items-center gap-0.5 rounded-md border border-gray-300 bg-white p-0.5",
                button {
                    class: "p-1.5 rounded hover:bg-gray-100 text-gray-600",
                    title: "Zoom out (S)",
                    onclick: move |_| viewport.set(viewport().zoom_out()),
                    Icon { icon: &icondata::AiZoomOutOutlined, class: "w-3.5 h-3.5" }
                }
                span { class: "px-1.5 text-[11px] font-mono text-gray-500 min-w-[3rem] text-center",
                    "{zoom_pct}%"
                }
                button {
                    class: "p-1.5 rounded hover:bg-gray-100 text-gray-600",
                    title: "Zoom in (W)",
                    onclick: move |_| viewport.set(viewport().zoom_in(0.5)),
                    Icon { icon: &icondata::AiZoomInOutlined, class: "w-3.5 h-3.5" }
                }
                button {
                    class: "px-2 py-1 text-[11px] rounded hover:bg-gray-100 text-gray-600 border-l border-gray-200 ml-0.5",
                    title: "Fit entire trace (F)",
                    onclick: move |_| viewport.set(Viewport::full()),
                    "Fit"
                }
            }
            button {
                class: "px-2 py-1.5 text-[11px] rounded-md border border-gray-300 bg-white hover:bg-gray-50 text-gray-600",
                title: "Pan earlier (A)",
                onclick: move |_| viewport.set(viewport().pan_screen(-1.0)),
                "◀"
            }
            button {
                class: "px-2 py-1.5 text-[11px] rounded-md border border-gray-300 bg-white hover:bg-gray-50 text-gray-600",
                title: "Pan later (D)",
                onclick: move |_| viewport.set(viewport().pan_screen(1.0)),
                "▶"
            }
            div { class: "text-xs text-gray-500 hidden sm:flex items-center gap-x-2",
                span { "{model.event_count} slices" }
                span { "·" }
                span { "{track_count} tracks" }
                span { "·" }
                span { "{format_duration_us(model.range_us())}" }
            }
            button {
                class: format!(
                    "sm:ml-auto inline-flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border border-{} text-{} bg-{} hover:bg-{}",
                    colors::CONTENT_ACCENT_BORDER,
                    colors::CONTENT_ACCENT_TEXT,
                    colors::CONTENT_ACCENT_BG,
                    colors::BTN_SECONDARY_HOVER,
                ),
                title: "Open full trace in Perfetto (new window)",
                onclick: move |_| on_export.call(()),
                Icon { icon: &icondata::AiExportOutlined, class: "w-3.5 h-3.5" }
                "Perfetto"
            }
            }
            div { class: "flex flex-wrap items-center gap-1.5 text-[11px]",
                button {
                    class: if in_overview {
                        "px-2 py-1 rounded-md border border-blue-200 bg-blue-50 text-blue-700 font-medium"
                    } else {
                        "px-2 py-1 rounded-md border border-gray-300 bg-white text-gray-600 hover:bg-gray-50"
                    },
                    onclick: move |_| {
                        view_state.write().reset();
                        viewport.set(Viewport::full());
                        selected.set(None);
                    },
                    "Overview"
                }
                if let Some(range) = view_state().expanded_range.clone() {
                    span { class: "text-gray-400", "›" }
                    button {
                        class: "px-2 py-1 rounded-md border border-gray-300 bg-white text-gray-600 hover:bg-gray-50 max-w-[10rem] truncate",
                        title: "Expanded time range",
                        onclick: move |_| {
                            view_state.write().drill_path.clear();
                            viewport.set({
                                let (lo, hi) = zoom_fracs(
                                    &model,
                                    &TimeRange {
                                        start_us: range.start_us,
                                        end_us: range.end_us,
                                    },
                                );
                                viewport().zoom_to_interval(lo, hi)
                            });
                        },
                        "{format_duration_us(range.start_us - model.min_ts_us)} – {format_duration_us(range.end_us - model.min_ts_us)}"
                    }
                }
                for (idx, label) in drill_labels.iter().enumerate() {
                    span { class: "text-gray-400", "›" }
                    button {
                        class: "px-2 py-1 rounded-md border border-gray-300 bg-white text-gray-600 hover:bg-gray-50 max-w-[9rem] truncate",
                        title: "{label}",
                        onclick: {
                            let keep = idx + 1;
                            move |_| {
                                view_state.write().drill_path.truncate(keep);
                            }
                        },
                        "{label}"
                    }
                }
                if !in_overview {
                    span {
                        class: "ml-1 px-2 py-0.5 rounded-full bg-emerald-50 text-emerald-700 border border-emerald-200 text-[10px] font-medium uppercase tracking-wide",
                        "Detail"
                    }
                }
            }
        }
    }
}

#[component]
fn TimelineRuler(view_min: f64, view_range: f64) -> Element {
    let ticks = [0.0, 0.25, 0.5, 0.75, 1.0];
    rsx! {
        div { class: "flex border-b border-gray-200 bg-white sticky top-0 z-10 shadow-sm",
            div { class: "w-[200px] shrink-0 px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-gray-400 border-r border-gray-100",
                "Track"
            }
            div { class: "flex-1 relative h-7 border-b border-gray-100",
                for pct in ticks {
                    {
                        let ts = view_min + view_range * pct;
                        let label = format_duration_us(ts - view_min);
                        rsx! {
                            div {
                                class: "absolute top-0 bottom-0 border-l border-gray-100",
                                style: "left: {pct * 100.0}%;",
                                span {
                                    class: "absolute top-1 left-1 text-[10px] text-gray-400 font-mono whitespace-nowrap",
                                    "{label}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn TimelineTrackRow(
    track: TimelineTrack,
    model: TimelineModel,
    view_state: Signal<TimelineViewState>,
    force_detail: bool,
    view_min: f64,
    view_range: f64,
    viewport: Signal<Viewport>,
    selected: Signal<Option<Selection>>,
) -> Element {
    let track_label = track.label.clone();
    let bars = build_track_bars(&track, &view_state(), &model, force_detail);
    let is_track_selected =
        selected().is_some_and(|s| s.key.pid == track.pid && s.key.tid == track.tid);

    rsx! {
        div {
            class: if is_track_selected {
                "flex border-b border-blue-100 bg-blue-50/30 min-h-[36px]"
            } else {
                "flex border-b border-gray-50 hover:bg-gray-50/60 min-h-[36px]"
            },
            div {
                class: "w-[200px] shrink-0 px-3 py-2 text-xs text-gray-700 border-r border-gray-100 truncate",
                title: "{track.label}",
                "{track.label}"
            }
            div { class: "flex-1 relative h-9 my-0.5 mx-1 bg-gray-50/40 rounded",
                for bar in bars {
                    TimelineBar {
                        key: "{bar_key(&bar)}",
                        item: bar,
                        track_label: track_label.clone(),
                        model: model.clone(),
                        view_state,
                        viewport,
                        view_min,
                        view_range,
                        selected,
                    }
                }
            }
        }
    }
}

fn bar_key(item: &TimelineBarItem) -> String {
    match item {
        TimelineBarItem::OverviewBucket {
            start_us, end_us, ..
        } => {
            format!("bucket-{start_us}-{end_us}")
        }
        TimelineBarItem::Slice { slice, .. } => {
            format!("{}-{}", slice.start_us, slice.name)
        }
    }
}

#[component]
fn TimelineBar(
    item: TimelineBarItem,
    track_label: String,
    model: TimelineModel,
    view_state: Signal<TimelineViewState>,
    viewport: Signal<Viewport>,
    view_min: f64,
    view_range: f64,
    selected: Signal<Option<Selection>>,
) -> Element {
    match item {
        TimelineBarItem::OverviewBucket {
            start_us,
            end_us,
            count,
            label,
            cat,
        } => {
            let slice_end = end_us;
            if slice_end < view_min || start_us > view_min + view_range {
                return rsx! {};
            }
            let left_pct = ((start_us - view_min) / view_range * 100.0).clamp(0.0, 100.0);
            let end_pct = ((slice_end - view_min) / view_range * 100.0).clamp(left_pct, 100.0);
            let width_pct = (end_pct - left_pct).max(0.8);
            let color = slice_color(&cat);
            let show_label = width_pct > 3.0;
            let range = TimeRange { start_us, end_us };

            rsx! {
                div {
                    class: "absolute top-1 h-7 rounded cursor-pointer {color} opacity-80 hover:opacity-100 hover:ring-2 hover:ring-blue-400/60 border border-white/20 bg-gradient-to-b from-white/10 to-black/10",
                    style: "left: {left_pct:.3}%; width: {width_pct:.3}%; min-width: 4px;",
                    title: "{label} · click to expand",
                    onclick: move |_| {
                        let (lo, hi) = zoom_fracs(&model, &range);
                        view_state.write().expanded_range = Some(range.clone());
                        viewport.set(viewport().zoom_to_interval(lo, hi));
                        selected.set(None);
                    },
                    if show_label {
                        span {
                            class: "absolute inset-0 flex items-center px-1 text-[10px] font-medium text-white truncate pointer-events-none drop-shadow-sm",
                            "{label}"
                        }
                    } else {
                        span {
                            class: "absolute inset-0 flex items-center justify-center text-[9px] font-bold text-white/90 pointer-events-none",
                            "{count}"
                        }
                    }
                }
            }
        }
        TimelineBarItem::Slice { slice, expandable } => {
            let slice_end = slice.end_us();
            if slice_end < view_min || slice.start_us > view_min + view_range {
                return rsx! {};
            }

            let left_pct = ((slice.start_us - view_min) / view_range * 100.0).clamp(0.0, 100.0);
            let end_pct = ((slice_end - view_min) / view_range * 100.0).clamp(left_pct, 100.0);
            let width_pct = if slice.dur_us <= 0.0 {
                0.2_f64
            } else {
                (end_pct - left_pct).max(0.2)
            };

            let key = SliceKey::from_slice(&slice);
            let is_selected = selected().is_some_and(|s| s.key == key);
            let color = slice_color(&slice.cat);
            let show_label = width_pct > 4.0;
            let zoom_lo = (slice.start_us - model.min_ts_us) / model.range_us();
            let zoom_hi = (slice.end_us() - model.min_ts_us) / model.range_us();

            rsx! {
                div {
                    class: if is_selected {
                        "absolute top-1 h-7 rounded cursor-pointer {color} ring-2 ring-blue-500 ring-offset-1 shadow-sm z-10"
                    } else if expandable {
                        "absolute top-1.5 h-6 rounded cursor-pointer {color} opacity-90 hover:opacity-100 hover:ring-2 hover:ring-emerald-400/70 border border-white/15"
                    } else {
                        "absolute top-1.5 h-6 rounded cursor-pointer {color} opacity-85 hover:opacity-100 hover:shadow-sm transition-shadow"
                    },
                    style: "left: {left_pct:.3}%; width: {width_pct:.3}%; min-width: 3px;",
                    title: if expandable {
                        format!("{} · {} · click to expand", slice.name, format_duration_us(slice.dur_us.max(0.0)))
                    } else {
                        format!("{} · {}", slice.name, format_duration_us(slice.dur_us.max(0.0)))
                    },
                    onclick: move |_| {
                        if expandable {
                            view_state.write().drill_path.push(key);
                            viewport.set(viewport().zoom_to_interval(zoom_lo, zoom_hi));
                            selected.set(None);
                        } else {
                            selected.set(Some(Selection {
                                key,
                                track_label: track_label.clone(),
                            }));
                        }
                    },
                    if show_label {
                        span {
                            class: "absolute inset-0 flex items-center gap-0.5 px-1 text-[10px] font-medium text-white truncate pointer-events-none drop-shadow-sm",
                            if expandable {
                                span { class: "opacity-90", "▸" }
                            }
                            "{slice.name}"
                        }
                    } else if expandable {
                        span {
                            class: "absolute inset-0 flex items-center justify-center text-[9px] font-bold text-white/90 pointer-events-none",
                            "▸"
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn SliceInspector(
    slice: TimelineSlice,
    track_label: String,
    full_range: f64,
    min_ts: f64,
    child_count: usize,
    on_close: EventHandler<()>,
    on_zoom: EventHandler<()>,
    on_drill: EventHandler<()>,
) -> Element {
    let color = slice_color(&slice.cat);
    let rel_start = slice.start_us - min_ts;
    let rel_end = slice.end_us() - min_ts;

    rsx! {
        div {
            class: "absolute top-3 right-3 z-30 w-72 max-h-[calc(100%-1.5rem)] flex flex-col overflow-hidden rounded-xl border border-gray-200/90 bg-white/95 backdrop-blur-md shadow-xl shadow-gray-900/10 pointer-events-auto",
            div { class: "px-4 py-3 border-b border-gray-100 bg-gray-50/90",
                div { class: "flex items-start justify-between gap-2",
                    div { class: "min-w-0 flex-1",
                        div { class: "flex items-center gap-2 mb-1",
                            span { class: "w-2.5 h-2.5 rounded-sm shrink-0 {color}" }
                            h3 { class: "text-sm font-semibold text-gray-900 truncate", "{slice.name}" }
                        }
                        p { class: "text-[11px] text-gray-500 truncate", "{track_label}" }
                    }
                    button {
                        class: "p-1 rounded-md text-gray-400 hover:text-gray-700 hover:bg-gray-200/80",
                        title: "Close",
                        onclick: move |_| on_close.call(()),
                        Icon { icon: &icondata::AiCloseOutlined, class: "w-4 h-4" }
                    }
                }
                span {
                    class: format!(
                        "inline-block mt-2 text-[10px] font-semibold uppercase tracking-wide px-2 py-0.5 rounded border bg-{} text-{} border-{}",
                        colors::CONTENT_ACCENT_BG,
                        colors::CONTENT_ACCENT_TEXT,
                        colors::CONTENT_ACCENT_BORDER,
                    ),
                    "{slice.cat}"
                }
            }
            div { class: "flex-1 overflow-y-auto p-4 space-y-4",
                div { class: "grid grid-cols-2 gap-3",
                    StatCell { label: "Duration", value: format_duration_us(slice.dur_us.max(0.0)) }
                    StatCell {
                        label: "% of trace",
                        value: format!("{:.1}%", slice.dur_us / full_range.max(1.0) * 100.0),
                    }
                    StatCell { label: "Start", value: format_duration_us(rel_start) }
                    StatCell { label: "End", value: format_duration_us(rel_end) }
                    StatCell { label: "Process", value: format!("pid {}", slice.pid) }
                    StatCell { label: "Thread", value: format!("tid {}", slice.tid) }
                }
                button {
                    class: format!(
                        "w-full px-3 py-2 text-xs rounded-md border border-{} text-{} bg-{} hover:bg-{} transition-colors",
                        colors::CONTENT_ACCENT_BORDER,
                        colors::CONTENT_ACCENT_TEXT,
                        colors::CONTENT_ACCENT_BG,
                        colors::BTN_SECONDARY_HOVER,
                    ),
                    onclick: move |_| on_zoom.call(()),
                    Icon { icon: &icondata::AiZoomInOutlined, class: "w-3.5 h-3.5 inline mr-1 -mt-0.5" }
                    "Zoom to this slice"
                }
                if child_count > 0 {
                    button {
                        class: "w-full px-3 py-2 text-xs rounded-md border border-emerald-200 text-emerald-800 bg-emerald-50 hover:bg-emerald-100 transition-colors",
                        onclick: move |_| on_drill.call(()),
                        "▸ Expand {child_count} nested slices"
                    }
                }
                if let Some(args) = &slice.args {
                    div { class: "space-y-2",
                        p { class: "text-[10px] font-semibold uppercase tracking-wide text-gray-400",
                            "Attributes"
                        }
                        SliceArgsList { args: args.clone() }
                    }
                }
            }
        }
    }
}

enum TimelineShortcut {
    ZoomIn,
    ZoomOut,
    PanLeft,
    PanRight,
    Fit,
    ZoomSelection,
    Back,
}

fn timeline_shortcut_from_key(key: &str) -> Option<TimelineShortcut> {
    match key {
        "w" | "W" => Some(TimelineShortcut::ZoomIn),
        "s" | "S" => Some(TimelineShortcut::ZoomOut),
        "a" | "A" => Some(TimelineShortcut::PanLeft),
        "d" | "D" => Some(TimelineShortcut::PanRight),
        "f" | "F" => Some(TimelineShortcut::Fit),
        "z" | "Z" => Some(TimelineShortcut::ZoomSelection),
        "Escape" => Some(TimelineShortcut::Back),
        _ => None,
    }
}

fn timeline_shortcuts_blocked() -> bool {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return true;
    };
    let Some(active) = document.active_element() else {
        return false;
    };
    let tag = active.tag_name();
    tag == "INPUT" || tag == "TEXTAREA" || tag == "SELECT"
}

fn apply_timeline_shortcut(
    action: TimelineShortcut,
    mut viewport: Signal<Viewport>,
    mut selected: Signal<Option<Selection>>,
    mut view_state: Signal<TimelineViewState>,
    model: &TimelineModel,
) -> bool {
    match action {
        TimelineShortcut::Back => {
            if selected().is_some() {
                selected.set(None);
                return true;
            }
            if !view_state().is_overview() {
                view_state.write().pop_drill();
                return true;
            }
            false
        }
        TimelineShortcut::ZoomIn => {
            viewport.set(viewport().zoom_in_by(0.5, KEY_ZOOM));
            true
        }
        TimelineShortcut::ZoomOut => {
            viewport.set(viewport().zoom_out_by(KEY_ZOOM));
            true
        }
        TimelineShortcut::PanLeft => {
            viewport.set(viewport().pan_screen(-1.0));
            true
        }
        TimelineShortcut::PanRight => {
            viewport.set(viewport().pan_screen(1.0));
            true
        }
        TimelineShortcut::Fit => {
            view_state.write().reset();
            viewport.set(Viewport::full());
            true
        }
        TimelineShortcut::ZoomSelection => {
            if let Some(sel) = selected() {
                if let Some(slice) = find_slice_owned(model, sel.key) {
                    let full_range = model.range_us();
                    let min_ts = model.min_ts_us;
                    let lo = (slice.start_us - min_ts) / full_range;
                    let hi = (slice.end_us() - min_ts) / full_range;
                    viewport.set(viewport().zoom_to_interval(lo, hi));
                    return true;
                }
            }
            false
        }
    }
}

#[component]
fn StatCell(label: &'static str, value: String) -> Element {
    rsx! {
        div { class: "rounded-lg border border-gray-100 bg-gray-50/50 px-2.5 py-2",
            p { class: "text-[10px] text-gray-400 uppercase tracking-wide", "{label}" }
            p { class: "text-sm font-mono font-medium text-gray-900 mt-0.5 truncate", "{value}" }
        }
    }
}

#[component]
fn SliceArgsList(args: serde_json::Value) -> Element {
    rsx! {
        if let Some(obj) = args.as_object() {
            div { class: "rounded-lg border border-gray-200 divide-y divide-gray-100 overflow-hidden",
                for (key, val) in obj.iter() {
                    div { class: "px-3 py-2 bg-white",
                        p { class: "text-[10px] font-medium text-gray-500", "{key}" }
                        p { class: "text-xs font-mono text-gray-800 break-all mt-0.5",
                            { arg_display(val) }
                        }
                    }
                }
            }
        } else {
            pre { class: "text-xs font-mono text-gray-600 bg-gray-50 border border-gray-200 rounded-lg p-3 overflow-x-auto",
                "{args}"
            }
        }
    }
}

fn arg_display(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

fn slice_color(cat: &str) -> &'static str {
    match cat {
        "span" | "default" => "bg-blue-500",
        "event" => "bg-violet-500",
        "python" | "cpu" => "bg-emerald-500",
        "cuda" | "gpu" => "bg-amber-500",
        _ if cat.contains("torch") => "bg-orange-500",
        _ => "bg-slate-500",
    }
}

fn format_duration_us(us: f64) -> String {
    if us >= 1_000_000.0 {
        format!("{:.2}s", us / 1_000_000.0)
    } else if us >= 1_000.0 {
        format!("{:.2}ms", us / 1_000.0)
    } else {
        format!("{us:.0}µs")
    }
}
