//! Shard routing for partitioned slot pools (reduces callback lock contention).
//!
//! Slot indices pack `(shard_id << 24) | local_idx` into each handle's
//! `self_idx` so `stop_event` / `record_state` lock only one shard.

use std::ffi::c_void;

use once_cell::sync::Lazy;

use crate::events::{
    event_type, CollSlot, KernelChSlot, NetPluginSlot, ProxyOpSlot, ProxyStepSlot, EVT_COLL,
    EVT_KERNEL_CH, EVT_NET_PLUGIN, EVT_PROXY_OP, EVT_PROXY_STEP,
};
use crate::pool::Indexed;

const IDX_MASK: u32 = 0x00FF_FFFF;
const SHARD_SHIFT: u32 = 24;

const DEFAULT_SHARDS: usize = 8;
const MIN_SHARDS: usize = 1;
const MAX_SHARDS: usize = 64;

static SHARD_COUNT: Lazy<usize> = Lazy::new(|| {
    if cfg!(test) {
        return 1;
    }
    std::env::var("PROBING_NCCL_POOL_SHARDS")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .map(|n| n.clamp(MIN_SHARDS, MAX_SHARDS))
        .unwrap_or(DEFAULT_SHARDS)
});

pub fn pool_shard_count() -> usize {
    *SHARD_COUNT
}

#[inline]
pub fn pack_slot_id(shard: usize, local_idx: u32) -> u32 {
    debug_assert!(shard < MAX_SHARDS);
    (local_idx & IDX_MASK) | ((shard as u32) << SHARD_SHIFT)
}

#[inline]
pub fn shard_id(packed: u32) -> usize {
    (packed >> SHARD_SHIFT) as usize
}

#[inline]
pub fn slot_index(packed: u32) -> u32 {
    packed & IDX_MASK
}

pub fn shard_for_comm(comm_hash: u64) -> usize {
    let n = pool_shard_count();
    (comm_hash as usize) % n
}

/// Read the shard id embedded in a live NCCL event handle (lock-free).
pub fn shard_for_handle(handle: *mut c_void) -> Option<usize> {
    if handle.is_null() {
        return None;
    }
    let packed = packed_self_idx(handle)?;
    Some(shard_id(packed))
}

fn packed_self_idx(handle: *mut c_void) -> Option<u32> {
    // SAFETY: NCCL only passes back handles this plugin previously returned.
    Some(unsafe {
        match event_type(handle) {
            EVT_COLL => (*(handle as *const CollSlot)).self_idx(),
            EVT_PROXY_OP => (*(handle as *const ProxyOpSlot)).self_idx(),
            EVT_PROXY_STEP => (*(handle as *const ProxyStepSlot)).self_idx(),
            EVT_KERNEL_CH => (*(handle as *const KernelChSlot)).self_idx(),
            EVT_NET_PLUGIN => (*(handle as *const NetPluginSlot)).self_idx(),
            _ => return None,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_roundtrip() {
        let packed = pack_slot_id(3, 42);
        assert_eq!(shard_id(packed), 3);
        assert_eq!(slot_index(packed), 42);
    }
}
