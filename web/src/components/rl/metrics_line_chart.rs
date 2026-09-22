use dioxus::prelude::*;

/// Most points that still get an individual marker, and the budget for click
/// targets on longer series.
const MARKER_LIMIT: usize = 80;
/// Points actually drawn. A few thousand steps squeezed into a few hundred pixels
/// overdraws into a solid band, so longer series are averaged into this many
/// buckets and the spread within each bucket is drawn as a range behind the line.
const PLOT_POINT_BUDGET: usize = 192;

/// One drawn point, standing for one or more input points.
#[derive(Debug, Clone, PartialEq)]
struct PlotPoint {
    x: f64,
    y: f64,
    /// Span of the inputs behind this point; both equal `y` when it stands alone.
    low: f64,
    high: f64,
    step: i64,
}

#[derive(Debug, Clone, PartialEq)]
struct PlotSeries {
    label: String,
    color: &'static str,
    readout: String,
    delta: String,
    points: Vec<PlotPoint>,
    /// Set when buckets hold more than one input, which is when the range band
    /// carries information worth drawing.
    aggregated: bool,
}

/// Average `points` into at most `budget` buckets, keeping each bucket's range.
fn downsample(points: &[ChartPoint], budget: usize) -> (Vec<PlotPoint>, bool) {
    let budget = budget.max(1);
    // Already summarised upstream, by the server's bucketed series endpoint.
    // Re-bucketing would average the averages and lose the true range.
    if points.iter().any(|point| point.band.is_some()) {
        let summarised = points
            .iter()
            .map(|point| {
                let (low, high) = point.band.unwrap_or((point.y, point.y));
                PlotPoint {
                    x: point.x,
                    y: point.y,
                    low,
                    high,
                    step: point.step,
                }
            })
            .collect();
        return (summarised, true);
    }
    if points.len() <= budget {
        let plain = points
            .iter()
            .map(|point| PlotPoint {
                x: point.x,
                y: point.y,
                low: point.y,
                high: point.y,
                step: point.step,
            })
            .collect();
        return (plain, false);
    }
    let mut out = Vec::with_capacity(budget);
    for bucket in 0..budget {
        // Split by position so bucket widths stay even across the axis.
        let start = bucket * points.len() / budget;
        let end = ((bucket + 1) * points.len() / budget).max(start + 1);
        let slice = &points[start..end.min(points.len())];
        if slice.is_empty() {
            continue;
        }
        let count = slice.len() as f64;
        let mut low = f64::INFINITY;
        let mut high = f64::NEG_INFINITY;
        let mut sum_x = 0.0;
        let mut sum_y = 0.0;
        for point in slice {
            sum_x += point.x;
            sum_y += point.y;
            low = low.min(point.y);
            high = high.max(point.y);
        }
        out.push(PlotPoint {
            x: sum_x / count,
            y: sum_y / count,
            low,
            high,
            // The bucket's newest step, so a click lands on real telemetry.
            step: slice[slice.len() - 1].step,
        });
    }
    (out, true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChartXAxis {
    #[default]
    Step,
    Time,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChartPoint {
    pub x: f64,
    pub y: f64,
    pub step: i64,
    /// Value range behind this point when whoever produced it already summarised
    /// several readings. `None` for a raw reading, which is drawn without a band.
    ///
    /// Prefer a spread around `y` over the outright extremes: extremes widen as
    /// the producer coarsens its buckets, until the band covers the plot.
    pub band: Option<(f64, f64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChartSeries {
    pub label: String,
    pub points: Vec<ChartPoint>,
    pub color: &'static str,
    /// Preformatted latest value shown in the legend. Empty hides it.
    pub readout: String,
    /// Preformatted step-over-step change shown after the readout.
    pub delta: String,
}

impl ChartSeries {
    /// A series with no legend readout, for callers that only plot a line.
    pub fn plain(label: String, points: Vec<ChartPoint>, color: &'static str) -> Self {
        Self {
            label,
            points,
            color,
            readout: String::new(),
            delta: String::new(),
        }
    }
}

#[component]
pub fn MetricsLineChart(
    title: String,
    series: Vec<ChartSeries>,
    #[props(default = 220.0)] height: f64,
    #[props(default = ChartXAxis::Step)] x_axis: ChartXAxis,
    #[props(default = false)] log_scale: bool,
    #[props(default = String::new())] subtitle: String,
    /// Hover text for the chart title, used to explain what the metric measures.
    #[props(default = String::new())]
    tooltip: String,
    #[props(default = None)] on_point_click: Option<EventHandler<i64>>,
) -> Element {
    // Index into the anchor series of the point under the pointer. Declared before
    // the early returns below: a hook skipped on the render where there is no data
    // yet, then run once it arrives, shifts the hook order.
    let mut hovered = use_signal(|| None::<usize>);

    if series.is_empty() || series.iter().all(|s| s.points.is_empty()) {
        return rsx! {
            div { class: "rounded-lg border border-slate-200 bg-white p-3",
                div {
                    class: "text-sm font-medium text-slate-700 mb-1",
                    title: "{tooltip}",
                    "{title}"
                }
                if !subtitle.is_empty() {
                    div { class: "text-xs text-slate-500 mb-2", "{subtitle}" }
                }
                div { class: "text-xs text-slate-400 py-6 text-center", "No samples yet" }
            }
        };
    }

    let width = 640.0;
    let chart_height = height;
    let pad_left = 52.0;
    let pad_right = 16.0;
    let pad_top = 12.0;
    let pad_bottom = 32.0;
    let plot_w = width - pad_left - pad_right;
    let plot_h = chart_height - pad_top - pad_bottom;
    let step_axis = matches!(x_axis, ChartXAxis::Step);

    let plotted = if log_scale {
        series
            .iter()
            .map(|s| ChartSeries {
                label: s.label.clone(),
                points: s
                    .points
                    .iter()
                    .filter(|point| point.y > 0.0)
                    .map(|point| ChartPoint {
                        x: point.x,
                        y: point.y.ln(),
                        step: point.step,
                        band: point.band.and_then(|(low, high)| {
                            // A band crossing zero has no logarithm to draw.
                            (low > 0.0 && high > 0.0).then_some((low.ln(), high.ln()))
                        }),
                    })
                    .collect(),
                color: s.color,
                readout: s.readout.clone(),
                delta: s.delta.clone(),
            })
            .collect::<Vec<_>>()
    } else {
        series.clone()
    };
    let plotted = plotted
        .iter()
        .map(|s| {
            let (points, aggregated) = downsample(&s.points, PLOT_POINT_BUDGET);
            PlotSeries {
                label: s.label.clone(),
                color: s.color,
                readout: s.readout.clone(),
                delta: s.delta.clone(),
                points,
                aggregated,
            }
        })
        .collect::<Vec<_>>();

    if plotted.iter().all(|s| s.points.is_empty()) {
        return rsx! {
            div { class: "rounded-lg border border-slate-200 bg-white p-3",
                div {
                    class: "text-sm font-medium text-slate-700 mb-1",
                    title: "{tooltip}",
                    "{title}"
                }
                div { class: "text-xs text-slate-400 py-6 text-center", "No positive values for log scale" }
            }
        };
    }

    let (x_min, x_max, y_min, y_max) = bounds(&plotted);
    let x_span = (x_max - x_min).max(1.0);
    let y_span = (y_max - y_min).max(f64::EPSILON);
    let y_ticks = y_axis_ticks(y_min, y_max);
    let x_labels = x_axis_labels(x_min, x_max, 5, step_axis);
    let clickable = on_point_click.is_some();
    let hover = if clickable { "cursor-pointer" } else { "" };

    // The densest series carries the crosshair; the others report whichever of
    // their own points sits nearest it, since runs need not share an x set.
    let anchor = plotted
        .iter()
        .enumerate()
        .max_by_key(|(_, s)| s.points.len())
        .map(|(index, _)| index)
        .unwrap_or(0);
    let hovered_point = hovered()
        .and_then(|index| plotted.get(anchor)?.points.get(index).cloned())
        .filter(|_| plotted[anchor].points.len() > 1);

    rsx! {
        div { class: "rounded-lg border border-slate-200 bg-white p-3",
            div { class: "flex flex-wrap items-center justify-between gap-x-2 gap-y-1 mb-1",
                div {
                    class: "text-sm font-medium text-slate-700",
                    title: "{tooltip}",
                    "{title}"
                }
                div { class: "flex flex-wrap gap-3 text-xs text-slate-600",
                    if let Some(point) = hovered_point.as_ref() {
                        span { class: "font-medium text-slate-500 whitespace-nowrap",
                            "{crosshair_label(point, step_axis)}"
                        }
                    }
                    for (index, s) in series.iter().enumerate().filter(|(_, s)| !s.points.is_empty()) {
                        {
                            // Under the pointer the legend reads out that point rather
                            // than the newest one, which is the whole use of a crosshair.
                            let probed = hovered_point.as_ref().and_then(|at| {
                                value_nearest(plotted.get(index)?, at.x)
                            });
                            let readout = match probed {
                                Some(y) => format_tick(if log_scale { y.exp() } else { y }),
                                None => s.readout.clone(),
                            };
                            rsx! {
                                div { class: "flex items-center gap-1.5 whitespace-nowrap",
                                    span {
                                        class: "inline-block w-2.5 h-2.5 rounded-full shrink-0",
                                        style: "background-color: {s.color};",
                                    }
                                    span { "{s.label}" }
                                    if !readout.is_empty() {
                                        span { class: "font-medium text-slate-800", "{readout}" }
                                    }
                                    // A delta against the previous point means nothing
                                    // once the readout is a probed point, not the last.
                                    if probed.is_none() && !s.delta.is_empty() {
                                        span { class: "text-slate-400", "{s.delta}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if !subtitle.is_empty() {
                div { class: "text-xs text-slate-500 mb-2", "{subtitle}" }
            }
            if clickable {
                div { class: "mb-1 text-xs text-slate-400", "Click a point to open samples at that step." }
            }
            svg {
                class: "w-full {hover}",
                view_box: "0 0 {width} {chart_height}",
                preserve_aspect_ratio: "none",
                onmouseleave: move |_| hovered.set(None),
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
                        let tick_label = if log_scale {
                            format_tick(tick.exp())
                        } else {
                            format_tick(*tick)
                        };
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
                                    "{tick_label}"
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
                // Spread within each bucket, drawn first so the mean line sits on top.
                for s in plotted.iter().filter(|s| s.aggregated && s.points.len() >= 2) {
                    {
                        let to_y = |value: f64| {
                            pad_top + plot_h - ((value - y_min) / y_span * plot_h)
                        };
                        let mut area = String::new();
                        for point in s.points.iter() {
                            let px = pad_left + ((point.x - x_min) / x_span * plot_w);
                            area.push_str(&format!("{px},{} ", to_y(point.high)));
                        }
                        for point in s.points.iter().rev() {
                            let px = pad_left + ((point.x - x_min) / x_span * plot_w);
                            area.push_str(&format!("{px},{} ", to_y(point.low)));
                        }
                        rsx! {
                            polygon {
                                points: "{area.trim()}",
                                fill: "{s.color}",
                                fill_opacity: "0.16",
                                stroke: "none",
                            }
                        }
                    }
                }
                for s in plotted.iter().filter(|s| s.points.len() >= 2) {
                    {
                        let polyline = s
                            .points
                            .iter()
                            .map(|point| {
                                let px = pad_left + ((point.x - x_min) / x_span * plot_w);
                                let py = pad_top + plot_h - ((point.y - y_min) / y_span * plot_h);
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
                for s in plotted.iter() {
                    {
                        // A marker per point turns into a smear once a run is a few
                        // hundred steps long, so past that only the line is drawn.
                        // Clicking is handled by the full-height strips below.
                        let total = s.points.len();
                        let show_markers = total <= MARKER_LIMIT;
                        rsx! {
                            for (index, point) in s.points.iter().enumerate() {
                                {
                                    let px = pad_left + ((point.x - x_min) / x_span * plot_w);
                                    let py = pad_top + plot_h - ((point.y - y_min) / y_span * plot_h);
                                    let is_last = index + 1 == total;
                                    rsx! {
                                        if show_markers || is_last {
                                            circle {
                                                cx: "{px}",
                                                cy: "{py}",
                                                r: "3.0",
                                                fill: "{s.color}",
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                // Crosshair, over the lines. The strips span the plot height because a
                // marker-sized target only answers when the pointer is already on the
                // line, which is not where a reader looks for a value.
                if let Some(at) = hovered_point.as_ref() {
                    {
                        let cx = pad_left + ((at.x - x_min) / x_span * plot_w);
                        rsx! {
                            line {
                                x1: "{cx}",
                                y1: "{pad_top}",
                                x2: "{cx}",
                                y2: "{pad_top + plot_h}",
                                stroke: "#94a3b8",
                                stroke_width: "1",
                                stroke_dasharray: "3 2",
                            }
                            for s in plotted.iter() {
                                if let Some(y) = value_nearest(s, at.x) {
                                    circle {
                                        cx: "{cx}",
                                        cy: "{pad_top + plot_h - ((y - y_min) / y_span * plot_h)}",
                                        r: "3.5",
                                        fill: "{s.color}",
                                        stroke: "#ffffff",
                                        stroke_width: "1.5",
                                    }
                                }
                            }
                        }
                    }
                }
                if plotted[anchor].points.len() > 1 {
                    {
                        let points = &plotted[anchor].points;
                        let to_x = |value: f64| pad_left + ((value - x_min) / x_span * plot_w);
                        rsx! {
                            for (index, point) in points.iter().enumerate() {
                                {
                                    // Bounded by the midpoints to its neighbours, so the
                                    // strip a pointer lands in is the nearest point even
                                    // when the x values are not evenly spaced.
                                    let px = to_x(point.x);
                                    let left = match index.checked_sub(1) {
                                        Some(prev) => (to_x(points[prev].x) + px) / 2.0,
                                        None => pad_left,
                                    };
                                    let right = match points.get(index + 1) {
                                        Some(next) => (px + to_x(next.x)) / 2.0,
                                        None => pad_left + plot_w,
                                    };
                                    let step = point.step;
                                    rsx! {
                                        rect {
                                            x: "{left}",
                                            y: "{pad_top}",
                                            width: "{(right - left).max(0.0)}",
                                            height: "{plot_h}",
                                            fill: "transparent",
                                            onmouseenter: move |_| hovered.set(Some(index)),
                                            onclick: move |_| {
                                                if let Some(handler) = on_point_click {
                                                    handler.call(step);
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

/// Where the crosshair sits, named so a bare number cannot be mistaken for one of
/// the values beside it.
fn crosshair_label(point: &PlotPoint, step_axis: bool) -> String {
    if step_axis {
        format!("step {}", point.step)
    } else {
        format_x(point.x, step_axis)
    }
}

/// Value of the point in `series` nearest `x`, for reporting what a crosshair
/// placed elsewhere sits on. `None` only when the series is empty.
fn value_nearest(series: &PlotSeries, x: f64) -> Option<f64> {
    series
        .points
        .iter()
        .min_by(|a, b| {
            (a.x - x)
                .abs()
                .partial_cmp(&(b.x - x).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|point| point.y)
}

fn bounds(series: &[PlotSeries]) -> (f64, f64, f64, f64) {
    let mut x_min = f64::INFINITY;
    let mut x_max = f64::NEG_INFINITY;
    let mut y_min = f64::INFINITY;
    let mut y_max = f64::NEG_INFINITY;

    for s in series {
        for point in &s.points {
            x_min = x_min.min(point.x);
            x_max = x_max.max(point.x);
            // Bound the range band too, so it cannot spill past the axis.
            y_min = y_min.min(point.low);
            y_max = y_max.max(point.high);
        }
    }

    if y_min == y_max {
        let pad = if y_min.abs() > 1.0 {
            y_min.abs() * 0.1
        } else {
            1.0
        };
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

fn x_axis_labels(x_min: f64, x_max: f64, count: usize, step_axis: bool) -> Vec<(String, f64)> {
    if count <= 1 {
        return vec![(format_x(x_min, step_axis), x_min)];
    }
    (0..count)
        .map(|i| {
            let x = x_min + (x_max - x_min) * (i as f64 / (count - 1) as f64);
            (format_x(x, step_axis), x)
        })
        .collect()
}

fn format_x(value: f64, step_axis: bool) -> String {
    if step_axis {
        format!("{value:.0}")
    } else if value >= 1_000_000_000.0 {
        // nanoseconds wall timestamps from rl.metric
        format_time_ns(value)
    } else if value >= 1_000.0 {
        format_time_ms(value)
    } else {
        format!("{value:.1}s")
    }
}

fn format_time_ns(ts_ns: f64) -> String {
    let secs = (ts_ns / 1_000_000_000.0) as i64;
    let nanos = (ts_ns % 1_000_000_000.0) as u32;
    chrono::DateTime::from_timestamp(secs, nanos)
        .map(|dt| dt.format("%H:%M:%S").to_string())
        .unwrap_or_else(|| format!("{ts_ns:.0}"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_x_uses_step_labels() {
        assert_eq!(format_x(42.0, true), "42");
    }

    fn ramp(count: usize) -> Vec<ChartPoint> {
        (0..count)
            .map(|index| ChartPoint {
                x: index as f64,
                y: index as f64,
                step: index as i64,
                ..Default::default()
            })
            .collect()
    }

    #[test]
    fn short_series_are_drawn_point_for_point() {
        let (points, aggregated) = downsample(&ramp(50), PLOT_POINT_BUDGET);
        assert!(!aggregated);
        assert_eq!(points.len(), 50);
        assert_eq!(points[7].y, 7.0);
        // Nothing to spread, so the band collapses onto the line.
        assert_eq!(points[7].low, points[7].high);
    }

    #[test]
    fn a_crosshair_reads_the_nearest_point_of_a_series_that_misses_its_x() {
        // A baseline run recorded on its own steps: the crosshair lands between two
        // of its points and must still report one of them, not nothing.
        let sparse = PlotSeries {
            label: "baseline".into(),
            color: "#000",
            readout: String::new(),
            delta: String::new(),
            points: vec![
                PlotPoint {
                    x: 0.0,
                    y: 1.0,
                    low: 1.0,
                    high: 1.0,
                    step: 0,
                },
                PlotPoint {
                    x: 100.0,
                    y: 5.0,
                    low: 5.0,
                    high: 5.0,
                    step: 100,
                },
            ],
            aggregated: false,
        };
        assert_eq!(value_nearest(&sparse, 40.0), Some(1.0));
        assert_eq!(value_nearest(&sparse, 60.0), Some(5.0));
    }

    #[test]
    fn a_crosshair_names_the_step_it_sits_on() {
        // A step, not the x coordinate: on a bucketed series the two differ, and the
        // step is what a reader can act on.
        let at = PlotPoint {
            x: 5176.0,
            y: 0.9,
            low: 0.9,
            high: 0.9,
            step: 5182,
        };
        assert_eq!(crosshair_label(&at, true), "step 5182");
        assert_eq!(crosshair_label(&at, false), format_x(5176.0, false));
    }

    #[test]
    fn a_crosshair_over_an_empty_series_reports_nothing() {
        let empty = PlotSeries {
            label: "empty".into(),
            color: "#000",
            readout: String::new(),
            delta: String::new(),
            points: Vec::new(),
            aggregated: false,
        };
        assert_eq!(value_nearest(&empty, 0.0), None);
    }

    #[test]
    fn an_upstream_band_is_kept_rather_than_re_bucketed() {
        // What the server's bucketed series endpoint sends: each point already
        // summarises a step range, so averaging them again would both lose the
        // true min/max and shift the line.
        let points = (0..10)
            .map(|index| ChartPoint {
                x: index as f64,
                y: index as f64,
                step: index as i64,
                band: Some((index as f64 - 1.0, index as f64 + 1.0)),
            })
            .collect::<Vec<_>>();
        let (plotted, aggregated) = downsample(&points, 4);
        assert!(aggregated, "an upstream band means the line is a summary");
        // Kept as sent, even though there are more points than the budget.
        assert_eq!(plotted.len(), 10);
        assert_eq!(plotted[3].y, 3.0);
        assert_eq!(plotted[3].low, 2.0);
        assert_eq!(plotted[3].high, 4.0);
    }

    #[test]
    fn long_series_collapse_to_the_budget_and_span_the_axis() {
        let (points, aggregated) = downsample(&ramp(1548), PLOT_POINT_BUDGET);
        assert!(aggregated);
        assert_eq!(points.len(), PLOT_POINT_BUDGET);
        // Full range is kept: no truncation to a recent window.
        assert_eq!(points[0].low, 0.0);
        assert_eq!(points[PLOT_POINT_BUDGET - 1].high, 1547.0);
        assert_eq!(points[PLOT_POINT_BUDGET - 1].step, 1547);
        // Buckets stay in order and cover every input.
        assert!(points.windows(2).all(|pair| pair[0].x < pair[1].x));
    }

    #[test]
    fn aggregation_keeps_spikes_in_the_range_band() {
        // One tall spike among flat zeros: averaging hides it, the band must not.
        let mut points = vec![
            ChartPoint {
                x: 0.0,
                y: 0.0,
                step: 0,
                ..Default::default()
            };
            1000
        ];
        for (index, point) in points.iter_mut().enumerate() {
            point.x = index as f64;
            point.step = index as i64;
        }
        points[500].y = 9.0;
        let (plotted, _) = downsample(&points, PLOT_POINT_BUDGET);
        let spike = plotted
            .iter()
            .find(|point| point.high > 1.0)
            .expect("spike survives aggregation");
        assert_eq!(spike.high, 9.0);
        assert_eq!(spike.low, 0.0);
        assert!(spike.y < 9.0, "mean is pulled down by the flat neighbours");
    }

    #[test]
    fn a_single_point_survives_downsampling() {
        let (points, aggregated) = downsample(&ramp(1), 1);
        assert!(!aggregated);
        assert_eq!(points.len(), 1);
        assert!(downsample(&[], PLOT_POINT_BUDGET).0.is_empty());
    }
}
