use super::ApiClient;
use crate::overhead::sql::{self, NCCL_COUNTERS, TRAIN_STEP_MEDIAN};
use crate::utils::error::{AppError, Result};
use probing_proto::prelude::DataFrame;

pub const OVERHEAD_POLL_MS: u32 = 5000;

pub fn empty_dataframe() -> DataFrame {
    DataFrame {
        names: vec![],
        cols: vec![],
        size: 0,
    }
}

pub fn is_nccl_counters_missing(err: &AppError) -> bool {
    matches!(err, AppError::Api(msg)
        if msg.contains("nccl.profiler_counters") && msg.contains("not found"))
}

impl ApiClient {
    pub async fn fetch_overhead_summary(&self) -> Result<DataFrame> {
        self.execute_query(&sql::summary()).await
    }

    pub async fn fetch_overhead_recent_steps(&self) -> Result<DataFrame> {
        self.execute_query(&sql::recent_steps()).await
    }

    pub async fn fetch_overhead_train_step_median(&self) -> Result<DataFrame> {
        self.execute_query(TRAIN_STEP_MEDIAN).await
    }

    /// Latest NCCL profiler health row for the overhead footnote.
    ///
    /// Returns `Ok(None)` when the extension table is not registered (single-GPU / no NCCL).
    pub async fn fetch_overhead_nccl_counters(&self) -> Result<Option<DataFrame>> {
        match self.execute_query(NCCL_COUNTERS).await {
            Ok(df) => Ok(Some(df)),
            Err(e) if is_nccl_counters_missing(&e) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_missing_nccl_counters_table() {
        let err = AppError::Api(
            "Error during planning: table 'probe.nccl.profiler_counters' not found".into(),
        );
        assert!(is_nccl_counters_missing(&err));
    }
}
