//! Parallel role ranks from training env (aligned with ``python.probing.parallel``).

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};

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

/// Pull `tp` / `pp` / `dp` out of a canonical role key such as
/// `"cp=0,dp=1,ep=0,pp=1,tp=0"`.
fn parse_role_key(role: &str) -> RoleRanks {
    let mut ranks = RoleRanks {
        tp_rank: -1,
        pp_rank: -1,
        dp_rank: -1,
    };
    for part in role.split(',') {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let Ok(value) = value.trim().parse::<i32>() else {
            continue;
        };
        if value < 0 {
            continue;
        }
        match key.trim() {
            "tp" => ranks.tp_rank = value,
            "pp" => ranks.pp_rank = value,
            "dp" => ranks.dp_rank = value,
            _ => {}
        }
    }
    ranks
}

pub fn snapshot() -> RoleRanks {
    let mut ranks = RoleRanks {
        tp_rank: read_env_i32(&["TENSOR_MODEL_PARALLEL_RANK", "TP_RANK", "PROBING_TP_RANK"]),
        pp_rank: read_env_i32(&["PIPELINE_MODEL_PARALLEL_RANK", "PP_RANK", "PROBING_PP_RANK"]),
        dp_rank: read_env_i32(&["DATA_PARALLEL_RANK", "DP_RANK", "PROBING_DP_RANK"]),
    };
    if ranks.tp_rank >= 0 && ranks.pp_rank >= 0 && ranks.dp_rank >= 0 {
        return ranks;
    }
    // Megatron keeps its parallel ranks in `parallel_state` and never exports
    // the env vars above, so without this the columns stay -1 on exactly the
    // jobs the topology analysis is written for. `probing.set_role()` publishes
    // the same dimensions as a role key.
    if let Ok(role) = std::env::var("PROBING_NODE_ROLE") {
        let from_role = parse_role_key(&role);
        if ranks.tp_rank < 0 {
            ranks.tp_rank = from_role.tp_rank;
        }
        if ranks.pp_rank < 0 {
            ranks.pp_rank = from_role.pp_rank;
        }
        if ranks.dp_rank < 0 {
            ranks.dp_rank = from_role.dp_rank;
        }
    }
    ranks
}

static CACHED_TP: AtomicI32 = AtomicI32::new(-1);
static CACHED_PP: AtomicI32 = AtomicI32::new(-1);
static CACHED_DP: AtomicI32 = AtomicI32::new(-1);
static ROLE_RESOLVED: AtomicBool = AtomicBool::new(false);
static SNAPSHOT_ATTEMPTS: AtomicU32 = AtomicU32::new(0);

/// How many calls to skip between re-reads while the role is still unknown.
const RESNAPSHOT_EVERY: u32 = 256;

fn load_cached() -> RoleRanks {
    RoleRanks {
        tp_rank: CACHED_TP.load(Ordering::Relaxed),
        pp_rank: CACHED_PP.load(Ordering::Relaxed),
        dp_rank: CACHED_DP.load(Ordering::Relaxed),
    }
}

/// Role ranks for the current process.
///
/// Once known they are fixed for the lifetime of a training process, and
/// `std::env::var` takes the process env lock — not something to pay on every
/// completed row. They are not known at the first NCCL event though: Megatron
/// only builds its parallel state after the process group is up, by which point
/// collectives have already been profiled. So an unresolved role is re-read
/// periodically instead of being frozen on first use.
pub fn cached() -> RoleRanks {
    if ROLE_RESOLVED.load(Ordering::Relaxed) {
        return load_cached();
    }
    if SNAPSHOT_ATTEMPTS.fetch_add(1, Ordering::Relaxed) % RESNAPSHOT_EVERY != 0 {
        return load_cached();
    }
    let ranks = snapshot();
    if ranks.tp_rank >= 0 || ranks.pp_rank >= 0 || ranks.dp_rank >= 0 {
        CACHED_TP.store(ranks.tp_rank, Ordering::Relaxed);
        CACHED_PP.store(ranks.pp_rank, Ordering::Relaxed);
        CACHED_DP.store(ranks.dp_rank, Ordering::Relaxed);
        ROLE_RESOLVED.store(true, Ordering::Relaxed);
    }
    ranks
}

#[cfg(test)]
pub(crate) fn reset_cached_for_tests() {
    CACHED_TP.store(-1, Ordering::Relaxed);
    CACHED_PP.store(-1, Ordering::Relaxed);
    CACHED_DP.store(-1, Ordering::Relaxed);
    ROLE_RESOLVED.store(false, Ordering::Relaxed);
    SNAPSHOT_ATTEMPTS.store(0, Ordering::Relaxed);
}

/// Global torch rank for counter snapshots (`RANK` / `LOCAL_RANK`).
pub fn training_rank() -> i32 {
    read_env_i32(&["RANK", "LOCAL_RANK"])
}

#[cfg(test)]
mod tests {
    use super::{cached, reset_cached_for_tests, snapshot};
    use std::sync::Mutex;

    static ENV_TEST_LOCK: Mutex<()> = Mutex::new(());

    const ALL_KEYS: [&str; 10] = [
        "TENSOR_MODEL_PARALLEL_RANK",
        "TP_RANK",
        "PROBING_TP_RANK",
        "PIPELINE_MODEL_PARALLEL_RANK",
        "PP_RANK",
        "PROBING_PP_RANK",
        "DATA_PARALLEL_RANK",
        "DP_RANK",
        "PROBING_DP_RANK",
        "PROBING_NODE_ROLE",
    ];

    fn clear_env() {
        for key in ALL_KEYS {
            std::env::remove_var(key);
        }
    }

    #[test]
    fn default_role_ranks_are_negative() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        clear_env();
        let r = snapshot();
        assert_eq!(r.tp_rank, -1);
        assert_eq!(r.pp_rank, -1);
        assert_eq!(r.dp_rank, -1);
    }

    #[test]
    fn snapshot_reads_tp_rank_env() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        clear_env();
        std::env::set_var("TP_RANK", "3");
        let r = snapshot();
        assert_eq!(r.tp_rank, 3);
        clear_env();
    }

    #[test]
    fn snapshot_falls_back_to_the_role_key() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        clear_env();
        // What `probing.set_role()` publishes under Megatron.
        std::env::set_var("PROBING_NODE_ROLE", "cp=0,dp=1,ep=0,pp=1,tp=0");
        let r = snapshot();
        assert_eq!((r.tp_rank, r.pp_rank, r.dp_rank), (0, 1, 1));
        clear_env();
    }

    #[test]
    fn dedicated_env_wins_over_the_role_key() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        clear_env();
        std::env::set_var("PROBING_NODE_ROLE", "dp=1,pp=1,tp=0");
        std::env::set_var("TENSOR_MODEL_PARALLEL_RANK", "7");
        let r = snapshot();
        assert_eq!(r.tp_rank, 7);
        assert_eq!(r.pp_rank, 1);
        clear_env();
    }

    #[test]
    fn role_set_after_the_first_events_is_picked_up() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        clear_env();
        reset_cached_for_tests();

        // Megatron has not built its parallel state yet.
        assert_eq!(cached().tp_rank, -1);

        std::env::set_var("PROBING_NODE_ROLE", "dp=1,pp=1,tp=0");
        // Rows in between keep reporting unknown until the next re-read.
        let mut resolved = None;
        for _ in 0..(super::RESNAPSHOT_EVERY * 2) {
            let r = cached();
            if r.tp_rank >= 0 {
                resolved = Some(r);
                break;
            }
        }
        let r = resolved.expect("role should be re-read once it becomes available");
        assert_eq!((r.tp_rank, r.pp_rank, r.dp_rank), (0, 1, 1));

        // Once resolved it is frozen, so the env lock is not taken per row.
        std::env::remove_var("PROBING_NODE_ROLE");
        assert_eq!(cached().tp_rank, 0);

        clear_env();
        reset_cached_for_tests();
    }
}
