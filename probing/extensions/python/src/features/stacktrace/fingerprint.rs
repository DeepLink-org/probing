//! Aggregation keys for stack samples (no symbolize / alloc).
//!
//! # Contract
//!
//! - Key covers **tid + flags + native PCs + Python keys** (not [`StackSource`]).
//! - The SIGPROF consumer currently **accepts only the Python main OS tid**;
//!   including `tid` keeps buckets correct if multi-thread sampling is enabled later.
//! - Collisions are statistically rare; export folds a representative stack per bucket.

use crate::features::stacktrace::compact::CompactStack;
use crate::features::stacktrace::snapshot::StackSnapshot;

/// 64-bit FNV-1a style fingerprint over tid + flags + PCs + py keys.
pub fn fingerprint(snapshot: &StackSnapshot) -> u64 {
    fingerprint_parts(
        snapshot.tid,
        snapshot.flags.0,
        &snapshot.native[..snapshot.native_len as usize],
        &snapshot.py[..snapshot.py_len as usize],
    )
}

/// Same key as [`fingerprint`] for a compacted sampler payload.
pub fn fingerprint_compact(stack: &CompactStack) -> u64 {
    fingerprint_parts(stack.tid, stack.flags.0, stack.native(), stack.py())
}

fn fingerprint_parts(tid: u64, flags: u16, native: &[usize], py: &[usize]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut h = OFFSET;
    let mix = |h: &mut u64, v: u64| {
        *h ^= v;
        *h = h.wrapping_mul(PRIME);
    };

    mix(&mut h, tid);
    mix(&mut h, native.len() as u64);
    mix(&mut h, py.len() as u64);
    mix(&mut h, flags as u64);

    for &pc in native {
        mix(&mut h, pc as u64);
    }
    for &key in py {
        mix(&mut h, key as u64);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::stacktrace::snapshot::{StackFlags, StackSource};

    #[test]
    fn same_content_same_fingerprint() {
        let a = StackSnapshot::from_parts(
            1,
            StackSource::Sigprof,
            &[0x10, 0x20],
            &[0x100, 0x200],
            StackFlags::empty(),
        );
        let mut b = a;
        b.source = StackSource::Sigusr2; // source is not part of the key
        assert_eq!(fingerprint(&a), fingerprint(&b));
        assert_eq!(
            fingerprint(&a),
            fingerprint_compact(&CompactStack::from_snapshot(&a))
        );
    }

    #[test]
    fn different_tid_different_fingerprint() {
        let a =
            StackSnapshot::from_parts(1, StackSource::Sigprof, &[0x10], &[], StackFlags::empty());
        let mut b = a;
        b.tid = 2;
        assert_ne!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn different_pc_different_fingerprint() {
        let a =
            StackSnapshot::from_parts(1, StackSource::Sigprof, &[0x10], &[], StackFlags::empty());
        let b =
            StackSnapshot::from_parts(1, StackSource::Sigprof, &[0x11], &[], StackFlags::empty());
        assert_ne!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn different_py_key_different_fingerprint() {
        let a =
            StackSnapshot::from_parts(1, StackSource::Sigprof, &[], &[0xabc], StackFlags::empty());
        let b =
            StackSnapshot::from_parts(1, StackSource::Sigprof, &[], &[0xabd], StackFlags::empty());
        assert_ne!(fingerprint(&a), fingerprint(&b));
    }
}
