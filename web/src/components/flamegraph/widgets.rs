use dioxus::prelude::*;

/// Root layout for flamegraph views.
#[component]
pub fn FlamegraphShell(
    #[props(optional)] min_height: Option<&'static str>,
    children: Element,
) -> Element {
    let min_h = min_height.unwrap_or("min-h-[520px]");
    rsx! {
        div { class: "flex flex-col {min_h}", {children} }
    }
}

/// One-line interaction hint above the chart.
#[component]
pub fn FlamegraphHint(message: &'static str) -> Element {
    rsx! {
        div {
            class: "px-4 py-2 border-b border-gray-100 text-xs text-gray-500",
            "{message}"
        }
    }
}

/// Toolbar row (search, filters, stats).
#[component]
pub fn FlamegraphToolbar(children: Element) -> Element {
    rsx! {
        div {
            class: "flex flex-wrap gap-3 items-center px-4 py-3 border-b border-gray-200 bg-white",
            {children}
        }
    }
}

#[component]
pub fn StackSearchInput(
    value: String,
    placeholder: &'static str,
    on_input: EventHandler<String>,
) -> Element {
    rsx! {
        input {
            r#type: "search",
            class: "flex-1 min-w-[180px] max-w-xs px-3 py-1.5 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500/30 focus:border-blue-500",
            placeholder: "{placeholder}",
            value: "{value}",
            oninput: move |e| on_input.call(e.value()),
        }
    }
}

#[component]
pub fn ModuleSearchInput(value: String, on_input: EventHandler<String>) -> Element {
    rsx! {
        StackSearchInput {
            value,
            placeholder: "Filter modules…",
            on_input,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PhasePillTone {
    All,
    Forward,
    Step,
    Backward,
    Neutral,
}

impl PhasePillTone {
    pub fn from_phase(phase: &str) -> Self {
        match phase {
            "all" => Self::All,
            "forward" => Self::Forward,
            "step" => Self::Step,
            "backward" => Self::Backward,
            _ => Self::Neutral,
        }
    }

    fn active_classes(self) -> &'static str {
        match self {
            Self::Forward => "bg-blue-100 text-blue-700 border-blue-200",
            Self::Step => "bg-amber-100 text-amber-800 border-amber-200",
            Self::Backward => "bg-purple-100 text-purple-800 border-purple-200",
            Self::All | Self::Neutral => "bg-gray-100 text-gray-800 border-gray-200",
        }
    }
}

#[component]
pub fn PhasePill(
    label: String,
    active: bool,
    tone: PhasePillTone,
    onclick: EventHandler<()>,
) -> Element {
    let class = if active {
        tone.active_classes()
    } else {
        "bg-white text-gray-500 border-gray-200 hover:bg-gray-50"
    };
    rsx! {
        button {
            class: "px-3 py-1 text-xs font-medium rounded-full border transition-colors {class}",
            onclick: move |_| onclick.call(()),
            "{label}"
        }
    }
}

#[component]
pub fn PhasePillRow(children: Element) -> Element {
    rsx! {
        div { class: "flex flex-wrap gap-1.5", {children} }
    }
}

#[component]
pub fn MetricPillRow(children: Element) -> Element {
    rsx! {
        div { class: "flex flex-wrap gap-1.5 items-center", {children} }
    }
}

#[component]
pub fn MetricPill(label: String, active: bool, onclick: EventHandler<()>) -> Element {
    let class = if active {
        "bg-emerald-100 text-emerald-800 border-emerald-200"
    } else {
        "bg-white text-gray-500 border-gray-200 hover:bg-gray-50"
    };
    rsx! {
        button {
            class: "px-3 py-1 text-xs font-medium rounded-full border transition-colors {class}",
            onclick: move |_| onclick.call(()),
            "{label}"
        }
    }
}

#[component]
pub fn StatChip(label: &'static str, value: String) -> Element {
    rsx! {
        span {
            class: "px-2 py-1 rounded-md bg-gray-100 border border-gray-200 text-xs text-gray-500",
            "{label} "
            strong { class: "text-gray-800", "{value}" }
        }
    }
}

#[component]
pub fn StatChipRow(children: Element) -> Element {
    rsx! {
        div { class: "flex gap-2 text-xs text-gray-500", {children} }
    }
}

#[component]
pub fn BreadcrumbBar(children: Element) -> Element {
    rsx! {
        div {
            class: "px-4 py-2 text-xs text-gray-500 border-b border-gray-100 bg-gray-50 flex flex-wrap gap-1 items-center",
            {children}
        }
    }
}

#[component]
pub fn BreadcrumbLink(label: String, onclick: EventHandler<()>) -> Element {
    rsx! {
        button {
            class: "text-blue-600 hover:underline",
            onclick: move |_| onclick.call(()),
            "{label}"
        }
    }
}

#[component]
pub fn BreadcrumbSeparator() -> Element {
    rsx! { span { "›" } }
}

#[component]
pub fn ChartPanel(
    onclick: Option<EventHandler<()>>,
    children: Element,
) -> Element {
    rsx! {
        div {
            class: "flex-1 overflow-auto p-4 bg-slate-950 rounded-b-lg",
            onclick: move |_| {
                if let Some(handler) = onclick {
                    handler.call(());
                }
            },
            {children}
        }
    }
}

#[component]
pub fn FlamegraphSvg(
    width: f64,
    height: f64,
    children: Element,
) -> Element {
    rsx! {
        svg {
            class: "w-full",
            xmlns: "http://www.w3.org/2000/svg",
            role: "img",
            view_box: "0 0 {width} {height}",
            {children}
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct TooltipState {
    pub title: String,
    pub lines: Vec<String>,
    pub x: f64,
    pub y: f64,
}

#[component]
pub fn FloatingTooltip(state: TooltipState) -> Element {
    let left = state.x + 14.0;
    let top = state.y + 14.0;
    let panel = "fixed z-50 pointer-events-none max-w-sm rounded-lg border border-slate-600 bg-slate-900/95 text-white text-xs px-3 py-2 shadow-xl";
    let line = "text-slate-400";
    rsx! {
        div {
            class: "{panel}",
            style: "left: {left}px; top: {top}px;",
            div { class: "font-semibold mb-1", "{state.title}" }
            for line_text in state.lines.iter() {
                div { class: "{line}", "{line_text}" }
            }
        }
    }
}
