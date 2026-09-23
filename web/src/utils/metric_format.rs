//! Unit-aware rendering for canonical RL metric names.
//!
//! The canonical vocabulary mixes durations, 0-1 ratios, token counts, GiB, and
//! plain gauges. Rendering them all through one numeric formatter loses the
//! unit, so the unit is inferred from the metric name once and reused for both
//! the value and its step-over-step delta.

/// The unit a canonical metric is expressed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricUnit {
    /// Seconds, rendered as `6h 26m`; deltas keep the same shape.
    Duration,
    /// A 0-1 fraction, rendered as `13.2 %`; deltas are percentage points.
    Ratio,
    /// A large tally, rendered as `3.43B`.
    Count,
    /// Gibibytes of accelerator memory.
    Gibibytes,
    /// US dollars.
    Money,
    /// Anything else: losses, entropies, KLs, multipliers.
    Plain,
}

/// Ratio-looking names that are actually multipliers, not 0-1 fractions.
const MULTIPLIER_SUFFIXES: [&str; 2] = ["ppl_ratio", "p99_p50_ratio"];

const RATIO_SUFFIXES: [&str; 3] = ["_ratio", "_frac", ".share"];

const RATIO_NAMES: [&str; 2] = ["policy.clip_frac_high", "policy.clip_frac_low"];

const COUNT_SUFFIXES: [&str; 8] = [
    ".tokens",
    "_tokens",
    "_tokens_s",
    "_samples_s",
    "_count",
    "_steps",
    "_prompts",
    "_total",
];

const COUNT_NAMES: [&str; 2] = ["sampler.batch_size", "environment.active"];

/// Infer how `name` should be rendered.
pub fn metric_unit(name: &str) -> MetricUnit {
    if name.starts_with("cost.usd") {
        return MetricUnit::Money;
    }
    if name.ends_with("_gb") {
        return MetricUnit::Gibibytes;
    }
    if name.ends_with("_s") && !name.ends_with("_tokens_s") && !name.ends_with("_samples_s") {
        return MetricUnit::Duration;
    }
    if MULTIPLIER_SUFFIXES
        .iter()
        .any(|suffix| name.ends_with(suffix))
    {
        return MetricUnit::Plain;
    }
    if RATIO_NAMES.contains(&name) || RATIO_SUFFIXES.iter().any(|suffix| name.ends_with(suffix)) {
        return MetricUnit::Ratio;
    }
    if COUNT_NAMES.contains(&name) || COUNT_SUFFIXES.iter().any(|suffix| name.ends_with(suffix)) {
        return MetricUnit::Count;
    }
    MetricUnit::Plain
}

/// Render `value` with the unit implied by `name`.
pub fn format_metric_value(name: &str, value: f64) -> String {
    if !value.is_finite() {
        return "—".to_string();
    }
    match metric_unit(name) {
        MetricUnit::Duration => format_duration_secs(value),
        MetricUnit::Ratio => format!("{:.1} %", value * 100.0),
        MetricUnit::Count => format_compact(value),
        MetricUnit::Gibibytes => format!("{value:.1} GB"),
        MetricUnit::Money => format_money(value),
        MetricUnit::Plain => format_plain(value),
    }
}

/// Render a step-over-step change, with an arrow and the unit's delta form.
pub fn format_metric_delta(name: &str, delta: f64) -> String {
    if !delta.is_finite() {
        return String::new();
    }
    let unit = metric_unit(name);
    let magnitude = match unit {
        MetricUnit::Duration => format_duration_secs(delta.abs()),
        // Percentage points, not percent of the previous value.
        MetricUnit::Ratio => format!("{:.2} pt", delta.abs() * 100.0),
        MetricUnit::Count => format_compact(delta.abs()),
        MetricUnit::Gibibytes => format!("{:.1} GB", delta.abs()),
        MetricUnit::Money => format_money(delta.abs()),
        MetricUnit::Plain => format_plain(delta.abs()),
    };
    if is_negligible(unit, delta) {
        return format!("±{magnitude}");
    }
    let arrow = if delta > 0.0 { "▲" } else { "▼" };
    format!("{arrow}{magnitude}")
}

fn is_negligible(unit: MetricUnit, delta: f64) -> bool {
    let epsilon = match unit {
        MetricUnit::Duration => 0.5,
        MetricUnit::Ratio => 0.000_05,
        MetricUnit::Count => 0.5,
        MetricUnit::Gibibytes => 0.05,
        MetricUnit::Money => 0.005,
        MetricUnit::Plain => 1e-9,
    };
    delta.abs() < epsilon
}

/// `6h 26m` / `3m 20s` / `0.45s`, matching how a reader reads step timings.
pub fn format_duration_secs(seconds: f64) -> String {
    if !seconds.is_finite() {
        return "—".to_string();
    }
    let total = seconds.abs();
    if total >= 3600.0 {
        let hours = (total / 3600.0).floor();
        let minutes = ((total - hours * 3600.0) / 60.0).floor();
        format!("{hours:.0}h {minutes:.0}m")
    } else if total >= 60.0 {
        let minutes = (total / 60.0).floor();
        let rest = total - minutes * 60.0;
        format!("{minutes:.0}m {rest:.0}s")
    } else if total >= 1.0 {
        format!("{total:.1}s")
    } else {
        format!("{:.0}ms", total * 1000.0)
    }
}

/// `3.43B` / `137k` / `842`.
pub fn format_compact(value: f64) -> String {
    let magnitude = value.abs();
    if magnitude >= 1e9 {
        format!("{:.2}B", value / 1e9)
    } else if magnitude >= 1e6 {
        format!("{:.2}M", value / 1e6)
    } else if magnitude >= 1e4 {
        format!("{:.1}k", value / 1e3)
    } else if magnitude >= 1.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}

fn format_money(value: f64) -> String {
    if value.abs() >= 1_000.0 {
        format!("${value:.0}")
    } else {
        format!("${value:.2}")
    }
}

fn format_plain(value: f64) -> String {
    let magnitude = value.abs();
    if magnitude >= 1_000.0 {
        format!("{value:.0}")
    } else if magnitude >= 10.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.4}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_render_as_hours_and_minutes() {
        assert_eq!(metric_unit("time.step_s"), MetricUnit::Duration);
        assert_eq!(format_metric_value("time.step_s", 23_186.0), "6h 26m");
        assert_eq!(format_metric_value("progress.eta_s", 142_620.0), "39h 37m");
        assert_eq!(format_metric_value("time.save_ckpt_s", 0.486), "486ms");
        assert_eq!(format_metric_value("sampler.task_p50_s", 3.52), "3.5s");
        assert_eq!(format_metric_delta("time.step_s", 6_420.0), "▲1h 47m");
    }

    #[test]
    fn ratios_render_as_percent_with_point_deltas() {
        assert_eq!(metric_unit("sampler.pass_zero_ratio"), MetricUnit::Ratio);
        assert_eq!(
            format_metric_value("sampler.pass_zero_ratio", 0.132),
            "13.2 %"
        );
        assert_eq!(
            format_metric_delta("sampler.pass_zero_ratio", 0.007),
            "▲0.70 pt"
        );
        assert_eq!(metric_unit("policy.clip_frac_high"), MetricUnit::Ratio);
        assert_eq!(metric_unit("progress.completed_ratio"), MetricUnit::Ratio);
    }

    #[test]
    fn ratio_named_multipliers_stay_plain() {
        // These are ratios of two quantities, not shares of a whole.
        assert_eq!(
            metric_unit("policy.train_infer_ppl_ratio"),
            MetricUnit::Plain
        );
        assert_eq!(metric_unit("sampler.task_p99_p50_ratio"), MetricUnit::Plain);
        assert_eq!(
            format_metric_value("sampler.task_p99_p50_ratio", 1.2251),
            "1.2251"
        );
    }

    #[test]
    fn counts_render_compact() {
        assert_eq!(metric_unit("train.tokens"), MetricUnit::Count);
        assert_eq!(format_metric_value("train.tokens", 3.43e9), "3.43B");
        assert_eq!(
            format_metric_value("throughput.e2e_tokens_s", 137_000.0),
            "137.0k"
        );
        assert_eq!(format_metric_delta("train.tokens", 1.0e7), "▲10.00M");
        assert_eq!(metric_unit("progress.total_steps"), MetricUnit::Count);
    }

    #[test]
    fn throughput_is_a_rate_not_a_duration() {
        // `_tokens_s` and `_samples_s` end in `_s` but are per-second rates.
        assert_eq!(metric_unit("throughput.e2e_tokens_s"), MetricUnit::Count);
        assert_eq!(
            metric_unit("throughput.rollout_samples_s"),
            MetricUnit::Count
        );
        assert_eq!(metric_unit("environment.queue_s"), MetricUnit::Duration);
    }

    #[test]
    fn memory_and_money_keep_their_units() {
        assert_eq!(
            format_metric_value("hardware.max_memory_gb", 58.43),
            "58.4 GB"
        );
        assert_eq!(
            format_metric_value("cost.usd_total", 2_620_670.0),
            "$2620670"
        );
        assert_eq!(format_metric_value("cost.usd_per_hour", 640.5), "$640.50");
    }

    #[test]
    fn unchanged_values_report_no_direction() {
        assert_eq!(format_metric_delta("reward.mean", 0.0), "±0.0000");
        assert_eq!(format_metric_delta("time.step_s", 0.1), "±100ms");
    }

    #[test]
    fn non_finite_values_degrade_to_placeholder() {
        assert_eq!(format_metric_value("reward.mean", f64::NAN), "—");
        assert_eq!(format_metric_delta("reward.mean", f64::INFINITY), "");
    }
}
