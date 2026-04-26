use crate::layout::Header;
use std::sync::atomic::Ordering;

/// Access `Header.refcount` as `&AtomicU32` via raw pointer projection.
///
/// Uses `addr_of!` to reach the field *without* creating an intermediate
/// `&Header` reference — the `&[u8]` → `&Header` route lets LLVM assume the
/// memory is `readonly`, conflicting with the atomic RMW in `acquire_ref` /
/// `release_ref`.
///
/// `#[inline(never)]` on the mutating callers provides an additional LLVM
/// optimisation barrier for provenance inherited from the `&[u8]` parameter.
#[inline(always)]
fn refcount_atomic(buf: &[u8]) -> &std::sync::atomic::AtomicU32 {
    let ptr = buf.as_ptr() as *const Header;
    unsafe { &*std::ptr::addr_of!((*ptr).refcount) }
}

pub fn refcount(buf: &[u8]) -> u32 {
    refcount_atomic(buf).load(Ordering::Acquire)
}

#[inline(never)]
pub fn acquire_ref(buf: &[u8]) -> u32 {
    refcount_atomic(buf).fetch_add(1, Ordering::Relaxed) + 1
}

/// When the count drops to zero, an `Acquire` fence ensures all prior
/// accesses from other holders are visible before the caller deallocates.
#[inline(never)]
pub fn release_ref(buf: &[u8]) -> u32 {
    let prev = refcount_atomic(buf).fetch_sub(1, Ordering::Release);
    debug_assert!(prev > 0, "release_ref on zero refcount");
    if prev == 1 {
        std::sync::atomic::fence(Ordering::Acquire);
    }
    prev - 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CachedReader;
    use crate::memtable::{MemTable, MemTableView, MemTableWriter};
    use crate::raw::init_buf;
    use crate::schema::{DType, Schema, Value};

    #[test]
    fn refcount_lifecycle() {
        let schema = Schema::new().col("x", DType::I32);
        let t = MemTable::new(&schema, 1024, 1);
        assert_eq!(t.refcount(), 1);

        assert_eq!(acquire_ref(t.as_bytes()), 2);
        assert_eq!(t.refcount(), 2);

        assert_eq!(release_ref(t.as_bytes()), 1);
        assert_eq!(t.refcount(), 1);

        assert_eq!(release_ref(t.as_bytes()), 0);
    }
    #[test]
    fn concurrent_refcount_stress() {
        use std::alloc;
        use std::sync::{Arc, Barrier};
        use std::thread;

        let schema = Schema::new().col("x", DType::I32);
        let size = MemTable::required_size(&schema, 1024, 1);
        let layout = alloc::Layout::from_size_align(size, 64).unwrap();
        let ptr = unsafe { alloc::alloc_zeroed(layout) };
        assert!(!ptr.is_null());

        unsafe {
            let buf = std::slice::from_raw_parts_mut(ptr, size);
            init_buf(buf, &schema, 1024, 1);
        }

        let num_threads = 16;
        let ops_per_thread = 1000;
        let barrier = Arc::new(Barrier::new(num_threads));
        let addr = ptr as usize;

        // each thread: acquire N refs then release N refs
        let handles: Vec<_> = (0..num_threads)
            .map(|_| {
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let buf = unsafe { std::slice::from_raw_parts(addr as *const u8, size) };
                    for _ in 0..ops_per_thread {
                        acquire_ref(buf);
                    }
                    for _ in 0..ops_per_thread {
                        release_ref(buf);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        // init set refcount=1, each thread net-zero → should be 1
        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            assert_eq!(refcount(buf), 1);
            release_ref(buf);
            alloc::dealloc(ptr, layout);
        }
    }

    #[test]
    fn stress_refcount_concurrent_dedup_lifecycle() {
        use std::alloc;
        use std::sync::{Arc, Barrier};
        use std::thread;

        let schema = Schema::new().col("tag", DType::Str).col("v", DType::I64);
        let size = MemTable::required_size(&schema, 16384, 4);
        let layout = alloc::Layout::from_size_align(size, 64).unwrap();
        let ptr = unsafe { alloc::alloc_zeroed(layout) };
        assert!(!ptr.is_null());

        unsafe {
            let buf = std::slice::from_raw_parts_mut(ptr, size);
            init_buf(buf, &schema, 16384, 4);
        }

        let num_producers = 4;
        let num_consumers = 4;
        let rows_per_producer = 200;
        let addr = ptr as usize;

        // acquire refs for all consumers
        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            for _ in 0..num_consumers {
                acquire_ref(buf);
            }
            assert_eq!(refcount(buf), 1 + num_consumers as u32);
        }

        let producer = thread::spawn(move || {
            let buf = unsafe { std::slice::from_raw_parts_mut(addr as *mut u8, size) };
            let mut mt = MemTableWriter::new(buf).unwrap();
            for tid in 0..num_producers {
                for i in 0..rows_per_producer as i64 {
                    mt.push_row(&[Value::Str("tag"), Value::I64(tid as i64 * 10000 + i)]);
                }
            }
        });

        producer.join().unwrap();

        // consumers verify data and release refs
        let consumer_barrier = Arc::new(Barrier::new(num_consumers));
        let consumer_handles: Vec<_> = (0..num_consumers)
            .map(|_| {
                let barrier = consumer_barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let buf = unsafe { std::slice::from_raw_parts(addr as *const u8, size) };
                    let view = MemTableView::new(buf).unwrap();
                    let mut cache = CachedReader::new(buf, 64);
                    let mut count = 0;
                    for chunk in 0..view.num_chunks() {
                        for row in view.rows(chunk) {
                            let mut c = cache.cursor(&row);
                            let tag = c.next_str();
                            let v = c.next_i64();
                            assert_eq!(tag, "tag");
                            assert!(v >= 0);
                            count += 1;
                        }
                    }
                    assert_eq!(count, num_producers * rows_per_producer);
                    release_ref(buf);
                })
            })
            .collect();

        for h in consumer_handles {
            h.join().unwrap();
        }

        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            assert_eq!(refcount(buf), 1);
            release_ref(buf);
            alloc::dealloc(ptr as *mut u8, layout);
        }
    }
}
