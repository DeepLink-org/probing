//! TorchProbe overhead domain logic (metrics, parsing, labels).

use probing_proto::prelude::{DataFrame, Ele};

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
    let shadow_baseline = df_scalar_i64(summary, "shadow_baseline", 0).unwrap_or(0);
    if shadow_baseline == 0 {
        return "Shadow off".to_string();
    }
    let probed_n = df_scalar_i64(summary, "probed_n", 0).unwrap_or(0);
    let shadow_n = df_scalar_i64(summary, "shadow_n", 0).unwrap_or(0);
    if probed_n == 0 || shadow_n == 0 {
        return "Collecting baseline…".to_string();
    }
    let probed_median_ms = df_scalar_f64(summary, "probed_median_ms", 0);
    let shadow_median_ms = df_scalar_f64(summary, "shadow_median_ms", 0);
    match (probed_median_ms, shadow_median_ms) {
        (Some(p), Some(s)) => {
            let (hook_tax, _) = format_overhead_pct(p, s, "Hook tax");
            let latest = df_scalar_i64(summary, "latest_step", 0).unwrap_or(-1);
            format!("{hook_tax} · step {latest}")
        }
        _ => "Collecting baseline…".to_string(),
    }
}

pub fn parse_overhead_steps(df: &DataFrame) -> Vec<OverheadStep> {
    let mut steps = Vec::new();
    let rows = dataframe_rows(df);
    for row in 0..rows {
        let Some(local_step) = df_scalar_i64(df, "local_step", row) else {
            continue;
        };
        let Some(duration_ms) = df_scalar_f64(df, "duration_ms", row) else {
            continue;
        };
        let is_shadow = df_col_index(df, "is_shadow")
            .and_then(|ci| df.cols.get(ci))
            .filter(|col| row < col.len())
            .map(|col| ele_boolish(&col.get(row)))
            .unwrap_or(false);
        let sampled = df_col_index(df, "sampled")
            .and_then(|ci| df.cols.get(ci))
            .filter(|col| row < col.len())
            .map(|col| ele_boolish(&col.get(row)))
            .unwrap_or(false);
        steps.push(OverheadStep {
            local_step,
            duration_ms,
            is_shadow,
            sampled,
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

pub fn dataframe_rows(df: &DataFrame) -> usize {
    df.cols.first().map(|c| c.len()).unwrap_or(0)
}

pub fn df_scalar_f64(df: &DataFrame, col: &str, row: usize) -> Option<f64> {
    let ci = df_col_index(df, col)?;
    let column = df.cols.get(ci)?;
    if row >= column.len() {
        return None;
    }
    ele_f64(&column.get(row))
}

pub fn df_scalar_i64(df: &DataFrame, col: &str, row: usize) -> Option<i64> {
    let ci = df_col_index(df, col)?;
    let column = df.cols.get(ci)?;
    if row >= column.len() {
        return None;
    }
    ele_i64(&column.get(row))
}

fn df_col_index(df: &DataFrame, name: &str) -> Option<usize> {
    df.names.iter().position(|n| n == name)
}

fn ele_f64(ele: &Ele) -> Option<f64> {
    match ele {
        Ele::F64(x) => Some(*x),
        Ele::F32(x) => Some(*x as f64),
        Ele::I64(x) => Some(*x as f64),
        Ele::I32(x) => Some(*x as f64),
        Ele::Text(s) => s.parse().ok(),
        _ => None,
    }
}

fn ele_i64(ele: &Ele) -> Option<i64> {
    match ele {
        Ele::I64(x) => Some(*x),
        Ele::I32(x) => Some(*x as i64),
        Ele::F64(x) => Some(*x as i64),
        Ele::Text(s) => s.parse().ok(),
        _ => None,
    }
}

fn ele_boolish(ele: &Ele) -> bool {
    match ele {
        Ele::BOOL(x) => *x,
        Ele::I64(x) => *x != 0,
        Ele::I32(x) => *x != 0,
        Ele::Text(s) => matches!(s.as_str(), "1" | "true" | "True"),
        _ => false,
    }
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
