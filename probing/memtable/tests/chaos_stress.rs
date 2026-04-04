//! 生产者 / 消费者 + ring wrap + `validate_buf` 的长时间混沌测试（约 65 秒）。
//!
//! 默认不跑（`#[ignore]`），手动执行：
//!
//! ```text
//! cargo test -p probing-memtable --test chaos_stress -- --ignored --nocapture
//! ```
//!
//! 注意：同一缓冲区的写端只能有一个线程持有 `DedupWriter`（`&mut`）；多写者并发在语言层面是 UB。
//! 本测试用**单写线程**内交替模拟两个逻辑生产者，保留相近写压力。

use probing_memtable::{
    validate_buf, CachedReader, DType, DedupWriter, MemTable, MemTableMut, MemTableView, Schema,
    Value,
};
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

fn next_u64(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

fn panic_message(err: Box<dyn std::any::Any + Send>) -> String {
    match err.downcast::<String>() {
        Ok(s) => *s,
        Err(err) => match err.downcast::<&'static str>() {
            Ok(s) => (*s).to_string(),
            Err(err) => format!("{err:?}"),
        },
    }
}

#[test]
#[ignore = "long-running chaos test; run manually"]
fn chaos_producer_consumer_wrap_stress_for_over_a_minute() {
    let schema = Schema::new()
        .col("topic", DType::Str)
        .col("msg", DType::Str)
        .col("seq", DType::I64);
    let chunk_size = 256u32;
    let num_chunks = 4u32;
    let need = MemTable::required_size(&schema, chunk_size as usize, num_chunks as usize);
    let mut buf = vec![0u8; need];
    {
        let _mt = MemTableMut::init(&mut buf, &schema, chunk_size, num_chunks);
    }
    let addr = buf.as_ptr() as usize;
    let buf_len = buf.len();

    let topics = ["alpha", "beta", "gamma", "delta", "epsilon"];
    let msgs = [
        "msg-0",
        "msg-1",
        "msg-2",
        "msg-3",
        "payload-xxxxxxxxxxxxxxxx",
        "payload-yyyyyyyyyyyyyyyy",
    ];
    let duration = Duration::from_secs(65);
    let start = Instant::now();
    let writes = Arc::new(AtomicUsize::new(0));
    let reads = Arc::new(AtomicUsize::new(0));
    let validations = Arc::new(AtomicUsize::new(0));
    let stale_panics = Arc::new(AtomicUsize::new(0));
    let manual_advances = Arc::new(AtomicUsize::new(0));
    // 1 写线程 + 2 读线程 + 1 校验线程
    let barrier = Arc::new(Barrier::new(4));

    let producer = {
        let barrier = barrier.clone();
        let writes = writes.clone();
        let manual_advances = manual_advances.clone();
        thread::spawn(move || {
            barrier.wait();
            let buf = unsafe { std::slice::from_raw_parts_mut(addr as *mut u8, buf_len) };
            let mut writer = DedupWriter::new(buf).unwrap();
            let mut seeds = [
                0xA5A5_5A5A_D3C3_B1B1u64 ^ 0,
                0xA5A5_5A5A_D3C3_B1B1u64 ^ 1,
            ];
            let mut seqs = [0i64, 1_000_000i64];
            while start.elapsed() < duration {
                for tid in 0..2usize {
                    let seed = &mut seeds[tid];
                    let seq = &mut seqs[tid];
                    let roll = (next_u64(seed) % 100) as u32;
                    let topic = topics[(next_u64(seed) as usize) % topics.len()];
                    let msg = msgs[(next_u64(seed) as usize) % msgs.len()];
                    if roll < 70 {
                        writer.push_row(&[Value::Str(topic), Value::Str(msg), Value::I64(*seq)]);
                        writes.fetch_add(1, Ordering::Relaxed);
                        *seq += 1;
                    } else if roll < 92 {
                        let ok = writer
                            .row_writer()
                            .put_str(topic)
                            .put_str(msg)
                            .put_i64(*seq)
                            .finish();
                        if ok {
                            writes.fetch_add(1, Ordering::Relaxed);
                            *seq += 1;
                        } else {
                            writer.advance_chunk();
                            manual_advances.fetch_add(1, Ordering::Relaxed);
                        }
                    } else {
                        writer.advance_chunk();
                        manual_advances.fetch_add(1, Ordering::Relaxed);
                    }
                }
                if next_u64(&mut seeds[0]) % 8 == 0 {
                    thread::yield_now();
                }
            }
        })
    };

    let consumer_handles: Vec<_> = (0..2)
        .map(|tid| {
            let barrier = barrier.clone();
            let reads = reads.clone();
            let stale_panics = stale_panics.clone();
            thread::spawn(move || {
                barrier.wait();
                let buf = unsafe { std::slice::from_raw_parts(addr as *const u8, buf_len) };
                let view = MemTableView::new(buf).unwrap();
                let mut cache = CachedReader::new(buf, 32);
                let mut seed = 0x1234_5678_9ABC_DEF0u64 ^ tid as u64;
                // 仅按时间结束：避免生产者 panic/未置位导致死循环
                while start.elapsed() < duration {
                    let scan = panic::catch_unwind(AssertUnwindSafe(|| {
                        let chunk = (next_u64(&mut seed) as usize) % view.num_chunks();
                        let mut local_reads = 0usize;
                        let iter = view.rows(chunk);
                        if !iter.is_valid() {
                            return 0usize;
                        }
                        for row in iter {
                            if next_u64(&mut seed) % 2 == 0 {
                                let topic = row.col_str(0);
                                let msg = row.col_str(1);
                                let seq = row.col_i64(2);
                                assert!(topics.contains(&topic), "bad topic: {topic}");
                                assert!(msgs.contains(&msg), "bad msg: {msg}");
                                assert!(seq >= 0, "bad seq: {seq}");
                            } else {
                                let mut c = cache.cursor(&row);
                                let topic = c.next_str();
                                let msg = c.next_str();
                                let seq = c.next_i64();
                                assert!(topics.contains(&topic), "bad topic: {topic}");
                                assert!(msgs.contains(&msg), "bad msg: {msg}");
                                assert!(seq >= 0, "bad seq: {seq}");
                            }
                            local_reads += 1;
                            if next_u64(&mut seed) % 16 == 0 {
                                thread::yield_now();
                            }
                        }
                        local_reads
                    }));
                    match scan {
                        Ok(n) => {
                            reads.fetch_add(n, Ordering::Relaxed);
                        }
                        Err(err) => {
                            let msg = panic_message(err);
                            if msg.contains("stale Row")
                                || msg.contains("stale RowIter")
                                || msg.contains("stale RowCursor")
                                || msg.contains("stale CachedReader")
                                || msg.contains("bad topic:")
                                || msg.contains("bad msg:")
                                || msg.contains("bad seq:")
                                || msg.contains("out of range for slice")
                                || msg.contains("range end index")
                                || msg.contains("range start index")
                                || msg.contains("chunk has been recycled")
                                || msg.contains("resolve_var")
                                || msg.contains("var_field_size")
                                || msg.contains("assertion `left == right`")
                                || msg.contains("index out of bounds")
                            {
                                stale_panics.fetch_add(1, Ordering::Relaxed);
                                thread::yield_now();
                            } else {
                                panic!("unexpected consumer panic: {msg}");
                            }
                        }
                    }
                }
            })
        })
        .collect();

    let validator = {
        let barrier = barrier.clone();
        let validations = validations.clone();
        thread::spawn(move || {
            barrier.wait();
            let buf = unsafe { std::slice::from_raw_parts(addr as *const u8, buf_len) };
            while start.elapsed() < duration {
                assert!(
                    validate_buf(buf).is_ok(),
                    "buffer validation failed during chaos test"
                );
                validations.fetch_add(1, Ordering::Relaxed);
                thread::yield_now();
            }
        })
    };

    producer.join().unwrap();
    for h in consumer_handles {
        h.join().unwrap();
    }
    validator.join().unwrap();

    let elapsed = start.elapsed();
    assert!(
        elapsed >= duration,
        "chaos test should run at least {:?}, only ran {:?}",
        duration,
        elapsed
    );
    assert!(writes.load(Ordering::Relaxed) > 10_000, "writes too low");
    assert!(reads.load(Ordering::Relaxed) > 10_000, "reads too low");
    assert!(
        validations.load(Ordering::Relaxed) > 1_000,
        "validator loop should have run many times"
    );
    assert!(
        manual_advances.load(Ordering::Relaxed) > 0,
        "manual chunk advance path was not exercised"
    );
    assert!(
        stale_panics.load(Ordering::Relaxed) > 0,
        "expected at least some stale-read detections under chaos"
    );
}
