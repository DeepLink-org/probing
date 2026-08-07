use super::ApiClient;
use crate::utils::error::{AppError, Result};
use probing_proto::prelude::{DataFrame, Ele};

/// Latest process-level CPU snapshot from `cpu.utilization`.
#[derive(Clone, Debug, Default, PartialEq)]
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
        let df = self
            .execute_query(
                "SELECT ts, platform, wall_ns, delta_user_ns, delta_sys_ns, delta_total_ns, \
                 cpu_user_pct, cpu_sys_pct, cpu_total_pct, rss_kb, thread_count, \
                 delta_vol_ctxt, delta_invol_ctxt \
                 FROM cpu.utilization WHERE scope = 'process' ORDER BY ts DESC LIMIT 1",
            )
            .await?;
        parse_cpu_snapshot(&df)
    }

    pub async fn fetch_cpu_history(&self, limit: usize) -> Result<Vec<CpuHistorySample>> {
        let df = self
            .execute_query(&format!(
                "SELECT delta_user_ns, delta_sys_ns, delta_total_ns \
                 FROM cpu.utilization WHERE scope = 'process' ORDER BY ts DESC LIMIT {limit}"
            ))
            .await?;
        parse_cpu_history(&df)
    }

    pub async fn fetch_cpu_top_threads(&self, limit: usize) -> Result<Vec<CpuThreadRow>> {
        let fetch_limit = limit.saturating_mul(4).max(limit);
        let df = self
            .execute_query(&format!(
                "SELECT ts, tid, comm, state, wchan, delta_user_ns, delta_sys_ns, delta_total_ns \
                 FROM cpu.tasks ORDER BY ts DESC, delta_total_ns DESC LIMIT {fetch_limit}"
            ))
            .await?;
        parse_cpu_top_threads(&df, limit)
    }
}

fn invalid_cpu_data(message: impl Into<String>) -> AppError {
    AppError::Api(format!("Invalid CPU metrics response: {}", message.into()))
}

fn required_columns(df: &DataFrame, names: &[&str]) -> Result<(usize, Vec<usize>)> {
    let rows = df.row_count();
    let indexes = names
        .iter()
        .map(|name| {
            let index = df
                .names
                .iter()
                .position(|n| n == name)
                .ok_or_else(|| invalid_cpu_data(format!("missing required column `{name}`")))?;
            let column = df
                .cols
                .get(index)
                .ok_or_else(|| invalid_cpu_data(format!("missing data for column `{name}`")))?;
            if column.len() != rows {
                return Err(invalid_cpu_data(format!(
                    "column `{name}` has {} rows, expected {rows}",
                    column.len()
                )));
            }
            Ok(index)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((rows, indexes))
}

fn cell(df: &DataFrame, row: usize, col: usize, name: &str) -> Result<Ele> {
    let value = df
        .cols
        .get(col)
        .ok_or_else(|| invalid_cpu_data(format!("missing data for column `{name}`")))?
        .get(row);
    if value == Ele::Nil {
        return Err(invalid_cpu_data(format!(
            "null or missing value in `{name}` at row {row}"
        )));
    }
    Ok(value)
}

fn ele_f32(e: Ele, name: &str) -> Result<f32> {
    match e {
        Ele::F32(v) => Ok(v),
        Ele::F64(v) => Ok(v as f32),
        Ele::I32(v) => Ok(v as f32),
        Ele::I64(v) => Ok(v as f32),
        other => Err(invalid_cpu_data(format!(
            "column `{name}` expected numeric data, got {other:?}"
        ))),
    }
}

fn ele_i64(e: Ele, name: &str) -> Result<i64> {
    match e {
        Ele::I64(v) => Ok(v),
        Ele::I32(v) => Ok(v as i64),
        other => Err(invalid_cpu_data(format!(
            "column `{name}` expected integer data, got {other:?}"
        ))),
    }
}

fn ele_i32(e: Ele, name: &str) -> Result<i32> {
    match e {
        Ele::I32(v) => Ok(v),
        Ele::I64(v) => i32::try_from(v).map_err(|_| {
            invalid_cpu_data(format!("column `{name}` value {v} is outside i32 range"))
        }),
        other => Err(invalid_cpu_data(format!(
            "column `{name}` expected integer data, got {other:?}"
        ))),
    }
}

fn ele_text(e: Ele, name: &str) -> Result<String> {
    match e {
        Ele::Text(s) => Ok(s),
        other => Err(invalid_cpu_data(format!(
            "column `{name}` expected text data, got {other:?}"
        ))),
    }
}

fn ns_to_ms(ns: i64) -> f32 {
    ns as f32 / 1_000_000.0
}

fn parse_cpu_snapshot(df: &DataFrame) -> Result<Option<CpuSnapshot>> {
    const COLUMNS: &[&str] = &[
        "platform",
        "delta_user_ns",
        "delta_sys_ns",
        "delta_total_ns",
        "cpu_user_pct",
        "cpu_sys_pct",
        "cpu_total_pct",
        "rss_kb",
        "thread_count",
        "delta_vol_ctxt",
        "delta_invol_ctxt",
    ];
    let (rows, indexes) = required_columns(df, COLUMNS)?;
    if rows == 0 {
        return Ok(None);
    }
    let get = |position: usize| cell(df, 0, indexes[position], COLUMNS[position]);
    Ok(Some(CpuSnapshot {
        platform: ele_text(get(0)?, COLUMNS[0])?,
        delta_user_ns: ele_i64(get(1)?, COLUMNS[1])?,
        delta_sys_ns: ele_i64(get(2)?, COLUMNS[2])?,
        delta_total_ns: ele_i64(get(3)?, COLUMNS[3])?,
        cpu_user_pct: ele_f32(get(4)?, COLUMNS[4])?,
        cpu_sys_pct: ele_f32(get(5)?, COLUMNS[5])?,
        cpu_total_pct: ele_f32(get(6)?, COLUMNS[6])?,
        rss_kb: ele_i64(get(7)?, COLUMNS[7])?,
        thread_count: ele_i32(get(8)?, COLUMNS[8])?,
        delta_vol_ctxt: ele_i64(get(9)?, COLUMNS[9])?,
        delta_invol_ctxt: ele_i64(get(10)?, COLUMNS[10])?,
    }))
}

fn parse_cpu_history(df: &DataFrame) -> Result<Vec<CpuHistorySample>> {
    const COLUMNS: &[&str] = &["delta_user_ns", "delta_sys_ns", "delta_total_ns"];
    let (rows, indexes) = required_columns(df, COLUMNS)?;
    let mut out = Vec::with_capacity(rows);
    for row in 0..rows {
        out.push(CpuHistorySample {
            user_ms: ns_to_ms(ele_i64(cell(df, row, indexes[0], COLUMNS[0])?, COLUMNS[0])?),
            sys_ms: ns_to_ms(ele_i64(cell(df, row, indexes[1], COLUMNS[1])?, COLUMNS[1])?),
            total_ms: ns_to_ms(ele_i64(cell(df, row, indexes[2], COLUMNS[2])?, COLUMNS[2])?),
        });
    }
    out.reverse();
    Ok(out)
}

fn parse_cpu_top_threads(df: &DataFrame, limit: usize) -> Result<Vec<CpuThreadRow>> {
    const COLUMNS: &[&str] = &[
        "ts",
        "tid",
        "comm",
        "state",
        "wchan",
        "delta_user_ns",
        "delta_sys_ns",
        "delta_total_ns",
    ];
    let (rows, indexes) = required_columns(df, COLUMNS)?;
    if rows == 0 {
        return Ok(vec![]);
    }

    let mut timestamps = Vec::with_capacity(rows);
    for row in 0..rows {
        timestamps.push(ele_i64(cell(df, row, indexes[0], COLUMNS[0])?, COLUMNS[0])?);
    }
    let latest_ts = timestamps
        .iter()
        .copied()
        .max()
        .ok_or_else(|| invalid_cpu_data("missing thread timestamps"))?;

    let mut out = Vec::new();
    for (row, ts) in timestamps.into_iter().enumerate() {
        if ts != latest_ts {
            continue;
        }
        let tid = ele_i32(cell(df, row, indexes[1], COLUMNS[1])?, COLUMNS[1])?;
        let comm = ele_text(cell(df, row, indexes[2], COLUMNS[2])?, COLUMNS[2])?;
        let state = ele_text(cell(df, row, indexes[3], COLUMNS[3])?, COLUMNS[3])?;
        let wchan = ele_text(cell(df, row, indexes[4], COLUMNS[4])?, COLUMNS[4])?
            .trim()
            .to_string();
        out.push(CpuThreadRow {
            tid,
            name: thread_display_name(&comm, tid),
            state,
            wchan: (!wchan.is_empty()).then_some(wchan),
            delta_user_ns: ele_i64(cell(df, row, indexes[5], COLUMNS[5])?, COLUMNS[5])?,
            delta_sys_ns: ele_i64(cell(df, row, indexes[6], COLUMNS[6])?, COLUMNS[6])?,
            delta_total_ns: ele_i64(cell(df, row, indexes[7], COLUMNS[7])?, COLUMNS[7])?,
        });
    }

    out.sort_by_key(|b| std::cmp::Reverse(b.delta_total_ns));
    out.truncate(limit);
    Ok(out)
}

pub fn format_cpu_ms(ns: i64) -> String {
    format!("{:.1} ms", ns as f64 / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use probing_proto::prelude::Seq;

    #[test]
    fn cpu_history_rejects_missing_required_column() {
        let df = DataFrame::new(
            vec!["delta_user_ns".into(), "delta_sys_ns".into()],
            vec![Seq::SeqI64(vec![1]), Seq::SeqI64(vec![2])],
        );
        let err = parse_cpu_history(&df).unwrap_err();
        assert!(err.to_string().contains("delta_total_ns"));
    }

    #[test]
    fn cpu_snapshot_rejects_wrong_value_type() {
        let names = vec![
            "platform",
            "delta_user_ns",
            "delta_sys_ns",
            "delta_total_ns",
            "cpu_user_pct",
            "cpu_sys_pct",
            "cpu_total_pct",
            "rss_kb",
            "thread_count",
            "delta_vol_ctxt",
            "delta_invol_ctxt",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let df = DataFrame::new(
            names,
            vec![
                Seq::SeqText(vec!["linux".into()]),
                Seq::SeqText(vec!["not-a-number".into()]),
                Seq::SeqI64(vec![0]),
                Seq::SeqI64(vec![0]),
                Seq::SeqF32(vec![0.0]),
                Seq::SeqF32(vec![0.0]),
                Seq::SeqF32(vec![0.0]),
                Seq::SeqI64(vec![0]),
                Seq::SeqI32(vec![1]),
                Seq::SeqI64(vec![0]),
                Seq::SeqI64(vec![0]),
            ],
        );
        let err = parse_cpu_snapshot(&df).unwrap_err();
        assert!(err.to_string().contains("delta_user_ns"));
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
