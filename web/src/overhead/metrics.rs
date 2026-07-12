//! TorchProbe overhead domain logic (metrics, parsing, labels).

use probing_proto::prelude::DataFrame;

use crate::overhead::sql::{MIN_DISPATCH_SAMPLES, MIN_SHADOW_SAMPLES, WINDOW_STEPS};
use crate::utils::error::AppError;

#[derive(Clone, Debug, PartialEq)]
pub struct OverheadStep {
    pub local_step: i64,
    pub duration_ms: f64,
    pub is_shadow: bool,
    pub sampled: bool,
}

/// Parsed summary row: raw observed medians + derived overhead percentages.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OverheadSnapshot {
    pub latest_step: i64,
    pub shadow_baseline: i64,
    pub shadow_normal: i64,
    pub shadow_median_ms: Option<f64>,
    pub dispatch_median_ms: Option<f64>,
    pub sampled_median_ms: Option<f64>,
    pub probed_median_ms: Option<f64>,
    pub probed_mean_ms: Option<f64>,
    pub shadow_mean_ms: Option<f64>,
    pub shadow_n: i64,
    pub dispatch_n: i64,
    pub sampled_n: i64,
    pub probed_n: i64,
    pub dispatch_overhead_pct: Option<f64>,
    pub sampled_overhead_pct: Option<f64>,
    pub blended_overhead_pct: Option<f64>,
    pub amortized_overhead_pct: Option<f64>,
    pub sample_rate: Option<f64>,
    pub sample_mode: Option<String>,
    /// `train.step` span median (compute-only); not from timing table.
    pub train_step_median_ms: Option<f64>,
}

/// Qualitative band for profiler overhead (dispatch path).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverheadLevel {
    Low,
    Moderate,
    High,
    Unknown,
}

impl OverheadLevel {
    pub fn from_pct(pct: Option<f64>) -> Self {
        match pct {
            None => Self::Unknown,
            Some(p) if p < 5.0 => Self::Low,
            Some(p) if p < 15.0 => Self::Moderate,
            Some(_) => Self::High,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Within normal range",
            Self::Moderate => "Moderately elevated",
            Self::High => "High — tune sample rate or shadow cadence",
            Self::Unknown => "Estimating…",
        }
    }

    pub fn is_reassuring(self) -> bool {
        matches!(self, Self::Low)
    }
}

/// Compact copy for the left sidebar monitor row.
#[derive(Clone, Debug, PartialEq)]
pub struct SidebarOverheadCopy {
    pub headline: String,
    pub performance: String,
    pub overhead: String,
    pub muted: bool,
}

impl OverheadSnapshot {
    pub fn from_summary(df: &DataFrame) -> Self {
        let shadow_baseline = df.scalar_i64("shadow_baseline", 0).unwrap_or(0);
        let shadow_n = df.scalar_i64("shadow_n", 0).unwrap_or(0);
        let dispatch_n = df.scalar_i64("dispatch_n", 0).unwrap_or(0);
        let shadow_median_ms = df.scalar_f64("shadow_median_ms", 0);
        let dispatch_median_ms = df.scalar_f64("dispatch_median_ms", 0);
        let sampled_median_ms = df.scalar_f64("sampled_median_ms", 0);
        let probed_median_ms = df.scalar_f64("probed_median_ms", 0);
        let probed_mean_ms = df.scalar_f64("probed_mean_ms", 0);
        let shadow_mean_ms = df.scalar_f64("shadow_mean_ms", 0);
        let sampled_n = df.scalar_i64("sampled_n", 0).unwrap_or(0);

        let stable = is_stable_sample(shadow_n, dispatch_n, shadow_baseline);
        let dispatch_overhead_pct = stable
            .then(|| overhead_pct_raw(dispatch_median_ms?, shadow_median_ms?))
            .flatten();
        let blended_overhead_pct = stable
            .then(|| overhead_pct_raw(probed_median_ms?, shadow_median_ms?))
            .flatten();
        let sampled_overhead_pct = (sampled_n > 0)
            .then(|| overhead_pct_raw(sampled_median_ms?, shadow_median_ms?))
            .flatten();
        let amortized_overhead_pct = amortized_overhead_pct(
            dispatch_overhead_pct,
            sampled_overhead_pct,
            blended_overhead_pct,
            df.scalar_f64("sample_rate", 0),
            sampled_n,
            df.scalar_i64("probed_n", 0).unwrap_or(0),
        );

        let sample_mode = df
            .names
            .iter()
            .position(|n| n == "sample_mode")
            .and_then(|ci| {
                df.cols.get(ci).filter(|col| !col.is_empty()).map(|col| {
                    use probing_proto::prelude::Ele;
                    match &col.get(0) {
                        Ele::Text(s) => s.clone(),
                        other => format!("{other:?}"),
                    }
                })
            });

        Self {
            latest_step: df.scalar_i64("latest_step", 0).unwrap_or(-1),
            shadow_baseline,
            shadow_normal: df.scalar_i64("shadow_normal", 0).unwrap_or(4),
            shadow_median_ms,
            dispatch_median_ms,
            sampled_median_ms,
            probed_median_ms,
            probed_mean_ms,
            shadow_mean_ms,
            shadow_n,
            dispatch_n,
            sampled_n,
            probed_n: df.scalar_i64("probed_n", 0).unwrap_or(0),
            dispatch_overhead_pct,
            sampled_overhead_pct,
            blended_overhead_pct,
            amortized_overhead_pct,
            sample_rate: df.scalar_f64("sample_rate", 0),
            sample_mode,
            train_step_median_ms: None,
        }
    }

    pub fn with_train_step_median(mut self, ms: Option<f64>) -> Self {
        self.train_step_median_ms = ms;
        self
    }

    pub fn is_stable(&self) -> bool {
        is_stable_sample(self.shadow_n, self.dispatch_n, self.shadow_baseline)
    }

    pub fn shadow_enabled(&self) -> bool {
        self.shadow_baseline > 0
    }

    pub fn cadence_label(&self) -> String {
        format!("{}:{}", self.shadow_normal, self.shadow_baseline)
    }

    pub fn window_label(&self) -> String {
        format!("last {WINDOW_STEPS} steps")
    }

    pub fn sampling_label(&self) -> String {
        match (&self.sample_mode, self.sample_rate) {
            (Some(mode), Some(rate)) => format!("{mode} @ {:.0}%", rate * 100.0),
            (Some(mode), None) => mode.clone(),
            (_, Some(rate)) => format!("{:.0}% step rate", rate * 100.0),
            _ => "—".to_string(),
        }
    }

    pub fn dispatch_level(&self) -> OverheadLevel {
        OverheadLevel::from_pct(self.dispatch_overhead_pct)
    }

    pub fn sidebar_copy(&self) -> SidebarOverheadCopy {
        if !self.shadow_enabled() {
            return SidebarOverheadCopy {
                headline: "Torch overhead".to_string(),
                performance: if self.latest_step > 0 {
                    format!("Step {}", self.latest_step)
                } else {
                    "Profiler on".to_string()
                },
                overhead: "Shadow off".to_string(),
                muted: true,
            };
        }

        if self.shadow_n == 0 || self.probed_n == 0 {
            return SidebarOverheadCopy {
                headline: format!("Step {}", self.latest_step.max(0)),
                performance: "Waiting for steps…".to_string(),
                overhead: "Collecting baseline".to_string(),
                muted: true,
            };
        }

        if !self.is_stable() {
            return SidebarOverheadCopy {
                headline: format!("Step {}", self.latest_step),
                performance: self
                    .shadow_median_ms
                    .map(|ms| format!("{} baseline", format_step_ms(ms)))
                    .unwrap_or_else(|| "Measuring…".to_string()),
                overhead: format!(
                    "Warming up ({}/{} shadow)",
                    self.shadow_n, MIN_SHADOW_SAMPLES
                ),
                muted: true,
            };
        }

        let wall = self
            .shadow_median_ms
            .map(format_step_ms)
            .unwrap_or_else(|| "—".to_string());
        let compute = self
            .train_step_median_ms
            .map(|ms| format!(" · compute {}", format_step_ms(ms)))
            .unwrap_or_default();
        let oh = self
            .dispatch_overhead_pct
            .map(format_pct_signed)
            .unwrap_or_else(|| "—".to_string());
        let level = self.dispatch_level();
        let overhead_line = if level.is_reassuring() {
            format!("{oh} extra · normal")
        } else {
            format!("{oh} extra · {}", level.label())
        };

        SidebarOverheadCopy {
            headline: format!("Step {} · {} / step", self.latest_step, wall),
            performance: format!("Shadow baseline{compute}"),
            overhead: overhead_line,
            muted: false,
        }
    }
}

fn is_stable_sample(shadow_n: i64, dispatch_n: i64, shadow_baseline: i64) -> bool {
    shadow_baseline > 0 && shadow_n >= MIN_SHADOW_SAMPLES && dispatch_n >= MIN_DISPATCH_SAMPLES
}

fn overhead_pct_raw(probed_ms: f64, shadow_ms: f64) -> Option<f64> {
    if shadow_ms <= 0.0 {
        None
    } else {
        Some((probed_ms / shadow_ms - 1.0) * 100.0)
    }
}

/// Expected per-step overhead when sampling is enabled: blend dispatch vs sampled path.
///
/// Uses configured `sample_rate` when present; otherwise empirical `sampled_n / probed_n`.
/// Avoids `mean(probed)/mean(shadow)` which diverges under step-time jitter and tiny shadow_n.
fn amortized_overhead_pct(
    dispatch_pct: Option<f64>,
    sampled_pct: Option<f64>,
    blended_pct: Option<f64>,
    sample_rate: Option<f64>,
    sampled_n: i64,
    probed_n: i64,
) -> Option<f64> {
    let dispatch = dispatch_pct.or(blended_pct)?;
    let rate = effective_sample_rate(sample_rate, sampled_n, probed_n);
    if sampled_n == 0 || rate <= 0.0 {
        return Some(dispatch);
    }
    let sampled = sampled_pct.unwrap_or(dispatch);
    Some((1.0 - rate) * dispatch + rate * sampled)
}

fn effective_sample_rate(sample_rate: Option<f64>, sampled_n: i64, probed_n: i64) -> f64 {
    sample_rate
        .filter(|r| r.is_finite() && (*r) >= 0.0 && (*r) <= 1.0)
        .or_else(|| {
            if probed_n > 0 && sampled_n > 0 {
                Some(sampled_n as f64 / probed_n as f64)
            } else {
                None
            }
        })
        .unwrap_or(0.0)
        .clamp(0.0, 1.0)
}

pub fn format_pct_signed(pct: f64) -> String {
    if pct.abs() < 0.5 {
        "≈0%".to_string()
    } else if pct.abs() < 5.0 {
        format!("~{:.0}%", pct)
    } else if pct > 0.0 {
        format!("+{pct:.1}%")
    } else {
        format!("{pct:.1}%")
    }
}

pub fn format_pct_display(pct: Option<f64>) -> String {
    pct.map(format_pct_signed)
        .unwrap_or_else(|| "—".to_string())
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
    use probing_proto::prelude::{DataFrame, Seq};

    fn sample_df() -> DataFrame {
        DataFrame {
            names: vec![
                "shadow_baseline".into(),
                "shadow_normal".into(),
                "latest_step".into(),
                "shadow_median_ms".into(),
                "dispatch_median_ms".into(),
                "probed_median_ms".into(),
                "shadow_n".into(),
                "dispatch_n".into(),
                "probed_n".into(),
                "sampled_n".into(),
            ],
            cols: vec![
                Seq::SeqI64(vec![1]),
                Seq::SeqI64(vec![4]),
                Seq::SeqI64(vec![120]),
                Seq::SeqF64(vec![800.0]),
                Seq::SeqF64(vec![816.0]),
                Seq::SeqF64(vec![820.0]),
                Seq::SeqI64(vec![16]),
                Seq::SeqI64(vec![64]),
                Seq::SeqI64(vec![80]),
                Seq::SeqI64(vec![4]),
            ],
            size: 1,
        }
    }

    #[test]
    fn snapshot_computes_dispatch_overhead() {
        let snap = OverheadSnapshot::from_summary(&sample_df());
        assert!(snap.is_stable());
        let pct = snap.dispatch_overhead_pct.unwrap();
        assert!((pct - 2.0).abs() < 0.2);
    }

    #[test]
    fn sidebar_copy_when_stable() {
        let copy = OverheadSnapshot::from_summary(&sample_df())
            .with_train_step_median(Some(780.0))
            .sidebar_copy();
        assert!(!copy.muted);
        assert!(copy.headline.contains("Step 120"));
        assert!(copy.headline.contains("800 ms"));
        assert!(copy.overhead.contains("normal"));
    }

    #[test]
    fn overhead_pct_positive() {
        let value = format_pct_signed((133.0 / 100.0 - 1.0) * 100.0);
        assert_eq!(value, "+33.0%");
    }

    #[test]
    fn overhead_pct_near_zero() {
        let value = format_pct_signed((100.2 / 100.0 - 1.0) * 100.0);
        assert_eq!(value, "≈0%");
    }

    #[test]
    fn amortized_blends_dispatch_and_sampled_by_rate() {
        let pct =
            amortized_overhead_pct(Some(1.9), Some(-4.9), Some(1.5), Some(0.05), 4, 50).unwrap();
        // 0.95 * 1.9 + 0.05 * (-4.9) ≈ 1.56
        assert!((pct - 1.56).abs() < 0.1);
    }

    #[test]
    fn amortized_without_sampling_matches_dispatch() {
        let pct = amortized_overhead_pct(Some(2.0), None, Some(2.0), Some(0.0), 0, 50).unwrap();
        assert!((pct - 2.0).abs() < 0.01);
    }

    /// Regression: medians ~2% but means imply 300%+ — amortized must stay weighted, not mean ratio.
    #[test]
    fn amortized_not_mean_ratio_when_means_diverge() {
        let df = user_mean_divergence_df();
        let snap = OverheadSnapshot::from_summary(&df);
        let dispatch = snap.dispatch_overhead_pct.expect("stable dispatch");
        let amortized = snap.amortized_overhead_pct.expect("amortized");
        assert!(dispatch < 5.0, "dispatch should be low: {dispatch}");
        assert!(
            (amortized - dispatch).abs() < 3.0,
            "amortized {amortized} should track dispatch {dispatch}, not mean ratio"
        );
        let mean_ratio = (533.0 / 130.0 - 1.0) * 100.0;
        assert!(
            (amortized - mean_ratio).abs() > 50.0,
            "amortized must not use mean(probed)/mean(shadow): got {amortized}, mean ratio {mean_ratio}"
        );
    }

    #[test]
    fn format_pct_signed_soft_display_under_five_percent() {
        assert_eq!(format_pct_signed(1.9), "~2%");
        assert_eq!(format_pct_signed(-4.9), "~-5%");
    }

    fn user_mean_divergence_df() -> DataFrame {
        DataFrame {
            names: vec![
                "shadow_baseline".into(),
                "shadow_normal".into(),
                "latest_step".into(),
                "shadow_median_ms".into(),
                "dispatch_median_ms".into(),
                "sampled_median_ms".into(),
                "probed_median_ms".into(),
                "shadow_mean_ms".into(),
                "probed_mean_ms".into(),
                "shadow_n".into(),
                "dispatch_n".into(),
                "sampled_n".into(),
                "probed_n".into(),
                "sample_rate".into(),
            ],
            cols: vec![
                Seq::SeqI64(vec![1]),
                Seq::SeqI64(vec![4]),
                Seq::SeqI64(vec![500]),
                Seq::SeqF64(vec![180.0]),
                Seq::SeqF64(vec![184.0]),
                Seq::SeqF64(vec![172.0]),
                Seq::SeqF64(vec![183.0]),
                Seq::SeqF64(vec![130.0]),
                Seq::SeqF64(vec![533.0]),
                Seq::SeqI64(vec![16]),
                Seq::SeqI64(vec![61]),
                Seq::SeqI64(vec![4]),
                Seq::SeqI64(vec![65]),
                Seq::SeqF64(vec![0.05]),
            ],
            size: 1,
        }
    }
}
