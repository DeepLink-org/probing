//! Signal-safe stack capture document.
//!
//! [`StackSnapshot`] is the only structure the signal path may write. Order
//! invariants: `native` is leaf → root; `py` is outer → inner (`PYSTACKS`).

use core::fmt;

pub const MAX_NATIVE: usize = 48;
pub const MAX_PY: usize = 128;

/// How this snapshot was filled.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StackSource {
    #[default]
    Unknown = 0,
    /// Python TLS / `PYSTACKS` keys only (no native walk).
    Vm = 1,
    /// `SIGPROF` sampling.
    Sigprof = 2,
    /// `SIGUSR2` on-demand capture.
    Sigusr2 = 3,
    /// Synchronous `backtrace` walk on the current thread.
    SyncWalk = 4,
}

impl fmt::Display for StackSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unknown => "unknown",
            Self::Vm => "vm",
            Self::Sigprof => "sigprof",
            Self::Sigusr2 => "sigusr2",
            Self::SyncWalk => "sync_walk",
        })
    }
}

/// Compact quality / truncation bits for a snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(transparent)]
pub struct StackFlags(pub u16);

impl StackFlags {
    pub const EMPTY: Self = Self(0);
    pub const NATIVE_TRUNCATED: Self = Self(0b0001);
    pub const PY_TRUNCATED: Self = Self(0b0010);
    pub const PY_TORN: Self = Self(0b0100);
    pub const PY_ABSENT: Self = Self(0b1000);

    pub const fn empty() -> Self {
        Self::EMPTY
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

/// POD snapshot shared by SIGPROF / SIGUSR2 / sync fill adapters.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct StackSnapshot {
    pub tid: u64,
    pub source: StackSource,
    pub flags: StackFlags,
    pub native_len: u32,
    pub py_len: u32,
    /// Native return addresses, leaf → root.
    pub native: [usize; MAX_NATIVE],
    /// Callee `PyCodeObject` pointers, outer → inner.
    pub py: [usize; MAX_PY],
}

impl StackSnapshot {
    pub const fn zeroed() -> Self {
        Self {
            tid: 0,
            source: StackSource::Unknown,
            flags: StackFlags::EMPTY,
            native_len: 0,
            py_len: 0,
            native: [0usize; MAX_NATIVE],
            py: [0usize; MAX_PY],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.native_len == 0 && self.py_len == 0
    }

    /// Test / adapter helper: build a snapshot from PC and Python keys.
    pub fn from_parts(
        tid: u64,
        source: StackSource,
        native_leaf_to_root: &[usize],
        py_outer_to_inner: &[usize],
        flags: StackFlags,
    ) -> Self {
        let mut s = Self::zeroed();
        s.tid = tid;
        s.source = source;
        s.flags = flags;
        let nlen = native_leaf_to_root.len().min(MAX_NATIVE);
        s.native[..nlen].copy_from_slice(&native_leaf_to_root[..nlen]);
        s.native_len = nlen as u32;
        if native_leaf_to_root.len() > MAX_NATIVE {
            s.flags.insert(StackFlags::NATIVE_TRUNCATED);
        }
        let plen = py_outer_to_inner.len().min(MAX_PY);
        s.py[..plen].copy_from_slice(&py_outer_to_inner[..plen]);
        s.py_len = plen as u32;
        if py_outer_to_inner.len() > MAX_PY {
            s.flags.insert(StackFlags::PY_TRUNCATED);
        }
        if plen == 0 {
            s.flags.insert(StackFlags::PY_ABSENT);
        }
        s
    }
}

/// Compatibility alias for older call sites.
pub type RawStackSnapshot = StackSnapshot;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_parts_sets_truncation_flags() {
        let native: Vec<usize> = (0..MAX_NATIVE + 3).collect();
        let s =
            StackSnapshot::from_parts(1, StackSource::SyncWalk, &native, &[], StackFlags::empty());
        assert_eq!(s.native_len as usize, MAX_NATIVE);
        assert!(s.flags.contains(StackFlags::NATIVE_TRUNCATED));
        assert!(s.flags.contains(StackFlags::PY_ABSENT));
        assert_eq!(s.source, StackSource::SyncWalk);
    }

    #[test]
    fn empty_zeroed() {
        assert!(StackSnapshot::zeroed().is_empty());
    }
}
