use dioxus::prelude::*;

#[derive(Debug, Clone, PartialEq)]
pub struct ChartSeries {
    pub label: String,
    pub points: Vec<(f64, f64)>,
    pub color: &'static str,
}

#[component]
pub fn MetricsLineChart(
    title: String,
    series: Vec<ChartSeries>,
    #[props(default = 280.0)] height: f64,
) -> Element {
    if series.is_empty() || series.iter().all(|s| s.points.is_empty()) {
        return rsx! {
            div { class: "rounded-lg border border-slate-200 bg-white p-4",
                div { class: "text-sm font-medium text-slate-700 mb-2", "{title}" }
                div { class: "text-xs text-slate-400 py-8 text-center", "No samples yet" }
            }
        };
    }

    let width = 640.0;
    let chart_height = height;
    let pad_left = 52.0;
    let pad_right = 16.0;
    let pad_top = 16.0;
    let pad_bottom = 36.0;
    let plot_w = width - pad_left - pad_right;
    let plot_h = chart_height - pad_top - pad_bottom;

    let (x_min, x_max, y_min, y_max) = bounds(&series);
    let x_span = (x_max - x_min).max(1.0);
    let y_span = (y_max - y_min).max(f64::EPSILON);

    let y_ticks = y_axis_ticks(y_min, y_max);
    let x_labels = x_axis_labels(&series, x_min, x_max, 5);

    rsx! {
        div { class: "rounded-lg border border-slate-200 bg-white p-4",
            div { class: "flex items-center justify-between gap-2 mb-3",
                div { class: "text-sm font-medium text-slate-700", "{title}" }
                div { class: "flex flex-wrap gap-3 text-[11px] text-slate-600",
                    for s in series.iter().filter(|s| !s.points.is_empty()) {
                        div { class: "flex items-center gap-1.5",
                            span {
                                class: "inline-block w-2.5 h-2.5 rounded-full",
                                style: "background-color: {s.color};",
                            }
                            span { "{s.label}" }
                        }
                    }
                }
            }
            svg {
                class: "w-full",
                view_box: "0 0 {width} {chart_height}",
                preserve_aspect_ratio: "none",
                rect {
                    x: "{pad_left}",
                    y: "{pad_top}",
                    width: "{plot_w}",
                    height: "{plot_h}",
                    fill: "#f8fafc",
                    stroke: "#e2e8f0",
                }
                for tick in y_ticks.iter() {
                    {
                        let y = pad_top + plot_h - ((tick - y_min) / y_span * plot_h);
                        let x2 = pad_left + plot_w;
                        let label_x = pad_left - 6.0;
                        let label_y = y + 4.0;
                        rsx! {
                            g {
                                line {
                                    x1: "{pad_left}",
                                    y1: "{y}",
                                    x2: "{x2}",
                                    y2: "{y}",
                                    stroke: "#e2e8f0",
                                    stroke_width: "1",
                                }
                                text {
                                    x: "{label_x}",
                                    y: "{label_y}",
                                    text_anchor: "end",
                                    font_size: "10",
                                    fill: "#64748b",
                                    "{format_tick(*tick)}"
                                }
                            }
                        }
                    }
                }
                for (label, x_val) in x_labels.iter() {
                    {
                        let x = pad_left + ((x_val - x_min) / x_span * plot_w);
                        let label_y = chart_height - 8.0;
                        rsx! {
                            text {
                                x: "{x}",
                                y: "{label_y}",
                                text_anchor: "middle",
                                font_size: "10",
                                fill: "#64748b",
                                "{label}"
                            }
                        }
                    }
                }
                for s in series.iter().filter(|s| s.points.len() >= 2) {
                    {
                        let polyline = s
                            .points
                            .iter()
                            .map(|(x, y)| {
                                let px = pad_left + ((x - x_min) / x_span * plot_w);
                                let py = pad_top + plot_h - ((y - y_min) / y_span * plot_h);
                                format!("{px},{py}")
                            })
                            .collect::<Vec<_>>()
                            .join(" ");
                        rsx! {
                            polyline {
                                points: "{polyline}",
                                fill: "none",
                                stroke: "{s.color}",
                                stroke_width: "2",
                                stroke_linejoin: "round",
                                stroke_linecap: "round",
                            }
                        }
                    }
                }
                for s in series.iter().filter(|s| s.points.len() == 1) {
                    {
                        let (x, y) = s.points[0];
                        let px = pad_left + ((x - x_min) / x_span * plot_w);
                        let py = pad_top + plot_h - ((y - y_min) / y_span * plot_h);
                        rsx! {
                            circle {
                                cx: "{px}",
                                cy: "{py}",
                                r: "3.5",
                                fill: "{s.color}",
                            }
                        }
                    }
                }
            }
        }
    }
}

fn bounds(series: &[ChartSeries]) -> (f64, f64, f64, f64) {
    let mut x_min = f64::INFINITY;
    let mut x_max = f64::NEG_INFINITY;
    let mut y_min = f64::INFINITY;
    let mut y_max = f64::NEG_INFINITY;

    for s in series {
        for (x, y) in &s.points {
            x_min = x_min.min(*x);
            x_max = x_max.max(*x);
            y_min = y_min.min(*y);
            y_max = y_max.max(*y);
        }
    }

    if y_min == y_max {
        let pad = if y_min.abs() > 1.0 { y_min.abs() * 0.1 } else { 1.0 };
        y_min -= pad;
        y_max += pad;
    } else {
        let pad = (y_max - y_min) * 0.08;
        y_min -= pad;
        y_max += pad;
    }

    (x_min, x_max, y_min, y_max)
}

fn y_axis_ticks(y_min: f64, y_max: f64) -> Vec<f64> {
    (0..=4)
        .map(|i| y_min + (y_max - y_min) * (i as f64 / 4.0))
        .collect()
}

fn x_axis_labels(_series: &[ChartSeries], x_min: f64, x_max: f64, count: usize) -> Vec<(String, f64)> {
    if count <= 1 {
        return vec![(format_time_ms(x_min), x_min)];
    }
    (0..count)
        .map(|i| {
            let x = x_min + (x_max - x_min) * (i as f64 / (count - 1) as f64);
            (format_time_ms(x), x)
        })
        .collect()
}

fn format_time_ms(ts_ms: f64) -> String {
    let secs = (ts_ms / 1000.0) as i64;
    let nanos = ((ts_ms % 1000.0) * 1_000_000.0) as u32;
    chrono::DateTime::from_timestamp(secs, nanos)
        .map(|dt| dt.format("%H:%M:%S").to_string())
        .unwrap_or_else(|| format!("{ts_ms:.0}"))
}

fn format_tick(value: f64) -> String {
    if value.abs() >= 1000.0 {
        format!("{value:.0}")
    } else if value.abs() >= 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.3}")
    }
}
