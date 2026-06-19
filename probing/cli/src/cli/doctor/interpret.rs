//! Deterministic evaluation of playbook ``interpretation.rules``.

use std::collections::HashMap;

use probing_proto::prelude::{DataFrame, Ele};

use super::loader::InterpretRule;

#[derive(Debug, Clone)]
pub struct StepEvidence {
    pub step_id: String,
    pub row_count: usize,
    pub dataframe: DataFrame,
}

#[derive(Debug, Clone)]
pub struct InterpretFinding {
    pub rule_id: String,
    pub severity: String,
    pub message: String,
}

pub fn evaluate_rules(
    rules: &[InterpretRule],
    steps: &[StepEvidence],
    params: &HashMap<String, String>,
) -> Vec<InterpretFinding> {
    let mut out = Vec::new();
    for rule in rules {
        let when = expand_params(&rule.when, params);
        if rule_matches(&when, steps, params) {
            out.push(InterpretFinding {
                rule_id: rule.id.clone(),
                severity: rule.severity.clone(),
                message: expand_message(&rule.message, steps, params),
            });
        }
    }
    out
}

fn expand_params(template: &str, params: &HashMap<String, String>) -> String {
    let mut out = template.to_string();
    for (k, v) in params {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

fn expand_message(
    template: &str,
    steps: &[StepEvidence],
    params: &HashMap<String, String>,
) -> String {
    let mut msg = expand_params(template, params);
    if let Some(rank) = worst_rank_by_avg_ms(steps) {
        msg = msg.replace("{worst_rank}", &rank);
    }
    if let Some(module) = top_module_by_pct(steps) {
        msg = msg.replace("{top_module}", &module);
    }
    if let Some(step) = latest_step_value(steps) {
        msg = msg.replace("{latest_step}", &step);
    }
    msg
}

fn step_by_id<'a>(steps: &'a [StepEvidence], id: &str) -> Option<&'a StepEvidence> {
    steps.iter().find(|s| s.step_id == id)
}

fn rule_matches(when: &str, steps: &[StepEvidence], params: &HashMap<String, String>) -> bool {
    let parts: Vec<&str> = when
        .split('|')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return false;
    }

    let mut idx = 0;
    let mut step_ev: Option<&StepEvidence> = None;
    if let Some(rest) = parts[0].strip_prefix("step:") {
        step_ev = step_by_id(steps, rest);
        if step_ev.is_none() {
            return false;
        }
        idx = 1;
    }

    let mut i = idx;
    while i < parts.len() {
        let part = parts[i];
        if let Some(col_name) = part.strip_prefix("column:") {
            let Some(ev) = step_ev else { return false };
            let tail = parts.get(i + 1).copied().unwrap_or("");
            if !eval_column_predicate(col_name.trim(), tail, ev) {
                return false;
            }
            i += 2;
            continue;
        }
        if !eval_clause(part, step_ev, steps, params) {
            return false;
        }
        i += 1;
    }
    true
}

fn eval_clause(
    clause: &str,
    step: Option<&StepEvidence>,
    steps: &[StepEvidence],
    params: &HashMap<String, String>,
) -> bool {
    if clause == "always" {
        return true;
    }
    if let Some(rest) = clause.strip_prefix("rows ") {
        let Some(ev) = step else { return false };
        return eval_rows_predicate(rest, ev.row_count, params);
    }
    if clause.contains("top(row)") {
        return eval_top_vs_median(clause, steps);
    }
    false
}

fn eval_rows_predicate(pred: &str, row_count: usize, params: &HashMap<String, String>) -> bool {
    if let Some((op, rhs)) = pred.split_once(' ') {
        let rhs = rhs.trim();
        let threshold = eval_numeric_expr(rhs, params);
        return match op {
            "==" => row_count == threshold as usize,
            ">=" => row_count >= threshold as usize,
            ">" => row_count > threshold as usize,
            "<=" => row_count <= threshold as usize,
            "<" => row_count < threshold as usize,
            _ => false,
        };
    }
    false
}

fn eval_numeric_expr(expr: &str, params: &HashMap<String, String>) -> f64 {
    let expr = expand_params(expr, params);
    if let Some((lhs, rhs)) = expr.split_once('*') {
        return lhs.trim().parse::<f64>().unwrap_or(0.0) * rhs.trim().parse::<f64>().unwrap_or(0.0);
    }
    expr.parse::<f64>().unwrap_or(0.0)
}

fn eval_column_predicate(col_name: &str, tail: &str, ev: &StepEvidence) -> bool {
    let nums = column_f64(&ev.dataframe, col_name);
    let texts = column_str(&ev.dataframe, col_name);

    if tail.contains("max/min(ratio)") {
        if let Some((_, rhs)) = tail.split_once('>') {
            let threshold = rhs.trim().parse::<f64>().unwrap_or(0.0);
            return max_min_ratio(&nums) > threshold;
        }
    }
    if let Some(rhs) = tail.strip_prefix("max >") {
        let threshold = rhs.trim().parse::<f64>().unwrap_or(0.0);
        return nums.iter().copied().fold(f64::NAN, f64::max).max(0.0) > threshold;
    }
    if let Some(rhs) = tail.strip_prefix("avg >") {
        let threshold = rhs.trim().parse::<f64>().unwrap_or(0.0);
        return avg(&nums) > threshold;
    }
    if let Some(rhs) = tail.strip_prefix("top >") {
        let threshold = rhs.trim().parse::<f64>().unwrap_or(0.0);
        return nums.iter().copied().fold(f64::NAN, f64::max).max(0.0) > threshold;
    }
    if let Some(rhs) = tail.strip_prefix("value >") {
        let threshold = rhs.trim().parse::<f64>().unwrap_or(0.0);
        return nums.first().copied().unwrap_or(0.0) > threshold;
    }
    if tail.starts_with("last >") {
        if let Some(rhs) = tail.strip_prefix("last >") {
            let rhs = rhs.trim();
            if let Some((mul, col)) = rhs.split_once("* avg(") {
                let factor = mul.trim().parse::<f64>().unwrap_or(2.0);
                let col = col.trim_end_matches(')');
                let col_vals = column_f64(&ev.dataframe, col);
                let last = col_vals.last().copied().unwrap_or(0.0);
                return last > factor * avg(&col_vals);
            }
        }
    }
    if tail.starts_with("any_contains(") {
        let inner = tail
            .trim_start_matches("any_contains(")
            .trim_end_matches(')');
        let needles: Vec<String> = inner
            .split(',')
            .map(|s| s.trim().trim_matches('\'').trim_matches('"').to_lowercase())
            .collect();
        return texts.iter().any(|t| {
            let tl = t.to_lowercase();
            needles.iter().any(|n| tl.contains(n))
        });
    }
    false
}

fn eval_top_vs_median(clause: &str, steps: &[StepEvidence]) -> bool {
    // step:rank_latency | rows >= 2 | top(row).avg_ms > 2 * median(avg_ms)
    let parts: Vec<&str> = clause.split('|').map(|p| p.trim()).collect();
    let step_id = parts
        .first()
        .and_then(|p| p.strip_prefix("step:"))
        .unwrap_or("rank_latency");
    let Some(ev) = step_by_id(steps, step_id) else {
        return false;
    };
    if ev.row_count < 2 {
        return false;
    }
    let vals = column_f64(&ev.dataframe, "avg_ms");
    if vals.is_empty() {
        return false;
    }
    let top = vals.iter().copied().fold(f64::NAN, f64::max);
    let med = median(&vals);
    top > 2.0 * med
}

fn column_f64(df: &DataFrame, name: &str) -> Vec<f64> {
    let idx = match df.names.iter().position(|n| n == name) {
        Some(i) => i,
        None => return Vec::new(),
    };
    let col = &df.cols[idx];
    (0..col.len())
        .filter_map(|i| ele_f64(&col.get(i)))
        .collect()
}

fn column_str(df: &DataFrame, name: &str) -> Vec<String> {
    let idx = match df.names.iter().position(|n| n == name) {
        Some(i) => i,
        None => return Vec::new(),
    };
    let col = &df.cols[idx];
    (0..col.len()).map(|i| ele_str(&col.get(i))).collect()
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

fn ele_str(ele: &Ele) -> String {
    match ele {
        Ele::Text(s) => s.clone(),
        Ele::Nil => String::new(),
        Ele::BOOL(b) => b.to_string(),
        Ele::I32(x) => x.to_string(),
        Ele::I64(x) => x.to_string(),
        Ele::F32(x) => x.to_string(),
        Ele::F64(x) => x.to_string(),
        Ele::Url(u) => u.clone(),
        Ele::DataTime(t) => t.to_string(),
    }
}

fn avg(vals: &[f64]) -> f64 {
    if vals.is_empty() {
        0.0
    } else {
        vals.iter().sum::<f64>() / vals.len() as f64
    }
}

fn median(vals: &[f64]) -> f64 {
    if vals.is_empty() {
        return 0.0;
    }
    let mut sorted = vals.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

fn max_min_ratio(vals: &[f64]) -> f64 {
    if vals.len() < 2 {
        return 0.0;
    }
    let max = vals.iter().copied().fold(f64::NAN, f64::max);
    let min = vals.iter().copied().fold(f64::NAN, f64::min);
    if min <= 0.0 {
        f64::INFINITY
    } else {
        max / min
    }
}

fn worst_rank_by_avg_ms(steps: &[StepEvidence]) -> Option<String> {
    let ev = step_by_id(steps, "rank_latency")?;
    let ranks = column_str(&ev.dataframe, "rank");
    let avgs = column_f64(&ev.dataframe, "avg_ms");
    ranks
        .into_iter()
        .zip(avgs)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(r, _)| r)
}

fn top_module_by_pct(steps: &[StepEvidence]) -> Option<String> {
    let ev = step_by_id(steps, "module_totals")?;
    let modules = column_str(&ev.dataframe, "module");
    let pcts = column_f64(&ev.dataframe, "pct_time");
    modules
        .into_iter()
        .zip(pcts)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(m, _)| m)
}

fn latest_step_value(steps: &[StepEvidence]) -> Option<String> {
    let ev = step_by_id(steps, "latest_torch_step")?;
    column_f64(&ev.dataframe, "latest_step")
        .first()
        .map(|v| v.to_string())
        .or_else(|| column_str(&ev.dataframe, "latest_step").first().cloned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use probing_proto::prelude::{DataFrame, Seq};

    fn df_one_col(name: &str, vals: Vec<f64>) -> DataFrame {
        DataFrame::new(vec![name.to_string()], vec![Seq::SeqF64(vals)])
    }

    #[test]
    fn rows_zero_rule() {
        let rules = vec![InterpretRule {
            id: "no_tables".into(),
            when: "step:available_tables | rows == 0".into(),
            severity: "error".into(),
            message: "no tables".into(),
        }];
        let steps = vec![StepEvidence {
            step_id: "available_tables".into(),
            row_count: 0,
            dataframe: DataFrame::default(),
        }];
        let findings = evaluate_rules(&rules, &steps, &HashMap::new());
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn max_min_ratio_rule() {
        let rules = vec![InterpretRule {
            id: "straggler".into(),
            when: "step:rank_latency | column:avg_ms | max/min(ratio) > 1.5".into(),
            severity: "warning".into(),
            message: "slow".into(),
        }];
        let steps = vec![StepEvidence {
            step_id: "rank_latency".into(),
            row_count: 3,
            dataframe: df_one_col("avg_ms", vec![10.0, 20.0, 40.0]),
        }];
        let findings = evaluate_rules(&rules, &steps, &HashMap::new());
        assert_eq!(findings.len(), 1);
    }
}
