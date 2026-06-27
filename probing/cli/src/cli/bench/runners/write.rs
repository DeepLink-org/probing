//! `write` — write throughput across backends, writer counts and APIs.
//!
//! MEMT is single-writer, so shared backends (`shm`/`file`/`shared`) run with
//! one writer. `--threads > 1` is only valid on the `heap` backend, where each
//! thread gets its own independent table (parallel throughput, one writer each).

use std::sync::Barrier;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use probing_memtable::MemTable;

use super::common;
use crate::cli::bench::args::{Backend, RingArgs, WriteArgs, WriterMode};
use crate::cli::bench::metrics::{Latency, Report};
use crate::cli::bench::workload::{RowGen, WorkloadSpec};

/// How a worker thread obtains its table handle.
enum Source {
    Heap,
    Shm(String),
    File(std::path::PathBuf),
}

struct WorkerOut {
    rows: u64,
    bytes: u64,
    latency: Option<Latency>,
}

pub fn run(args: &WriteArgs, json: bool, seed: u64) -> Result<()> {
    let spec = args.schema.spec();
    let threads = args.threads.max(1);

    if args.writer == WriterMode::Streaming && threads > 1 {
        bail!(
            "--writer streaming requires --threads 1 (advance-on-overflow is not concurrency-safe)"
        );
    }
    // MEMT is single-writer. Multiple threads writing the SAME mapping is
    // unsupported, so shared backends are capped to one writer. The heap
    // backend instead gives each thread its own independent table.
    if threads > 1 && args.backend != Backend::Heap {
        bail!(
            "--threads > 1 requires --backend heap (independent per-thread tables); \
             shared backends (shm/file/shared) are single-writer"
        );
    }
    if threads > 1 && args.backend == Backend::Heap {
        eprintln!(
            "note: heap backend cannot be shared; --threads {threads} uses independent \
             per-thread tables (parallel, single-writer each)"
        );
    }

    // Set up the backing for shared backends; keep the creator handle alive
    // for the whole run so attached worker handles stay valid.
    let mut cleanup_file: Option<std::path::PathBuf> = None;
    let (source, _creator) = match args.backend {
        Backend::Heap => (Source::Heap, None),
        Backend::Shm => {
            let name = common::shm_name();
            let creator = MemTable::shm(
                &name,
                &spec.schema(),
                args.ring.chunk_size,
                args.ring.chunks,
            )?;
            (Source::Shm(name), Some(creator))
        }
        Backend::File => {
            let path = args
                .path
                .clone()
                .unwrap_or_else(|| common::temp_path("write"));
            if args.path.is_none() {
                cleanup_file = Some(path.clone());
            }
            let creator = MemTable::file_at(
                &path,
                &spec.schema(),
                args.ring.chunk_size,
                args.ring.chunks,
            )?;
            (Source::File(path), Some(creator))
        }
        Backend::Shared => {
            let name = format!("bench-{}", common::unique_token());
            let creator = MemTable::shared(
                &name,
                &spec.schema(),
                args.ring.chunk_size,
                args.ring.chunks,
            )?;
            let path = creator
                .path()
                .context("shared memtable has no file path")?
                .to_path_buf();
            (Source::File(path), Some(creator))
        }
    };

    let per_thread = args.rows / threads as u64;
    let remainder = args.rows % threads as u64;
    let barrier = Barrier::new(threads + 1);
    let lat_cap = if args.latency { 1 << 16 } else { 0 };

    let (outs, elapsed) = std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(threads);
        for tid in 0..threads {
            let rows = per_thread + if (tid as u64) < remainder { 1 } else { 0 };
            let spec = spec.clone();
            let source = &source;
            let barrier = &barrier;
            let ring = args.ring.clone();
            let writer = args.writer;
            let warmup = args.warmup;
            handles.push(scope.spawn(move || -> Result<WorkerOut> {
                let mut table = open_handle(source, &spec, &ring)?;
                let seed = seed ^ (0x9E37_79B9_u64.wrapping_mul(tid as u64 + 1));
                // Distinct time windows per writer.
                let start_ts = (tid as i64) * 1_000_000_000;
                let mut gen = RowGen::new(spec.clone(), seed, start_ts);

                run_rows(&mut table, &mut gen, writer, warmup, &mut None);

                barrier.wait();
                let mut lat = (lat_cap > 0).then(|| Latency::new(lat_cap));
                let written = run_rows(&mut table, &mut gen, writer, rows, &mut lat);
                Ok(WorkerOut {
                    rows: written,
                    bytes: written * spec.approx_row_bytes() as u64,
                    latency: lat,
                })
            }));
        }

        // Release the workers together, then time the full write window.
        barrier.wait();
        let start = Instant::now();
        let outs: Vec<Result<WorkerOut>> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        (outs, start.elapsed())
    });

    let mut total_rows = 0u64;
    let mut total_bytes = 0u64;
    let mut merged = Latency::new(lat_cap.max(1));
    for o in outs {
        let o = o?;
        total_rows += o.rows;
        total_bytes += o.bytes;
        if let Some(l) = o.latency {
            merged.merge(&l);
        }
    }

    if let Some(p) = cleanup_file {
        let _ = std::fs::remove_file(p);
    }

    let mut report = Report::new(format!(
        "write · {:?} · {:?}",
        args.backend, args.schema.schema
    ));
    report
        .text("backend", format!("{:?}", args.backend))
        .text("schema", format!("{:?}", args.schema.schema))
        .text("writer", format!("{:?}", args.writer))
        .count("threads", threads as u64)
        .count("rows", total_rows)
        .duration("elapsed", elapsed)
        .rate("throughput", total_rows, elapsed, "rows")
        .byte_rate("bandwidth", total_bytes, elapsed)
        .rate("per-thread", total_rows / threads as u64, elapsed, "rows");
    if args.latency {
        report.latency("latency", &merged);
    }
    report.emit(json);
    Ok(())
}

fn open_handle(source: &Source, spec: &WorkloadSpec, ring: &RingArgs) -> Result<MemTable> {
    Ok(match source {
        Source::Heap => MemTable::new(&spec.schema(), ring.chunk_size, ring.chunks),
        Source::Shm(name) => MemTable::open_shm(name)?,
        Source::File(path) => MemTable::open_file(path)?,
    })
}

/// Write `rows` rows, optionally recording per-row latency. Returns rows written.
fn run_rows(
    table: &mut MemTable,
    gen: &mut RowGen,
    mode: WriterMode,
    rows: u64,
    lat: &mut Option<Latency>,
) -> u64 {
    let mut scratch: Vec<f64> = Vec::new();
    for _ in 0..rows {
        let t = lat.as_ref().map(|_| Instant::now());
        match mode {
            WriterMode::Push => {
                let values = gen.values(&mut scratch);
                table.push_row_unchecked(&values);
            }
            WriterMode::Streaming => {
                let ok = {
                    let mut w = table.row_writer();
                    gen.write_into(&mut w)
                };
                if !ok {
                    table.advance_chunk();
                    let mut w = table.row_writer();
                    let _ = gen.write_into(&mut w);
                }
            }
        }
        if let (Some(l), Some(t)) = (lat.as_mut(), t) {
            l.record(t.elapsed().as_nanos() as u64);
        }
    }
    rows
}
