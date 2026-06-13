//! `compact` — cold-tier roller throughput and hot→cold compression ratio.
//!
//! Ingest is interleaved with drain passes so sealed chunks are compacted
//! before the ring can recycle them; we time the drain work separately from
//! the end-to-end wall clock.

use std::time::{Duration, Instant};

use anyhow::Result;
use probing_memtable::memc::{ColdStore, Compactor, CompactorConfig};
use probing_memtable::MemTable;

use crate::cli::bench::args::CompactArgs;
use crate::cli::bench::metrics::Report;
use crate::cli::bench::workload::RowGen;

pub fn run(args: &CompactArgs, json: bool, seed: u64) -> Result<()> {
    let spec = args.schema.spec();
    let row_bytes = spec.approx_row_bytes() as u64;

    let dir = match &args.dir {
        Some(d) => {
            std::fs::create_dir_all(d)?;
            d.clone()
        }
        None => super::common::temp_dir("compact")?,
    };

    let mut table = MemTable::new(&spec.schema(), args.ring.chunk_size, args.ring.chunks);
    let store = ColdStore::open(&dir)?;
    let config = CompactorConfig {
        target_segment_bytes: args.target_mb * 1024 * 1024,
        max_segment_age: Duration::from_secs(3600),
        poll_interval: Duration::from_millis(1),
        max_total_bytes: None,
        ttl: None,
    };
    let mut compactor = Compactor::new(store, config);

    // Drain every ~half-ring worth of rows so undrained sealed chunks never
    // exceed ring capacity.
    let rows_per_chunk =
        ((args.ring.chunk_size as u64).saturating_sub(40)) / (row_bytes + 4).max(1);
    let batch = (rows_per_chunk * (args.ring.chunks as u64 / 2).max(1)).max(1);

    let mut gen = RowGen::new(spec.clone(), seed, 0);
    let mut scratch: Vec<f64> = Vec::new();
    let name = "bench";

    let mut ingested = 0u64;
    let mut drained = 0u64;
    let mut drain_time = Duration::ZERO;
    let wall = Instant::now();

    while ingested < args.rows {
        let n = batch.min(args.rows - ingested);
        for _ in 0..n {
            let values = gen.values(&mut scratch);
            table.push_row_unchecked(&values);
        }
        ingested += n;

        let t = Instant::now();
        drained += compactor.drain_view(name, &table.view())? as u64;
        drain_time += t.elapsed();
    }

    // Final drains (sealed-but-not-yet-drained chunks) + seal the tail.
    loop {
        let t = Instant::now();
        let n = compactor.drain_view(name, &table.view())? as u64;
        drain_time += t.elapsed();
        drained += n;
        if n == 0 {
            break;
        }
    }
    let t = Instant::now();
    compactor.flush()?;
    drain_time += t.elapsed();
    let wall = wall.elapsed();

    let stats = compactor.stats();
    let logical = drained * row_bytes;
    let ratio = if stats.total_bytes > 0 {
        logical as f64 / stats.total_bytes as f64
    } else {
        0.0
    };

    let mut report = Report::new(format!("compact · {:?}", args.schema.schema));
    report
        .text("schema", format!("{:?}", args.schema.schema))
        .count("rows ingested", ingested)
        .count("rows drained", drained)
        .count("cold segments", stats.segment_count as u64)
        .bytes("hot logical", logical)
        .bytes("cold on-disk", stats.total_bytes)
        .ratio("compression", ratio)
        .duration("drain time", drain_time)
        .duration("wall time", wall)
        .rate("compact rate", drained, drain_time, "rows")
        .byte_rate("cold write rate", stats.total_bytes, drain_time);
    report.emit(json);

    if args.dir.is_none() && !args.keep {
        let _ = std::fs::remove_dir_all(&dir);
    } else {
        report_dir(&dir, json);
    }
    Ok(())
}

fn report_dir(dir: &std::path::Path, json: bool) {
    if !json {
        println!("  cold dir: {}", dir.display());
    }
}
