//! Heap-compact stack payload for sampler aggregation.
//!
//! Sampling stores one [`CompactStack`] per fingerprint bucket instead of a full
//! fixed-size [`StackSnapshot`] (~1.4 KiB of zero padding). Reconstruct a
//! snapshot only when folding/exporting.

use crate::features::stacktrace::snapshot::{
    StackFlags, StackSnapshot, StackSource, MAX_NATIVE, MAX_PY,
};

/// Variable-length stack shape: only used native PCs + Python keys.
#[derive(Clone, Debug)]
pub struct CompactStack {
    pub tid: u64,
    pub source: StackSource,
    pub flags: StackFlags,
    native: Box<[usize]>,
    py: Box<[usize]>,
}

impl CompactStack {
    pub fn from_snapshot(s: &StackSnapshot) -> Self {
        let nlen = (s.native_len as usize).min(MAX_NATIVE);
        let plen = (s.py_len as usize).min(MAX_PY);
        Self {
            tid: s.tid,
            source: s.source,
            flags: s.flags,
            native: s.native[..nlen].into(),
            py: s.py[..plen].into(),
        }
    }

    pub fn native(&self) -> &[usize] {
        &self.native
    }

    pub fn py(&self) -> &[usize] {
        &self.py
    }

    /// Rebuild a fixed POD snapshot for the parse/fold path.
    pub fn to_snapshot(&self) -> StackSnapshot {
        StackSnapshot::from_parts(self.tid, self.source, &self.native, &self.py, self.flags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_preserves_used_frames() {
        let snap = StackSnapshot::from_parts(
            7,
            StackSource::Sigprof,
            &[0x10, 0x20, 0x30],
            &[0x100, 0x200],
            StackFlags::empty(),
        );
        let c = CompactStack::from_snapshot(&snap);
        assert_eq!(c.native(), &[0x10, 0x20, 0x30]);
        assert_eq!(c.py(), &[0x100, 0x200]);
        let back = c.to_snapshot();
        assert_eq!(back.tid, 7);
        assert_eq!(back.native_len, 3);
        assert_eq!(back.py_len, 2);
        assert_eq!(&back.native[..3], &[0x10, 0x20, 0x30]);
        assert_eq!(&back.py[..2], &[0x100, 0x200]);
    }

    #[test]
    fn smaller_than_full_snapshot_for_short_stacks() {
        let snap = StackSnapshot::from_parts(
            1,
            StackSource::Sigprof,
            &[1usize; 4],
            &[2usize; 2],
            StackFlags::empty(),
        );
        let c = CompactStack::from_snapshot(&snap);
        let compact_bytes = std::mem::size_of_val(&*c.native)
            + std::mem::size_of_val(&*c.py)
            + std::mem::size_of::<CompactStack>();
        assert!(compact_bytes < std::mem::size_of::<StackSnapshot>());
    }
}
