//! `coldscan` — read + decode throughput over MEMC cold segments.
//!
//! Opens every `.memc` segment in a cold directory and decodes every page
//! (Pco / raw), folding row counts into a sink. Reports both the logical
//! (decoded) throughput and the on-disk (compressed) read rate.

use std::time::Instant;

use anyhow::{bail, Context, Result};
use probing_memtable::memc::{ColdStore, SegmentReader};

use crate::cli::bench::args::ColdscanArgs;
use crate::cli::bench::metrics::Report;

pub fn run(args: &ColdscanArgs, json: bool, seed: u64) -> Result<()> {
    let spec = args.schema.spec();
    let row_bytes = spec.approx_row_bytes() as u64;

    let (dir, built, temp) = match &args.dir {
        Some(d) => (d.clone(), 0u64, false),
        None => {
            let dir = super::common::temp_dir("coldscan")?;
            let drained = super::common::build_cold(
                &dir,
                &spec,
                &args.ring,
                args.rows,
                args.target_mb,
                seed,
            )?;
            (dir, drained, true)
        }
    };

    let store = ColdStore::open(&dir)?;
    let segments = store.segment_paths();
    if segments.is_empty() {
        bail!("no .memc segments found under {}", dir.display());
    }

    let iters = args.iters.max(1);
    let mut rows_per_pass = 0u64;
    let mut disk_per_pass = 0u64;
    let mut sink = 0u64;

    let start = Instant::now();
    for _ in 0..iters {
        let mut rows = 0u64;
        let mut disk = 0u64;
        for path in &segments {
            let reader =
                SegmentReader::open(path).with_context(|| format!("open {}", path.display()))?;
            for (i, page) in reader.pages().iter().enumerate() {
                disk += page.block_len as u64;
                let cols = reader
                    .read_page(i)
                    .map_err(|e| anyhow::anyhow!("decode page {i}: {e}"))?;
                let n = cols.first().map(|c| c.len()).unwrap_or(0) as u64;
                rows += n;
                sink = sink.wrapping_add(n);
            }
        }
        rows_per_pass = rows;
        disk_per_pass = disk;
    }
    let elapsed = start.elapsed();
    std::hint::black_box(sink);

    let rows_total = rows_per_pass * iters as u64;
    let disk_total = disk_per_pass * iters as u64;
    let logical_total = rows_total * row_bytes;

    let mut report = Report::new(format!("coldscan · {:?}", args.schema.schema));
    report.text("schema", format!("{:?}", args.schema.schema));
    if built > 0 {
        report.count("rows built", built);
    }
    report
        .count("segments", segments.len() as u64)
        .count("rows/pass", rows_per_pass)
        .count("read passes", iters as u64)
        .duration("elapsed", elapsed)
        .rate("decode rate", rows_total, elapsed, "rows")
        .byte_rate("logical rate", logical_total, elapsed)
        .byte_rate("on-disk read", disk_total, elapsed);
    report.emit(json);

    if temp {
        let _ = std::fs::remove_dir_all(&dir);
    }
    Ok(())
}
