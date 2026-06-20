//! Fixed-size slot pools (no heap alloc on NCCL callback hot path).

use std::mem::MaybeUninit;

pub const INVALID_IDX: u32 = u32::MAX;

pub struct SlotPool<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
}

struct Slot<T> {
    value: MaybeUninit<T>,
    live: bool,
}

impl<T> SlotPool<T> {
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

    pub fn index_of(&self, ptr: *mut T) -> Option<u32> {
        for (i, slot) in self.slots.iter().enumerate() {
            if slot.live && std::ptr::eq(slot.value.as_ptr(), ptr) {
                return Some(i as u32);
            }
        }
        None
    }

    pub fn get_mut(&mut self, idx: u32) -> Option<&mut T> {
        let slot = self.slots.get_mut(idx as usize)?;
        if slot.live {
            Some(unsafe { slot.value.assume_init_mut() })
        } else {
            None
        }
    }

    pub fn free_ptr(&mut self, ptr: *mut T) {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct X(u32);

    #[test]
    fn alloc_free_roundtrip() {
        let mut pool = SlotPool::with_capacity(4);
        let (p, idx) = pool.alloc(|| X(7)).unwrap();
        assert_eq!(unsafe { (*p).0 }, 7);
        assert_eq!(idx, 0);
        pool.free_idx(idx);
        let (p2, idx2) = pool.alloc(|| X(9)).unwrap();
        assert_eq!(idx2, 0);
        assert_eq!(unsafe { (*p2).0 }, 9);
    }
}
