//! Fixed-size slot pools (no heap alloc on NCCL callback hot path).

use std::mem::MaybeUninit;

pub const INVALID_IDX: u32 = u32::MAX;

/// Slot types embed their own pool index so `index_of` is O(1): the NCCL
/// callback hot path resolves handles by reading the index back from the
/// slot instead of scanning the pool.
pub trait Indexed {
    fn set_self_idx(&mut self, idx: u32);
    fn self_idx(&self) -> u32;
}

pub struct SlotPool<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
}

struct Slot<T> {
    value: MaybeUninit<T>,
    live: bool,
}

impl<T: Indexed> SlotPool<T> {
    pub fn with_capacity(cap: usize) -> Self {
        let mut free: Vec<u32> = (0..cap as u32).collect();
        free.reverse();
        Self {
            slots: (0..cap)
                .map(|_| Slot {
                    value: MaybeUninit::uninit(),
                    live: false,
                })
                .collect(),
            free,
        }
    }

    pub fn alloc(&mut self, init: impl FnOnce() -> T) -> Option<(*mut T, u32)> {
        let idx = self.free.pop()?;
        let slot = &mut self.slots[idx as usize];
        debug_assert!(!slot.live);
        slot.value.write(init());
        slot.live = true;
        let ptr = slot.value.as_mut_ptr();
        Some((ptr, idx))
    }

    /// Resolve a pointer back to its pool index in O(1).
    ///
    /// Reads the embedded index and validates it (bounds, liveness, pointer
    /// identity), so stale or foreign pool pointers return `None` instead of
    /// aliasing another slot.
    ///
    /// # Safety
    ///
    /// `ptr` must be readable as a `T` (in practice: an event handle that
    /// this plugin previously returned to NCCL from some pool's `alloc`).
    pub unsafe fn index_of(&self, ptr: *mut T) -> Option<u32> {
        if ptr.is_null() {
            return None;
        }
        let packed = (*ptr).self_idx();
        let idx = crate::shard::slot_index(packed);
        let slot = self.slots.get(idx as usize)?;
        if slot.live && std::ptr::eq(slot.value.as_ptr(), ptr) {
            Some(idx)
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, idx: u32) -> Option<&mut T> {
        let slot = self.slots.get_mut(idx as usize)?;
        if slot.live {
            Some(unsafe { slot.value.assume_init_mut() })
        } else {
            None
        }
    }

    /// Visit every live slot (read-only). Used by the in-flight snapshot scan.
    pub fn for_each_live(&self, mut f: impl FnMut(u32, &T)) {
        for (i, slot) in self.slots.iter().enumerate() {
            if slot.live {
                f(i as u32, unsafe { slot.value.assume_init_ref() });
            }
        }
    }

    /// # Safety
    ///
    /// Same contract as [`SlotPool::index_of`].
    #[allow(dead_code)]
    pub unsafe fn free_ptr(&mut self, ptr: *mut T) {
        if let Some(idx) = self.index_of(ptr) {
            self.free_idx(idx);
        }
    }

    pub fn free_idx(&mut self, idx: u32) {
        let slot = match self.slots.get_mut(idx as usize) {
            Some(s) if s.live => s,
            _ => return,
        };
        slot.live = false;
        unsafe { slot.value.assume_init_drop() };
        self.free.push(idx);
    }

    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    pub fn live_count(&self) -> usize {
        self.capacity().saturating_sub(self.free.len())
    }

    /// Live slots as a percentage of capacity (0–100).
    pub fn usage_pct(&self) -> u8 {
        let cap = self.capacity();
        if cap == 0 {
            return 0;
        }
        ((self.live_count() * 100) / cap).min(100) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct X {
        v: u32,
        idx: u32,
    }

    impl X {
        fn new(v: u32) -> Self {
            Self {
                v,
                idx: INVALID_IDX,
            }
        }
    }

    impl Indexed for X {
        fn set_self_idx(&mut self, idx: u32) {
            self.idx = idx;
        }
        fn self_idx(&self) -> u32 {
            self.idx
        }
    }

    #[test]
    fn alloc_free_roundtrip() {
        let mut pool = SlotPool::with_capacity(4);
        let (p, idx) = pool.alloc(|| X::new(7)).unwrap();
        unsafe {
            (*p).set_self_idx(idx);
        }
        assert_eq!(unsafe { (*p).v }, 7);
        assert_eq!(idx, 0);
        pool.free_idx(idx);
        let (p2, idx2) = pool.alloc(|| X::new(9)).unwrap();
        unsafe {
            (*p2).set_self_idx(idx2);
        }
        assert_eq!(idx2, 0);
        assert_eq!(unsafe { (*p2).v }, 9);
    }

    #[test]
    fn index_of_is_validated() {
        let mut pool = SlotPool::with_capacity(2);
        let (p, idx) = pool.alloc(|| X::new(1)).unwrap();
        unsafe {
            (*p).set_self_idx(idx);
        }
        assert_eq!(unsafe { pool.index_of(p) }, Some(idx));
        // Stale handle after free must not resolve.
        pool.free_idx(idx);
        assert_eq!(unsafe { pool.index_of(p) }, None);
        // A pointer from another pool must not resolve here either.
        let mut other = SlotPool::with_capacity(2);
        let (q, qidx) = other.alloc(|| X::new(2)).unwrap();
        unsafe {
            (*q).set_self_idx(qidx);
        }
        assert_eq!(unsafe { pool.index_of(q) }, None);
    }

    #[test]
    fn usage_pct_tracks_live_slots() {
        let mut pool = SlotPool::with_capacity(4);
        assert_eq!(pool.usage_pct(), 0);
        let (_, idx) = pool.alloc(|| X::new(1)).unwrap();
        pool.get_mut(idx).unwrap().set_self_idx(idx);
        assert_eq!(pool.usage_pct(), 25);
        pool.free_idx(idx);
        assert_eq!(pool.usage_pct(), 0);
    }
}
