//! Fixed slot-pool sizing (`PROBING_NCCL_MAX_*_SLOTS`).

const DEFAULT_MAX_COLL: usize = 512;
const DEFAULT_MAX_PROXY_OP: usize = 8192;
const DEFAULT_MAX_PROXY_STEP: usize = 32768;
const DEFAULT_MAX_KERNEL_CH: usize = 8192;
const DEFAULT_MAX_NET: usize = 4096;

const MIN_SLOTS: usize = 64;
const MAX_SLOTS: usize = 1_048_576;

#[derive(Clone, Copy, Debug)]
pub struct PoolLimits {
    pub coll: usize,
    pub proxy_op: usize,
    pub proxy_step: usize,
    pub kernel_ch: usize,
    pub net: usize,
}

impl Default for PoolLimits {
    fn default() -> Self {
        Self {
            coll: DEFAULT_MAX_COLL,
            proxy_op: DEFAULT_MAX_PROXY_OP,
            proxy_step: DEFAULT_MAX_PROXY_STEP,
            kernel_ch: DEFAULT_MAX_KERNEL_CH,
            net: DEFAULT_MAX_NET,
        }
    }
}

/// Divide global limits across pool shards (each shard owns a fraction).
pub fn per_shard_limits(total: PoolLimits, shards: usize) -> PoolLimits {
    let n = shards.max(1);
    PoolLimits {
        coll: (total.coll / n).max(32),
        proxy_op: (total.proxy_op / n).max(256),
        proxy_step: (total.proxy_step / n).max(1024),
        kernel_ch: (total.kernel_ch / n).max(256),
        net: (total.net / n).max(128),
    }
}

fn parse_env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .map(|v| v.clamp(MIN_SLOTS, MAX_SLOTS))
        .unwrap_or(default)
}

/// Resolved slot limits (read once per process).
pub fn pool_limits() -> PoolLimits {
    static LIMITS: once_cell::sync::Lazy<PoolLimits> = once_cell::sync::Lazy::new(|| PoolLimits {
        coll: parse_env_usize("PROBING_NCCL_MAX_COLL_SLOTS", DEFAULT_MAX_COLL),
        proxy_op: parse_env_usize("PROBING_NCCL_MAX_PROXY_OP_SLOTS", DEFAULT_MAX_PROXY_OP),
        proxy_step: parse_env_usize("PROBING_NCCL_MAX_PROXY_STEP_SLOTS", DEFAULT_MAX_PROXY_STEP),
        kernel_ch: parse_env_usize("PROBING_NCCL_MAX_KERNEL_CH_SLOTS", DEFAULT_MAX_KERNEL_CH),
        net: parse_env_usize("PROBING_NCCL_MAX_NET_SLOTS", DEFAULT_MAX_NET),
    });
    *LIMITS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_larger_than_legacy_hardcodes() {
        let limits = PoolLimits::default();
        assert!(limits.coll >= 512);
        assert!(limits.proxy_op >= 8192);
        assert!(limits.proxy_step >= 32768);
    }
}
