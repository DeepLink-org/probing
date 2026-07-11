//! TorchProbe overhead domain logic (metrics, parsing, labels).

use probing_proto::prelude::DataFrame;

use crate::overhead::sql::WINDOW_STEPS;
use crate::utils::error::AppError;

#[derive(Clone, Debug, PartialEq)]
pub struct OverheadStep {
    pub local_step: i64,
    pub duration_ms: f64,
    pub is_shadow: bool,
    pub sampled: bool,
}

pub fn table_missing_message(err: &AppError) -> Option<&'static str> {
    let msg = err.display_message().to_ascii_lowercase();
    if msg.contains("torch_step_timing")
        && (msg.contains("not found") || msg.contains("does not exist"))
    {
        Some(
            "Table python.torch_step_timing is not registered yet — enable SET probing.torch.profiling=on and wait for the first training steps.",
        )
    } else {
        None
    }
}

pub fn table_missing_trigger_label(err: &AppError) -> Option<String> {
    table_missing_message(err).map(|_| "Profiler off".to_string())
}

pub fn overhead_trigger_label(summary: &DataFrame) -> String {
    let shadow_baseline = summary.scalar_i64("shadow_baseline", 0).unwrap_or(0);
    if shadow_baseline == 0 {
        return "Shadow off".to_string();
    }
    let probed_n = summary.scalar_i64("probed_n", 0).unwrap_or(0);
    let shadow_n = summary.scalar_i64("shadow_n", 0).unwrap_or(0);
    if probed_n == 0 || shadow_n == 0 {
        return "Collecting baseline…".to_string();
    }
    let probed_median_ms = summary.scalar_f64("probed_median_ms", 0);
    let shadow_median_ms = summary.scalar_f64("shadow_median_ms", 0);
    match (probed_median_ms, shadow_median_ms) {
        (Some(p), Some(s)) => {
            let (hook_tax, _) = format_overhead_pct(p, s, "Hook tax");
            let latest = summary.scalar_i64("latest_step", 0).unwrap_or(-1);
            format!("{hook_tax} · step {latest}")
        }
        _ => "Collecting baseline…".to_string(),
    }
}

pub fn parse_overhead_steps(df: &DataFrame) -> Vec<OverheadStep> {
    let mut steps = Vec::new();
    let rows = df.row_count();
    for row in 0..rows {
        let Some(local_step) = df.scalar_i64("local_step", row) else {
            continue;
        };
        let Some(duration_ms) = df.scalar_f64("duration_ms", row) else {
            continue;
        };
        steps.push(OverheadStep {
            local_step,
            duration_ms,
            is_shadow: df.scalar_boolish("is_shadow", row),
            sampled: df.scalar_boolish("sampled", row),
        });
    }
    steps
}

pub fn format_step_ms(ms: f64) -> String {
    if ms >= 100.0 {
        format!("{ms:.0} ms")
    } else if ms >= 1.0 {
        format!("{ms:.1} ms")
    } else if ms > 0.0 {
        format!("{ms:.2} ms")
    } else {
        "0 ms".to_string()
    }
}

pub fn format_overhead_pct(
    probed_ms: f64,
    shadow_ms: f64,
    context: &str,
) -> (String, Option<String>) {
    if shadow_ms <= 0.0 {
        return (
            "—".to_string(),
            Some("Need shadow baseline steps".to_string()),
        );
    }
    let pct = (probed_ms / shadow_ms - 1.0) * 100.0;
    let hint = Some(format!("{context} · last {WINDOW_STEPS} steps"));
    if pct.abs() < 0.5 {
        ("≈0%".to_string(), hint)
    } else if pct > 0.0 {
        (format!("+{pct:.1}%"), hint)
    } else {
        (format!("{pct:.1}%"), hint)
    }
}

#[inline]
pub fn dataframe_rows(df: &DataFrame) -> usize {
    df.row_count()
}

#[inline]
pub fn df_scalar_f64(df: &DataFrame, col: &str, row: usize) -> Option<f64> {
    df.scalar_f64(col, row)
}

#[inline]
pub fn df_scalar_i64(df: &DataFrame, col: &str, row: usize) -> Option<i64> {
    df.scalar_i64(col, row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overhead_pct_positive() {
        let (value, _) = format_overhead_pct(133.0, 100.0, "test");
        assert_eq!(value, "+33.0%");
    }

    #[test]
    fn overhead_pct_near_zero() {
        let (value, _) = format_overhead_pct(100.2, 100.0, "test");
        assert_eq!(value, "≈0%");
    }
}
