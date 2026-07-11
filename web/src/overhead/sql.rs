//! SQL for TorchProbe in-run overhead (`python.torch_step_timing`).

/// Rolling window size for overhead aggregates (larger → stabler medians).
pub const WINDOW_STEPS: i64 = 80;

/// Recent step bars in the timeline panel.
pub const RECENT_STEPS: i64 = 24;

/// Legacy mmap rows may have registered numeric columns as Utf8 — cast at query time.
const STEP_SEC: &str = "CAST(step_duration_sec AS DOUBLE)";
const IS_SHADOW: &str = "CAST(is_shadow AS BIGINT)";
const IS_SAMPLED: &str = "CAST(sampled AS BIGINT)";
const SHADOW_NORMAL: &str = "CAST(shadow_normal AS BIGINT)";
const SHADOW_BASELINE: &str = "CAST(shadow_baseline AS BIGINT)";
const SAMPLE_RATE: &str = "CAST(sample_rate AS DOUBLE)";

const TIMING_TABLE: &str = "python.torch_step_timing";

fn window_start(table: &str, window_steps: i64) -> String {
    format!("GREATEST(COALESCE((SELECT max(local_step) FROM {table}), 0) - {window_steps}, 1)")
}

pub fn summary() -> String {
    let win = window_start(TIMING_TABLE, WINDOW_STEPS);
    format!(
        "SELECT \
         round((SELECT median({STEP_SEC}) FROM {TIMING_TABLE} \
           WHERE {IS_SHADOW} = 0 AND local_step >= {win}) * 1000, 1) AS probed_median_ms, \
         round((SELECT median({STEP_SEC}) FROM {TIMING_TABLE} \
           WHERE {IS_SHADOW} = 0 AND {IS_SAMPLED} = 1 AND local_step >= {win}) * 1000, 1) AS sampled_median_ms, \
         round((SELECT median({STEP_SEC}) FROM {TIMING_TABLE} \
           WHERE {IS_SHADOW} = 1 AND local_step >= {win}) * 1000, 1) AS shadow_median_ms, \
         round((SELECT avg({STEP_SEC}) FROM {TIMING_TABLE} \
           WHERE {IS_SHADOW} = 0 AND local_step >= {win}) * 1000, 1) AS probed_mean_ms, \
         round((SELECT avg({STEP_SEC}) FROM {TIMING_TABLE} \
           WHERE {IS_SHADOW} = 1 AND local_step >= {win}) * 1000, 1) AS shadow_mean_ms, \
         (SELECT count(*) FROM {TIMING_TABLE} \
           WHERE {IS_SHADOW} = 0 AND local_step >= {win}) AS probed_n, \
         (SELECT count(*) FROM {TIMING_TABLE} \
           WHERE {IS_SHADOW} = 0 AND {IS_SAMPLED} = 1 AND local_step >= {win}) AS sampled_n, \
         (SELECT count(*) FROM {TIMING_TABLE} \
           WHERE {IS_SHADOW} = 1 AND local_step >= {win}) AS shadow_n, \
         (SELECT max(local_step) FROM {TIMING_TABLE}) AS latest_step, \
         (SELECT {SHADOW_NORMAL} FROM {TIMING_TABLE} ORDER BY local_step DESC LIMIT 1) AS shadow_normal, \
         (SELECT {SHADOW_BASELINE} FROM {TIMING_TABLE} ORDER BY local_step DESC LIMIT 1) AS shadow_baseline, \
         (SELECT {SAMPLE_RATE} FROM {TIMING_TABLE} ORDER BY local_step DESC LIMIT 1) AS sample_rate, \
         (SELECT sample_mode FROM {TIMING_TABLE} ORDER BY local_step DESC LIMIT 1) AS sample_mode"
    )
}

pub fn recent_steps() -> String {
    format!(
        "SELECT local_step, \
         round({STEP_SEC} * 1000, 1) AS duration_ms, {IS_SHADOW} AS is_shadow, {IS_SAMPLED} AS sampled \
         FROM {TIMING_TABLE} \
         WHERE local_step >= GREATEST(COALESCE((SELECT max(local_step) FROM {TIMING_TABLE}), 0) - {RECENT_STEPS}, 1) \
         ORDER BY local_step"
    )
}

pub const TRAIN_STEP_MEDIAN: &str =
    "SELECT round(median((CAST(e.time AS BIGINT) - CAST(s.time AS BIGINT)) / 1000000.0), 1) AS train_step_median_ms \
     FROM python.trace_event s \
     JOIN python.trace_event e ON s.span_id = e.span_id AND e.record_type = 'span_end' \
     WHERE s.record_type = 'span_start' AND s.name = 'train.step'";

pub const NCCL_COUNTERS: &str = "SELECT coll_events, rows_written, pool_exhausted, write_errors \
     FROM nccl.profiler_counters ORDER BY ts DESC LIMIT 1";
