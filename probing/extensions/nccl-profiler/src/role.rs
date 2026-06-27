//! Parallel role ranks from training env (aligned with ``python.probing.parallel``).

#[derive(Clone, Copy, Debug, Default)]
pub struct RoleRanks {
    pub tp_rank: i32,
    pub pp_rank: i32,
    pub dp_rank: i32,
}

fn read_env_i32(keys: &[&str]) -> i32 {
    for key in keys {
        if let Ok(raw) = std::env::var(key) {
            if let Ok(v) = raw.trim().parse::<i32>() {
                if v >= 0 {
                    return v;
                }
            }
        }
    }
    -1
}

pub fn snapshot() -> RoleRanks {
    RoleRanks {
        tp_rank: read_env_i32(&["TENSOR_MODEL_PARALLEL_RANK", "TP_RANK", "PROBING_TP_RANK"]),
        pp_rank: read_env_i32(&["PIPELINE_MODEL_PARALLEL_RANK", "PP_RANK", "PROBING_PP_RANK"]),
        dp_rank: read_env_i32(&["DATA_PARALLEL_RANK", "DP_RANK", "PROBING_DP_RANK"]),
    }
}

#[cfg(test)]
mod tests {
    use super::snapshot;
    use std::sync::Mutex;

    static ENV_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn default_role_ranks_are_negative() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        for key in [
            "TENSOR_MODEL_PARALLEL_RANK",
            "TP_RANK",
            "PROBING_TP_RANK",
            "PIPELINE_MODEL_PARALLEL_RANK",
            "PP_RANK",
            "PROBING_PP_RANK",
            "DATA_PARALLEL_RANK",
            "DP_RANK",
            "PROBING_DP_RANK",
        ] {
            std::env::remove_var(key);
        }
        let r = snapshot();
        assert_eq!(r.tp_rank, -1);
        assert_eq!(r.pp_rank, -1);
        assert_eq!(r.dp_rank, -1);
    }

    #[test]
    fn snapshot_reads_tp_rank_env() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        std::env::set_var("TP_RANK", "3");
        let r = snapshot();
        assert_eq!(r.tp_rank, 3);
        std::env::remove_var("TP_RANK");
    }
}
