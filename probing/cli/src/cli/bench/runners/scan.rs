//! `scan` — sequential read throughput over a populated hot ring.
//!
//! Reads every committed row in logical (oldest→newest) order through the
//! O(1)-per-column cursor, folding values into a sink so the work is not
//! optimised away.

use std::time::Instant;

use anyhow::Result;
use probing_memtable::{DType, MemTable};

use crate::cli::bench::args::ScanArgs;
use crate::cli::bench::metrics::Report;
use super::common::{populate, scan_all};

pub fn run(args: &ScanArgs, json: bool, seed: u64) -> Result<()> {
    let spec = args.schema.spec();
    let mut table = MemTable::new(&spec.schema(), args.ring.chunk_size, args.ring.chunks);
    populate(&mut table, &spec, args.rows, seed);

    let dtypes: Vec<DType> = (0..table.num_cols())
        .map(|i| table.col_dtype(i).expect("known dtype"))
        .collect();

    // Warm pass (also tells us how many rows survived the ring).
    let resident = scan_all(&table, &dtypes);

    let iters = args.iters.max(1);
    let start = Instant::now();
    let mut sink = 0u64;
    for _ in 0..iters {
        sink = sink.wrapping_add(scan_all(&table, &dtypes).0);
    }
    let elapsed = start.elapsed();
    std::hint::black_box(sink);

    let rows_total = resident.1 * iters as u64;
    let bytes_total = rows_total * spec.approx_row_bytes() as u64;

    let mut report = Report::new(format!("scan · {:?}", args.schema.schema));
    report
        .text("schema", format!("{:?}", args.schema.schema))
        .count("rows ingested", args.rows)
        .count("rows resident", resident.1)
        .count("scan passes", iters as u64)
        .duration("elapsed", elapsed)
        .rate("throughput", rows_total, elapsed, "rows")
        .byte_rate("bandwidth", bytes_total, elapsed);
    report.emit(json);
    Ok(())
}
