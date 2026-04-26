//! MEMH v3 performance benchmark.
//!
//! ## What is measured
//!
//! | section  | case                   | what it isolates                              |
//! |----------|------------------------|-----------------------------------------------|
//! | insert   | new-scalar             | hash + probe + PUT_INLINE record + slot       |
//! | insert   | update-scalar-inline   | hash + probe + slot.val_bytes update only     |
//! | insert   | new-str                | hash + probe + PUT record + slot              |
//! | insert   | update-str             | hash + probe + new PUT record per call        |
//! | get      | scalar-hit             | hash + probe + decode from slot               |
//! | get      | str-hit                | hash + probe + decode from arena              |
//! | get      | miss                   | hash + probe chain to EMPTY                   |
//! | iter     | clean                  | linear arena scan, all records live           |
//! | iter     | fragmented             | arena scan with 4× stale records per key      |
//! | baseline | xxh3-64 (10-byte key)  | raw hashing cost reference                    |
//! | baseline | xxh3-64 (32-byte key)  | raw hashing cost reference                    |
//!
//! Note: `insert/update-str` includes a ~2MB memcpy reset at the start of each
//! iteration to prevent arena exhaustion.  Overhead is ≲10% at N=4096.

use std::hint::black_box;
use std::time::{Duration, Instant};

use probing_memtable::{
    memh::{init_buf, view_from_buf, writer_from_buf},
    Value,
};
use xxhash_rust::xxh3;

// ── Parameters ────────────────────────────────────────────

const WARMUP: Duration = Duration::from_millis(300);
const TARGET: Duration = Duration::from_millis(1500);

const N: usize = 4_096;
const NUM_BUCKETS: u32 = 8_192;
const ARENA_CAP: usize = 2 * 1024 * 1024;
const FRAG_UPDATES: usize = 4;

// ── Harness ───────────────────────────────────────────────

struct BenchResult {
    name: &'static str,
    ops_per_iter: u64,
    iterations: u64,
    elapsed: Duration,
}

impl BenchResult {
    fn total_ops(&self) -> u64 {
        self.ops_per_iter * self.iterations
    }
    fn ns_per_op(&self) -> f64 {
        self.elapsed.as_secs_f64() * 1e9 / self.total_ops() as f64
    }
    fn mops_per_sec(&self) -> f64 {
        self.total_ops() as f64 / self.elapsed.as_secs_f64() / 1_000_000.0
    }
}

fn run_case<F>(name: &'static str, ops_per_iter: u64, mut f: F) -> BenchResult
where
    F: FnMut() -> u64,
{
    let warmup_start = Instant::now();
    while warmup_start.elapsed() < WARMUP {
        black_box(f());
    }

    let start = Instant::now();
    let mut iterations = 0u64;
    while start.elapsed() < TARGET {
        black_box(f());
        iterations += 1;
    }
    BenchResult {
        name,
        ops_per_iter,
        iterations,
        elapsed: start.elapsed(),
    }
}

// ── Buffer helpers ────────────────────────────────────────

fn fresh_buf() -> Vec<u8> {
    let size = probing_memtable::memh::layout::required_total_size(NUM_BUCKETS, ARENA_CAP);
    let mut buf = vec![0u8; size];
    init_buf(&mut buf, NUM_BUCKETS, ARENA_CAP, 0).unwrap();
    buf
}

fn populate_scalar(buf: &mut Vec<u8>, keys: &[String]) {
    init_buf(buf, NUM_BUCKETS, ARENA_CAP, 0).unwrap();
    let mut w = writer_from_buf(buf).unwrap();
    for (i, k) in keys.iter().enumerate() {
        w.insert(k, &Value::I64(i as i64)).unwrap();
    }
}

fn populate_str(buf: &mut Vec<u8>, keys: &[String], vals: &[String]) {
    init_buf(buf, NUM_BUCKETS, ARENA_CAP, 0).unwrap();
    let mut w = writer_from_buf(buf).unwrap();
    for (k, v) in keys.iter().zip(vals.iter()) {
        w.insert(k, &Value::Str(v)).unwrap();
    }
}

fn build_fragmented_str_buf(keys: &[String], vals: &[String]) -> Vec<u8> {
    let arena_cap = ARENA_CAP * (FRAG_UPDATES + 1);
    let size = probing_memtable::memh::layout::required_total_size(NUM_BUCKETS, arena_cap);
    let mut buf = vec![0u8; size];
    init_buf(&mut buf, NUM_BUCKETS, arena_cap, 0).unwrap();
    let mut w = writer_from_buf(&mut buf).unwrap();
    for (k, v) in keys.iter().zip(vals.iter()) {
        w.insert(k, &Value::Str(v)).unwrap();
    }
    for round in 0..FRAG_UPDATES {
        for (i, k) in keys.iter().enumerate() {
            let upd = format!("v{}_{i}", round + 1);
            w.insert(k, &Value::Str(&upd)).unwrap();
        }
    }
    buf
}

// ── Benchmark functions ───────────────────────────────────

fn bench_get_scalar_hit(buf: &[u8], keys: &[String]) -> u64 {
    let v = view_from_buf(buf).unwrap();
    let mut sum = 0i64;
    for k in keys {
        if let Some(probing_memtable::memh::TypedValue::I64(n)) = v.get(k) {
            sum = sum.wrapping_add(n);
        }
    }
    black_box(sum);
    keys.len() as u64
}

fn bench_get_str_hit(buf: &[u8], keys: &[String]) -> u64 {
    let v = view_from_buf(buf).unwrap();
    let mut sum = 0usize;
    for k in keys {
        if let Some(probing_memtable::memh::TypedValue::Str(s)) = v.get(k) {
            sum = sum.wrapping_add(s.len());
        }
    }
    black_box(sum);
    keys.len() as u64
}

fn bench_get_miss(buf: &[u8], miss_keys: &[String]) -> u64 {
    let v = view_from_buf(buf).unwrap();
    let mut found = 0usize;
    for k in miss_keys {
        if v.get(k).is_some() {
            found += 1;
        }
    }
    black_box(found);
    miss_keys.len() as u64
}

fn bench_iter(buf: &[u8]) -> u64 {
    let v = view_from_buf(buf).unwrap();
    let mut count = 0u64;
    for (k, val) in v.iter() {
        black_box((k, &val));
        count += 1;
    }
    count
}

// ── Report ────────────────────────────────────────────────

fn get_r<'a>(results: &'a [BenchResult], name: &str) -> &'a BenchResult {
    results
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("missing: {name}"))
}

fn print_section(results: &[BenchResult], title: &str, rows: &[(&str, &str)]) {
    println!("{title}");
    println!(
        "  {:<42} {:>8} {:>12} {:>12}",
        "case", "iters", "ns/op", "M ops/s"
    );
    println!("  {}", "-".repeat(80));
    for &(name, note) in rows {
        let r = get_r(results, name);
        println!(
            "  {:<42} {:>8} {:>12.2} {:>12.2}  {}",
            r.name,
            r.iterations,
            r.ns_per_op(),
            r.mops_per_sec(),
            note
        );
    }
    println!();
}

fn print_report(results: &[BenchResult]) {
    println!();
    println!("═══════════════════════════════════════════════════════════════");
    println!(
        " MEMH v3  N={N}  buckets={NUM_BUCKETS}  arena={}KB",
        ARENA_CAP / 1024
    );
    println!("═══════════════════════════════════════════════════════════════");
    println!();

    print_section(
        results,
        "insert",
        &[
            (
                "insert/new-scalar",
                "fresh table; PUT_INLINE record + slot commit",
            ),
            (
                "insert/update-scalar-inline",
                "same key; update slot.val_bytes, zero arena writes",
            ),
            ("insert/new-str", "fresh table; PUT record + slot commit"),
            (
                "insert/update-str",
                "same key; new PUT record each call  [incl. ~2MB reset/iter]",
            ),
        ],
    );

    print_section(
        results,
        "get",
        &[
            ("get/scalar-hit", "key exists; decode from slot.val_bytes"),
            ("get/str-hit", "key exists; decode from arena record"),
            ("get/miss", "key absent; probe chain to EMPTY"),
        ],
    );

    print_section(
        results,
        "iter",
        &[
            (
                "iter/clean",
                &format!("N={N} live records, no stale records"),
            ),
            (
                "iter/fragmented",
                &format!("N={N} live + {FRAG_UPDATES}×N stale, liveness check overhead"),
            ),
        ],
    );

    print_section(
        results,
        "baseline (hash only)",
        &[
            ("baseline/xxh3-short", "xxh3_64, 10-byte key"),
            ("baseline/xxh3-long", "xxh3_64, 32-byte key"),
        ],
    );

    let su = get_r(results, "insert/update-scalar-inline");
    let stu = get_r(results, "insert/update-str");
    println!(
        "scalar-inline speedup vs str-update: {:.1}×",
        stu.ns_per_op() / su.ns_per_op()
    );
    let ic = get_r(results, "iter/clean");
    let ifr = get_r(results, "iter/fragmented");
    println!(
        "iter fragmentation overhead ({FRAG_UPDATES}× stale records): {:.1}×",
        ifr.ns_per_op() / ic.ns_per_op()
    );
    println!();
}

// ── main ──────────────────────────────────────────────────

fn main() {
    let keys = (0..N).map(|i| format!("key_{i:06}")).collect::<Vec<_>>();
    let miss_keys = (0..N).map(|i| format!("absent_{i:06}")).collect::<Vec<_>>();
    let str_vals = (0..N).map(|i| format!("val_{i:06}")).collect::<Vec<_>>();

    let mut scalar_buf = fresh_buf();
    populate_scalar(&mut scalar_buf, &keys);

    let mut str_buf = fresh_buf();
    populate_str(&mut str_buf, &keys, &str_vals);

    let frag_buf = build_fragmented_str_buf(&keys, &str_vals);

    // Template for str-update reset (memcpy into work_buf resets the arena).
    let str_template = str_buf.clone();
    let mut str_work = str_buf.clone();

    let mut results = Vec::new();

    // ── insert ────────────────────────────────────────────

    results.push(run_case("insert/new-scalar", N as u64, || {
        let mut buf = fresh_buf();
        let mut w = writer_from_buf(&mut buf).unwrap();
        for (i, k) in keys.iter().enumerate() {
            black_box(w.insert(k, &Value::I64(i as i64)).unwrap());
        }
        N as u64
    }));

    // scalar → scalar: zero arena writes, arena never fills.
    {
        let mut upd_buf = scalar_buf.clone();
        let mut w = writer_from_buf(&mut upd_buf).unwrap();
        results.push(run_case("insert/update-scalar-inline", N as u64, || {
            for (i, k) in keys.iter().enumerate() {
                black_box(w.insert(k, &Value::I64((i as i64).wrapping_neg())).unwrap());
            }
            N as u64
        }));
    }

    results.push(run_case("insert/new-str", N as u64, || {
        let mut buf = fresh_buf();
        let mut w = writer_from_buf(&mut buf).unwrap();
        for (k, v) in keys.iter().zip(str_vals.iter()) {
            black_box(w.insert(k, &Value::Str(v)).unwrap());
        }
        N as u64
    }));

    // str → str: appends new record each call; reset buffer to prevent arena exhaustion.
    // The memcpy (~2MB) adds ≲10% overhead vs N str-updates at ~200ns each.
    results.push(run_case("insert/update-str", N as u64, || {
        str_work.copy_from_slice(&str_template);
        let mut w = writer_from_buf(&mut str_work).unwrap();
        for (k, v) in keys.iter().zip(str_vals.iter()) {
            black_box(w.insert(k, &Value::Str(v)).unwrap());
        }
        N as u64
    }));

    // ── get ───────────────────────────────────────────────

    results.push(run_case("get/scalar-hit", N as u64, || {
        bench_get_scalar_hit(&scalar_buf, &keys)
    }));
    results.push(run_case("get/str-hit", N as u64, || {
        bench_get_str_hit(&str_buf, &keys)
    }));
    results.push(run_case("get/miss", N as u64, || {
        bench_get_miss(&scalar_buf, &miss_keys)
    }));

    // ── iter ──────────────────────────────────────────────

    results.push(run_case("iter/clean", N as u64, || bench_iter(&str_buf)));
    results.push(run_case("iter/fragmented", N as u64, || {
        bench_iter(&frag_buf)
    }));

    // ── baseline ──────────────────────────────────────────

    results.push(run_case("baseline/xxh3-short", N as u64, || {
        let mut sum = 0u64;
        for i in 0..N as u64 {
            sum = sum.wrapping_add(xxh3::xxh3_64_with_seed(black_box(b"key_000042"), i));
        }
        black_box(sum);
        N as u64
    }));
    results.push(run_case("baseline/xxh3-long", N as u64, || {
        let mut sum = 0u64;
        for i in 0..N as u64 {
            sum = sum.wrapping_add(xxh3::xxh3_64_with_seed(
                black_box(b"key_000042_some_longer_prefix___"),
                i,
            ));
        }
        black_box(sum);
        N as u64
    }));

    print_report(&results);
}
