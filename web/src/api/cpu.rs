use super::ApiClient;
use crate::utils::error::{AppError, Result};
use probing_proto::prelude::{DataFrame, Ele};

/// Latest process-level CPU snapshot from `cpu.utilization`.
#[derive(Clone, Debug, Default)]
pub struct CpuSnapshot {
    pub platform: String,
    pub delta_user_ns: i64,
    pub delta_sys_ns: i64,
    pub delta_total_ns: i64,
    pub cpu_user_pct: f32,
    pub cpu_sys_pct: f32,
    pub cpu_total_pct: f32,
    pub rss_kb: i64,
    pub thread_count: i32,
    pub delta_vol_ctxt: i64,
    pub delta_invol_ctxt: i64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CpuHistorySample {
    pub user_ms: f32,
    pub sys_ms: f32,
    pub total_ms: f32,
}

/// One row in the latest CPU thread ranking.
#[derive(Clone, Debug, PartialEq)]
pub struct CpuThreadRow {
    pub tid: i32,
    pub name: String,
    pub state: String,
    pub wchan: Option<String>,
    pub delta_user_ns: i64,
    pub delta_sys_ns: i64,
    pub delta_total_ns: i64,
}

fn is_cpu_table_missing(err: &AppError) -> bool {
    matches!(err, AppError::Api(msg)
        if msg.contains("cpu.") && msg.contains("not found"))
}

pub fn thread_display_name(comm: &str, tid: i32) -> String {
    let trimmed = comm.trim();
    if !trimmed.is_empty() {
        trimmed.to_string()
    } else {
        format!("thread-{tid}")
    }
}

impl ApiClient {
    pub async fn fetch_cpu_latest(&self) -> Result<Option<CpuSnapshot>> {
        match self
            .execute_query(
                "SELECT ts, platform, wall_ns, delta_user_ns, delta_sys_ns, delta_total_ns, \
                 cpu_user_pct, cpu_sys_pct, cpu_total_pct, rss_kb, thread_count, \
                 delta_vol_ctxt, delta_invol_ctxt \
                 FROM cpu.utilization WHERE scope = 'process' ORDER BY ts DESC LIMIT 1",
            )
            .await
        {
            Ok(df) => Ok(parse_cpu_snapshot(&df)),
            Err(e) if is_cpu_table_missing(&e) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn fetch_cpu_history(&self, limit: usize) -> Result<Vec<CpuHistorySample>> {
        match self
            .execute_query(&format!(
                "SELECT delta_user_ns, delta_sys_ns, delta_total_ns \
                 FROM cpu.utilization WHERE scope = 'process' ORDER BY ts DESC LIMIT {limit}"
            ))
            .await
        {
            Ok(df) => Ok(parse_cpu_history(&df)),
            Err(e) if is_cpu_table_missing(&e) => Ok(vec![]),
            Err(e) => Err(e),
        }
    }

    pub async fn fetch_cpu_top_threads(&self, limit: usize) -> Result<Vec<CpuThreadRow>> {
        let fetch_limit = limit.saturating_mul(4).max(limit);
        match self
            .execute_query(&format!(
                "SELECT ts, tid, comm, state, wchan, delta_user_ns, delta_sys_ns, delta_total_ns \
                 FROM cpu.tasks ORDER BY ts DESC, delta_total_ns DESC LIMIT {fetch_limit}"
            ))
            .await
        {
            Ok(df) => Ok(parse_cpu_top_threads(&df, limit)),
            Err(e) if is_cpu_table_missing(&e) => Ok(vec![]),
            Err(e) => Err(e),
        }
    }
}

fn col_index(df: &DataFrame, name: &str) -> Option<usize> {
    df.names.iter().position(|n| n == name)
}

fn cell(df: &DataFrame, row: usize, col: usize) -> Option<Ele> {
    df.cols.get(col).map(|c| c.get(row))
}

fn ele_f32(e: Ele) -> f32 {
    match e {
        Ele::F32(v) => v,
        Ele::F64(v) => v as f32,
        Ele::I32(v) => v as f32,
        Ele::I64(v) => v as f32,
        _ => 0.0,
    }
}

fn ele_i64(e: Ele) -> i64 {
    match e {
        Ele::I64(v) => v,
        Ele::I32(v) => v as i64,
        _ => 0,
    }
}

fn ele_i32(e: Ele) -> i32 {
    match e {
        Ele::I32(v) => v,
        Ele::I64(v) => v as i32,
        _ => 0,
    }
}

fn ele_text(e: Ele) -> String {
    match e {
        Ele::Text(s) => s,
        _ => String::new(),
    }
}

fn ns_to_ms(ns: i64) -> f32 {
    ns as f32 / 1_000_000.0
}

fn parse_cpu_snapshot(df: &DataFrame) -> Option<CpuSnapshot> {
    if df.cols.first().map(|c| c.len()).unwrap_or(0) == 0 {
        return None;
    }
    let idx = |name| col_index(df, name);
    Some(CpuSnapshot {
        platform: idx("platform")
            .and_then(|c| cell(df, 0, c).map(ele_text))
            .unwrap_or_default(),
        delta_user_ns: idx("delta_user_ns")
            .and_then(|c| cell(df, 0, c).map(ele_i64))
            .unwrap_or(0),
        delta_sys_ns: idx("delta_sys_ns")
            .and_then(|c| cell(df, 0, c).map(ele_i64))
            .unwrap_or(0),
        delta_total_ns: idx("delta_total_ns")
            .and_then(|c| cell(df, 0, c).map(ele_i64))
            .unwrap_or(0),
        cpu_user_pct: idx("cpu_user_pct")
            .and_then(|c| cell(df, 0, c).map(ele_f32))
            .unwrap_or(0.0),
        cpu_sys_pct: idx("cpu_sys_pct")
            .and_then(|c| cell(df, 0, c).map(ele_f32))
            .unwrap_or(0.0),
        cpu_total_pct: idx("cpu_total_pct")
            .and_then(|c| cell(df, 0, c).map(ele_f32))
            .unwrap_or(0.0),
        rss_kb: idx("rss_kb")
            .and_then(|c| cell(df, 0, c).map(ele_i64))
            .unwrap_or(0),
        thread_count: idx("thread_count")
            .and_then(|c| cell(df, 0, c).map(ele_i32))
            .unwrap_or(0),
        delta_vol_ctxt: idx("delta_vol_ctxt")
            .and_then(|c| cell(df, 0, c).map(ele_i64))
            .unwrap_or(0),
        delta_invol_ctxt: idx("delta_invol_ctxt")
            .and_then(|c| cell(df, 0, c).map(ele_i64))
            .unwrap_or(0),
    })
}

fn parse_cpu_history(df: &DataFrame) -> Vec<CpuHistorySample> {
    let rows = df.cols.first().map(|c| c.len()).unwrap_or(0);
    let idx = |name| col_index(df, name);
    let mut out: Vec<CpuHistorySample> = (0..rows)
        .map(|r| CpuHistorySample {
            user_ms: idx("delta_user_ns")
                .and_then(|c| cell(df, r, c).map(ele_i64))
                .map(ns_to_ms)
                .unwrap_or(0.0),
            sys_ms: idx("delta_sys_ns")
                .and_then(|c| cell(df, r, c).map(ele_i64))
                .map(ns_to_ms)
                .unwrap_or(0.0),
            total_ms: idx("delta_total_ns")
                .and_then(|c| cell(df, r, c).map(ele_i64))
                .map(ns_to_ms)
                .unwrap_or(0.0),
        })
        .collect();
    out.reverse();
    out
}

fn parse_cpu_top_threads(df: &DataFrame, limit: usize) -> Vec<CpuThreadRow> {
    let rows = df.cols.first().map(|c| c.len()).unwrap_or(0);
    if rows == 0 {
        return vec![];
    }
    let idx = |name: &str| col_index(df, name);

    let latest_ts = (0..rows)
        .filter_map(|r| idx("ts").and_then(|c| cell(df, r, c).map(ele_i64)))
        .max()
        .unwrap_or(0);

    let mut out: Vec<CpuThreadRow> = (0..rows)
        .filter_map(|r| {
            let ts = idx("ts").and_then(|c| cell(df, r, c).map(ele_i64))?;
            if ts != latest_ts {
                return None;
            }
            let tid = idx("tid").and_then(|c| cell(df, r, c).map(ele_i32))?;
            let comm = idx("comm")
                .and_then(|c| cell(df, r, c).map(ele_text))
                .unwrap_or_default();
            let state = idx("state")
                .and_then(|c| cell(df, r, c).map(ele_text))
                .unwrap_or_default();
            let wchan = idx("wchan")
                .and_then(|c| cell(df, r, c).map(ele_text))
                .filter(|s| !s.trim().is_empty());
            Some(CpuThreadRow {
                tid,
                name: thread_display_name(&comm, tid),
                state,
                wchan,
                delta_user_ns: idx("delta_user_ns")
                    .and_then(|c| cell(df, r, c).map(ele_i64))
                    .unwrap_or(0),
                delta_sys_ns: idx("delta_sys_ns")
                    .and_then(|c| cell(df, r, c).map(ele_i64))
                    .unwrap_or(0),
                delta_total_ns: idx("delta_total_ns")
                    .and_then(|c| cell(df, r, c).map(ele_i64))
                    .unwrap_or(0),
            })
        })
        .collect();

    out.sort_by_key(|b| std::cmp::Reverse(b.delta_total_ns));
    out.truncate(limit);
    out
}

pub fn format_rss(kb: i64) -> String {
    if kb >= 1024 * 1024 {
        format!("{:.1} GB", kb as f64 / (1024.0 * 1024.0))
    } else if kb >= 1024 {
        format!("{:.1} MB", kb as f64 / 1024.0)
    } else {
        format!("{kb} KB")
    }
}

pub fn format_pct(v: f32) -> String {
    format!("{v:.1}%")
}

pub fn format_cpu_ms(ns: i64) -> String {
    format!("{:.1} ms", ns as f64 / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_missing_cpu_table_error() {
        let err =
            AppError::Api("Error during planning: table 'probe.cpu.utilization' not found".into());
        assert!(is_cpu_table_missing(&err));
    }

    #[test]
    fn thread_display_name_prefers_comm() {
        assert_eq!(
            thread_display_name("tokio-runtime-worker", 42),
            "tokio-runtime-worker"
        );
        assert_eq!(thread_display_name("  ", 7), "thread-7");
    }
}
