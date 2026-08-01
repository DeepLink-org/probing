//! Executable Loom models for the MEMT atomic protocols.
//!
//! These models intentionally duplicate only the control protocol, not the
//! byte layout. Keep the ordering mapping next to each transition so changes
//! to `raw.rs`, `row.rs`, or `writer.rs` must update the model in the same PR.

use loom::cell::UnsafeCell;
use loom::sync::atomic::{AtomicUsize, Ordering};
use loom::sync::Arc;
use loom::thread;

const EMPTY: usize = 0;
const WRITING: usize = 1;
const LEASE_CLAIMING: usize = 2;
const LEASE_ACTIVE: usize = 3;
const LEASE_RECYCLER: usize = 4;

/// Model `writer.rs::RowWriter::finish` / `raw.rs::push_row_raw` paired with
/// `pin_chunk`: payload bytes are initialized before the `used` Release store,
/// and readers acquire `used` before touching those bytes.
#[test]
fn published_extent_makes_the_complete_row_visible() {
    loom::model(|| {
        let payload = Arc::new(UnsafeCell::new((0usize, 0usize)));
        let used = Arc::new(AtomicUsize::new(0));

        let writer_payload = Arc::clone(&payload);
        let writer_used = Arc::clone(&used);
        let writer = thread::spawn(move || {
            writer_payload.with_mut(|ptr| unsafe {
                (*ptr).0 = 0xCAFE;
                (*ptr).1 = 0xBABE;
            });
            // Production mapping: ChunkHeader.used.store(..., Release).
            writer_used.store(2, Ordering::Release);
        });

        let reader_payload = Arc::clone(&payload);
        let reader_used = Arc::clone(&used);
        let reader = thread::spawn(move || {
            // Production mapping: pin_chunk loads ChunkHeader.used with Acquire.
            if reader_used.load(Ordering::Acquire) == 2 {
                let row = reader_payload.with(|ptr| unsafe { *ptr });
                assert_eq!(row, (0xCAFE, 0xBABE), "observed a torn published row");
            }
        });

        writer.join().unwrap();
        reader.join().unwrap();
    });
}

struct RecyclableChunk {
    state: AtomicUsize,
    generation: AtomicUsize,
    leases: [AtomicUsize; 2],
    used: AtomicUsize,
    // Separate atomic words let the model observe a torn logical row without
    // imposing Loom UnsafeCell's stronger Rust happens-before requirement on
    // file mappings shared across processes.
    payload_head: AtomicUsize,
    payload_tail: AtomicUsize,
}

impl RecyclableChunk {
    fn new() -> Self {
        Self {
            state: AtomicUsize::new(WRITING),
            generation: AtomicUsize::new(1),
            leases: [AtomicUsize::new(0), AtomicUsize::new(0)],
            used: AtomicUsize::new(1),
            payload_head: AtomicUsize::new(1),
            payload_tail: AtomicUsize::new(1),
        }
    }

    fn lease(&self, slot: usize) -> &AtomicUsize {
        &self.leases[slot]
    }

    /// Minimal faithful model of `row::pin_chunk` for one live process and
    /// two lease slots. Returns only after state/generation validation.
    fn read_pinned(&self) -> Option<(usize, usize, usize)> {
        let state_before = self.state.load(Ordering::SeqCst);
        if state_before == EMPTY {
            return None;
        }
        let generation_before = self.generation.load(Ordering::SeqCst);
        let slot = (0..self.leases.len()).find(|&slot| {
            self.lease(slot)
                .compare_exchange(0, LEASE_CLAIMING, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        })?;
        // Production mapping: owner_start_time is initialized while the
        // packed lease has a non-zero owner and refs==0. The recycler must
        // already treat that transient state as occupied.
        thread::yield_now();
        self.lease(slot).store(LEASE_ACTIVE, Ordering::SeqCst);

        let state_after = self.state.load(Ordering::SeqCst);
        let generation_after = self.generation.load(Ordering::SeqCst);
        let result = if state_before == state_after
            && state_after != EMPTY
            && generation_before == generation_after
        {
            // Production mapping: pin_chunk loads used with Acquire and only
            // exposes bytes below that published extent.
            if self.used.load(Ordering::Acquire) == 1 {
                Some((
                    generation_after,
                    self.payload_head.load(Ordering::Relaxed),
                    self.payload_tail.load(Ordering::Relaxed),
                ))
            } else {
                None
            }
        } else {
            None
        };
        self.lease(slot).store(0, Ordering::SeqCst);
        result
    }

    /// Minimal faithful model of `raw::advance_chunk_raw_detailed` for a
    /// recycle target. The intermediate EMPTY state blocks new valid pins;
    /// any lease, including a claim with refcount zero, blocks mutation.
    fn try_recycle_and_append(&self) -> bool {
        let previous_state = self.state.load(Ordering::SeqCst);
        self.state.store(EMPTY, Ordering::SeqCst);
        let mut reserved = 0;
        for slot in 0..self.leases.len() {
            if self
                .lease(slot)
                .compare_exchange(0, LEASE_CLAIMING, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                for rollback in 0..reserved {
                    self.lease(rollback).store(0, Ordering::SeqCst);
                }
                self.state.store(previous_state, Ordering::SeqCst);
                return false;
            }
            // Production uses the refs==0 claim state while initializing the
            // process identity, then publishes the non-incrementable recycler
            // marker. Both states exclude reader acquisition.
            thread::yield_now();
            self.lease(slot).store(LEASE_RECYCLER, Ordering::SeqCst);
            reserved += 1;
        }

        // Production mapping: advance resets extent metadata before the
        // AcqRel generation publication. This ordering closes the state ABA
        // window for a claimant that arrives after the final lease check.
        self.used.store(0, Ordering::Relaxed);
        let next_generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.state.store(WRITING, Ordering::Release);
        for slot in 0..reserved {
            self.lease(slot).store(0, Ordering::SeqCst);
        }

        // Model the next RowWriter append into the recycled byte range.
        self.payload_head.store(next_generation, Ordering::Relaxed);
        thread::yield_now();
        self.payload_tail.store(next_generation, Ordering::Relaxed);
        self.used.store(1, Ordering::Release);
        true
    }
}

/// Exhaustively checks the pin/recycle race. A successful reader must see both
/// independently-written payload words matching its stable generation.
#[test]
fn pinned_generation_never_exposes_torn_recycled_payload() {
    loom::model(|| {
        let chunk = Arc::new(RecyclableChunk::new());

        let reader_chunk = Arc::clone(&chunk);
        let reader = thread::spawn(move || reader_chunk.read_pinned());

        let writer_chunk = Arc::clone(&chunk);
        let writer = thread::spawn(move || writer_chunk.try_recycle_and_append());

        let read = reader.join().unwrap();
        let _recycled = writer.join().unwrap();
        if let Some((generation, head, tail)) = read {
            assert_eq!(head, generation, "reader observed a stale row head");
            assert_eq!(tail, generation, "reader observed a stale row tail");
        }
        for slot in 0..chunk.leases.len() {
            assert_eq!(
                chunk.lease(slot).load(Ordering::SeqCst),
                0,
                "reader release or partial recycler rollback leaked a lease"
            );
        }
    });
}
