//! Deterministic evaluation of skill ``interpretation.rules``.

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
    expand_step_column_placeholders(&mut msg, steps);
    msg
}

/// Replace `{step_id.column}` and bare `{column}` placeholders from the first matching row.
pub fn expand_step_column_placeholders(msg: &mut String, steps: &[StepEvidence]) {
    for ev in steps {
        for col in &ev.dataframe.names {
            let qualified = format!("{}.{}", ev.step_id, col);
            if let Some(val) = cell_display_first(&ev.dataframe, col) {
                *msg = msg.replace(&format!("{{{qualified}}}"), &val);
                if msg.contains(&format!("{{{col}}}")) {
                    *msg = msg.replace(&format!("{{{col}}}"), &val);
                }
            }
        }
    }
}

pub fn cell_display_first(df: &DataFrame, col: &str) -> Option<String> {
    let idx = df.names.iter().position(|n| n == col)?;
    let col_data = &df.cols[idx];
    if col_data.is_empty() {
        return None;
    }
    Some(ele_str(&col_data.get(0)))
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
        if !eval_clause(part, step_ev, params) {
            return false;
        }
        i += 1;
    }
    true
}

fn eval_clause(
    clause: &str,
    step: Option<&StepEvidence>,
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
        let Some(ev) = step else { return false };
        return eval_top_vs_median(clause, ev);
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
    if let Some(rhs) = tail.strip_prefix("value <") {
        let threshold = rhs.trim().parse::<f64>().unwrap_or(0.0);
        return nums.first().copied().unwrap_or(0.0) < threshold;
    }
    if let Some(rhs) = tail.strip_prefix("value ==") {
        let threshold = rhs.trim().parse::<f64>().unwrap_or(f64::NAN);
        let value = nums.first().copied().unwrap_or(0.0);
        return (value - threshold).abs() < f64::EPSILON;
    }
    if let Some(rest) = tail.strip_prefix("ratio(") {
        if let Some((expr, pred)) = rest.split_once(')') {
            let threshold = pred
                .trim()
                .strip_prefix('>')
                .map(str::trim)
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0);
            if let Some((num_col, den_col)) = expr.split_once('/') {
                let num = column_f64(&ev.dataframe, num_col.trim())
                    .first()
                    .copied()
                    .unwrap_or(0.0);
                let den = column_f64(&ev.dataframe, den_col.trim())
                    .first()
                    .copied()
                    .unwrap_or(0.0);
                let ratio = if den <= 0.0 { 0.0 } else { num / den };
                return ratio > threshold;
            }
        }
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

fn eval_top_vs_median(clause: &str, ev: &StepEvidence) -> bool {
    // top(row).avg_ms > 2 * median(avg_ms)
    // top(row).worst_fraction > 2 * median(worst_fraction)
    if ev.row_count < 2 {
        return false;
    }
    let (col, factor) = parse_top_median_clause(clause);
    let vals = column_f64(&ev.dataframe, col);
    if vals.is_empty() {
        return false;
    }
    let top = vals.iter().copied().fold(f64::NAN, f64::max);
    let med = median(&vals);
    top > factor * med
}

fn parse_top_median_clause(clause: &str) -> (&str, f64) {
    let mut col = "avg_ms";
    let mut factor = 2.0;
    if let Some(top_part) = clause.split('|').find(|p| p.contains("top(row)")) {
        if let Some(rest) = top_part.strip_prefix("top(row).") {
            if let Some((col_name, rhs)) = rest.split_once(" > ") {
                col = col_name.trim();
                if let Some((f_str, med_part)) = rhs.split_once(" * median(") {
                    factor = f_str.trim().parse().unwrap_or(2.0);
                    let med_col = med_part.trim_end_matches(')').trim();
                    if !med_col.is_empty() {
                        col = med_col;
                    }
                }
            }
        }
    }
    (col, factor)
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
    fn value_eq_zero_rule() {
        let rules = vec![InterpretRule {
            id: "shadow_off".into(),
            when: "step:torch_probe_overhead | column:shadow_baseline | value == 0".into(),
            severity: "info".into(),
            message: "shadow off".into(),
        }];
        let steps = vec![StepEvidence {
            step_id: "torch_probe_overhead".into(),
            row_count: 1,
            dataframe: df_one_col("shadow_baseline", vec![0.0]),
        }];
        let findings = evaluate_rules(&rules, &steps, &HashMap::new());
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn ratio_rule() {
        let rules = vec![InterpretRule {
            id: "many_slow".into(),
            when: "step:job_slowdown_proxy | column:slow_steps | ratio(slow_steps/steps_in_window) > 0.3"
                .into(),
            severity: "warning".into(),
            message: "slow".into(),
        }];
        let steps = vec![StepEvidence {
            step_id: "job_slowdown_proxy".into(),
            row_count: 1,
            dataframe: DataFrame::new(
                vec!["slow_steps".into(), "steps_in_window".into()],
                vec![Seq::SeqF64(vec![4.0]), Seq::SeqF64(vec![10.0])],
            ),
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

    #[test]
    fn top_vs_median_worst_fraction() {
        let rules = vec![InterpretRule {
            id: "dominant".into(),
            when: "step:worst_fraction | rows >= 2 | top(row).worst_fraction > 2 * median(worst_fraction)"
                .into(),
            severity: "info".into(),
            message: "dominant".into(),
        }];
        let steps = vec![StepEvidence {
            step_id: "worst_fraction".into(),
            row_count: 3,
            dataframe: df_one_col("worst_fraction", vec![0.1, 0.15, 0.6]),
        }];
        let findings = evaluate_rules(&rules, &steps, &HashMap::new());
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn interpret_rules_match_fixture() {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/skill_interpret_parity.yaml");
        let raw = std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", fixture_path.display()));
        let doc: serde_yaml::Value = serde_yaml::from_str(&raw).expect("parse fixture yaml");
        let cases = doc
            .get("cases")
            .and_then(|v| v.as_sequence())
            .expect("cases array");

        for case in cases {
            let name = case
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unnamed");
            let rules_val = case
                .get("rules")
                .and_then(|v| v.as_sequence())
                .expect("rules");
            let mut rules = Vec::new();
            for r in rules_val {
                rules.push(InterpretRule {
                    id: r.get("id").and_then(|v| v.as_str()).unwrap().into(),
                    when: r.get("when").and_then(|v| v.as_str()).unwrap().into(),
                    severity: r.get("severity").and_then(|v| v.as_str()).unwrap().into(),
                    message: r.get("message").and_then(|v| v.as_str()).unwrap().into(),
                });
            }

            let steps_val = case
                .get("steps")
                .and_then(|v| v.as_sequence())
                .expect("steps");
            let mut steps = Vec::new();
            for s in steps_val {
                let step_id = s
                    .get("step_id")
                    .and_then(|v| v.as_str())
                    .unwrap()
                    .to_string();
                let row_count = s.get("row_count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let mut df = DataFrame::default();
                if let Some(cols) = s.get("columns").and_then(|v| v.as_mapping()) {
                    let mut names = Vec::new();
                    let mut seqs = Vec::new();
                    for (col_name, col_vals) in cols {
                        let name = col_name.as_str().unwrap().to_string();
                        let vals: Vec<f64> = col_vals
                            .as_sequence()
                            .unwrap()
                            .iter()
                            .map(|v| v.as_f64().unwrap())
                            .collect();
                        names.push(name);
                        seqs.push(Seq::SeqF64(vals));
                    }
                    df = DataFrame::new(names, seqs);
                }
                steps.push(StepEvidence {
                    step_id,
                    row_count,
                    dataframe: df,
                });
            }

            let mut params = HashMap::new();
            if let Some(p) = case.get("params").and_then(|v| v.as_mapping()) {
                for (k, v) in p {
                    params.insert(
                        k.as_str().unwrap().to_string(),
                        v.as_str().unwrap().to_string(),
                    );
                }
            }

            let expect_count = case
                .get("expect_count")
                .and_then(|v| v.as_u64())
                .expect("expect_count") as usize;
            let findings = evaluate_rules(&rules, &steps, &params);
            assert_eq!(findings.len(), expect_count, "case {name}");
        }
    }
}
