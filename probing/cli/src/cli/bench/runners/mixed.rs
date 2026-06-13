//! `mixed` — end-to-end pipeline / soak: a single writer, optional background
//! compactor, and concurrent readers over one shared table for a fixed
//! duration. Reports per-role throughput plus the resulting cold-tier
//! footprint. MEMT is single-writer, so the writer count is fixed at 1.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use probing_memtable::memc::{ColdStore, Compactor, CompactorConfig};
use probing_memtable::{DType, MemTable};

use crate::cli::bench::args::{Backend, MixedArgs};
use crate::cli::bench::metrics::Report;
use super::common::{scan_all, shm_name, temp_dir, temp_path, unique_token, Attach};
use crate::cli::bench::workload::RowGen;

pub fn run(args: &MixedArgs, json: bool, seed: u64) -> Result<()> {
    let spec = args.schema.spec();
    let row_bytes = spec.approx_row_bytes() as u64;
    if args.writers > 1 {
        bail!("mixed is single-writer (MEMT); --writers must be 1");
    }
    let writers = 1usize;
    let readers = args.readers;

    // Create the shared backing; keep the creator alive for the whole run.
    let mut cleanup_file: Option<std::path::PathBuf> = None;
    let (attach, _creator) = match args.backend {
        Backend::Heap => bail!("mixed requires a shared backend (shm/file/shared), not heap"),
        Backend::Shm => {
            let name = shm_name();
            let creator =
                MemTable::shm(&name, &spec.schema(), args.ring.chunk_size, args.ring.chunks)?;
            (Attach::Shm(name), creator)
        }
        Backend::File => {
            let path = temp_path("mixed");
            cleanup_file = Some(path.clone());
            let creator =
                MemTable::file_at(&path, &spec.schema(), args.ring.chunk_size, args.ring.chunks)?;
            (Attach::File(path), creator)
        }
        Backend::Shared => {
            let name = format!("bench-{}", unique_token());
            let creator =
                MemTable::shared(&name, &spec.schema(), args.ring.chunk_size, args.ring.chunks)?;
            let path = creator.path().expect("shared path").to_path_buf();
            (Attach::File(path), creator)
        }
    };

    let dtypes: Vec<DType> = (0..spec.schema().cols.len())
        .map(|i| spec.schema().cols[i].dtype)
        .collect();

    let stop = Arc::new(AtomicBool::new(false));
    let write_rows = Arc::new(AtomicU64::new(0));
    let read_rows = Arc::new(AtomicU64::new(0));
    let read_passes = Arc::new(AtomicU64::new(0));

    // Background compactor (own read handle to the shared mapping).
    let cold_dir = temp_dir("mixed-cold")?;
    let compactor_handle = if args.no_compact {
        None
    } else {
        let store = ColdStore::open(&cold_dir)?;
        let config = CompactorConfig {
            target_segment_bytes: args.target_mb * 1024 * 1024,
            max_segment_age: Duration::from_secs(args.duration.max(1)),
            poll_interval: Duration::from_millis(50),
            max_total_bytes: args.max_total_mb.map(|m| m * 1024 * 1024),
            ttl: args.ttl_secs.map(Duration::from_secs),
        };
        let handle = attach.open()?;
        Some(Compactor::new(store, config).spawn(vec![("bench".to_string(), handle)]))
    };

    let mut threads = Vec::new();

    for tid in 0..writers {
        let attach = attach.clone();
        let spec = spec.clone();
        let stop = stop.clone();
        let write_rows = write_rows.clone();
        let seed = seed ^ (0x9E37_79B9_u64.wrapping_mul(tid as u64 + 1));
        threads.push(std::thread::spawn(move || -> Result<()> {
            let mut table = attach.open()?;
            let mut gen = RowGen::new(spec.clone(), seed, (tid as i64) * 1_000_000_000);
            let mut scratch: Vec<f64> = Vec::new();
            let mut local = 0u64;
            while !stop.load(Ordering::Relaxed) {
                for _ in 0..256 {
                    let values = gen.values(&mut scratch);
                    table.push_row_unchecked(&values);
                }
                local += 256;
            }
            write_rows.fetch_add(local, Ordering::Relaxed);
            Ok(())
        }));
    }

    for _ in 0..readers {
        let attach = attach.clone();
        let stop = stop.clone();
        let read_rows = read_rows.clone();
        let read_passes = read_passes.clone();
        let dtypes = dtypes.clone();
        threads.push(std::thread::spawn(move || -> Result<()> {
            let table = attach.open()?;
            let mut rows = 0u64;
            let mut passes = 0u64;
            let mut sink = 0u64;
            while !stop.load(Ordering::Relaxed) {
                let (s, n) = scan_all(&table, &dtypes);
                sink = sink.wrapping_add(s);
                rows += n;
                passes += 1;
            }
            std::hint::black_box(sink);
            read_rows.fetch_add(rows, Ordering::Relaxed);
            read_passes.fetch_add(passes, Ordering::Relaxed);
            Ok(())
        }));
    }

    let start = Instant::now();
    std::thread::sleep(Duration::from_secs(args.duration.max(1)));
    stop.store(true, Ordering::Relaxed);
    for t in threads {
        t.join().unwrap()?;
    }
    let elapsed = start.elapsed();

    if let Some(h) = compactor_handle {
        h.stop();
    }

    let total_writes = write_rows.load(Ordering::Relaxed);
    let total_reads = read_rows.load(Ordering::Relaxed);
    let passes = read_passes.load(Ordering::Relaxed);

    let cold = if args.no_compact {
        None
    } else {
        ColdStore::open(&cold_dir).ok().map(|s| s.stats())
    };

    let mut report = Report::new(format!("mixed · {:?} · {:?}", args.backend, args.schema.schema));
    report
        .text("backend", format!("{:?}", args.backend))
        .text("schema", format!("{:?}", args.schema.schema))
        .count("writers", writers as u64)
        .count("readers", readers as u64)
        .text("compactor", if args.no_compact { "off" } else { "on" })
        .duration("duration", elapsed)
        .count("rows written", total_writes)
        .rate("write rate", total_writes, elapsed, "rows")
        .byte_rate("write bw", total_writes * row_bytes, elapsed);
    if readers > 0 {
        report
            .count("scan passes", passes)
            .count("rows scanned", total_reads)
            .rate("read rate", total_reads, elapsed, "rows");
    }
    if let Some(c) = cold {
        let logical = total_writes * row_bytes;
        let ratio = if c.total_bytes > 0 {
            logical as f64 / c.total_bytes as f64
        } else {
            0.0
        };
        report
            .count("cold segments", c.segment_count as u64)
            .bytes("cold on-disk", c.total_bytes)
            .ratio("compression*", ratio);
    }
    report.emit(json);

    if let Some(p) = cleanup_file {
        let _ = std::fs::remove_file(p);
    }
    let _ = std::fs::remove_dir_all(&cold_dir);
    Ok(())
}
