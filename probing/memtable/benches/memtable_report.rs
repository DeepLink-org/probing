use probing_memtable::{
    CachedReader, DType, MemTable, MemTableView, MemTableWriter, RowCursor, Schema, Value,
};
use std::hint::black_box;
use std::time::{Duration, Instant};

const WARMUP: Duration = Duration::from_millis(300);
const TARGET: Duration = Duration::from_millis(1200);
const FIXED_ROWS: usize = 200_000;
const FIXED_READ_ROWS: usize = 400_000;
const STRING_ROWS: usize = 120_000;
const DEDUP_READ_ROWS: usize = 220_000;

type FixedInput = (i64, i64);
type StringInput = (i64, &'static str, &'static str);

struct BenchResult {
    name: &'static str,
    rows_per_iter: u64,
    bytes_per_iter: u64,
    iterations: u64,
    elapsed: Duration,
}

struct DedupStats {
    plain_payload_bytes: usize,
    dedup_payload_bytes: usize,
    plain_non_empty_chunks: usize,
    dedup_non_empty_chunks: usize,
    total_rows: usize,
}

impl BenchResult {
    fn total_rows(&self) -> u64 {
        self.rows_per_iter.saturating_mul(self.iterations)
    }

    fn total_bytes(&self) -> u64 {
        self.bytes_per_iter.saturating_mul(self.iterations)
    }

    fn ns_per_row(&self) -> f64 {
        self.elapsed.as_secs_f64() * 1e9 / self.total_rows() as f64
    }

    fn rows_per_sec(&self) -> f64 {
        self.total_rows() as f64 / self.elapsed.as_secs_f64()
    }

    fn mrows_per_sec(&self) -> f64 {
        self.rows_per_sec() / 1_000_000.0
    }

    fn mib_per_sec(&self) -> f64 {
        self.total_bytes() as f64 / self.elapsed.as_secs_f64() / (1024.0 * 1024.0)
    }
}

fn run_case<F>(name: &'static str, rows_per_iter: u64, bytes_per_iter: u64, mut f: F) -> BenchResult
where
    F: FnMut() -> u64,
{
    let warmup_start = Instant::now();
    while warmup_start.elapsed() < WARMUP {
        black_box(f());
    }

    let start = Instant::now();
    let mut iterations = 0u64;
    let mut rows = 0u64;
    while start.elapsed() < TARGET {
        rows = rows.saturating_add(black_box(f()));
        iterations += 1;
    }
    let elapsed = start.elapsed();
    let measured_rows = rows_per_iter.saturating_mul(iterations);
    assert_eq!(
        rows, measured_rows,
        "benchmark {name} returned inconsistent row counts"
    );

    BenchResult {
        name,
        rows_per_iter,
        bytes_per_iter,
        iterations,
        elapsed,
    }
}

fn sum_rows(table: &MemTable) -> usize {
    (0..table.num_chunks()).map(|c| table.num_rows(c)).sum()
}

fn fixed_schema() -> Schema {
    Schema::new().col("ts", DType::I64).col("value", DType::I64)
}

fn string_schema() -> Schema {
    Schema::new()
        .col("ts", DType::I64)
        .col("level", DType::Str)
        .col("msg", DType::Str)
}

fn fixed_inputs(rows: usize) -> Vec<FixedInput> {
    (0..rows as i64).map(|i| (i, i * 2)).collect()
}

fn string_inputs(rows: usize) -> Vec<StringInput> {
    let levels = ["INFO", "WARN", "ERROR", "DEBUG"];
    (0..rows as i64)
        .map(|i| {
            let level = levels[i as usize % levels.len()];
            let msg = if i % 2 == 0 {
                "service_started"
            } else {
                "request_finished"
            };
            (i, level, msg)
        })
        .collect()
}

fn encoded_fixed_rows(rows: &[FixedInput]) -> Vec<u8> {
    let mut out = vec![0u8; rows.len() * (4 + 8 + 8)];
    let mut off = 0usize;
    for &(ts, value) in rows {
        out[off..off + 4].copy_from_slice(&16u32.to_le_bytes());
        out[off + 4..off + 12].copy_from_slice(&ts.to_le_bytes());
        out[off + 12..off + 20].copy_from_slice(&value.to_le_bytes());
        off += 20;
    }
    out
}

fn build_fixed_table(rows: &[FixedInput]) -> MemTable {
    let schema = fixed_schema();
    let bytes_per_row = 4 + 8 + 8;
    let chunk_size = 64 * 1024;
    let total_bytes = rows.len() * bytes_per_row;
    let num_chunks = ((total_bytes / chunk_size) + 2).max(2);
    let mut table = MemTable::new(&schema, chunk_size as u32, num_chunks as u32);
    for &(ts, value) in rows {
        table.push_row(&[Value::I64(ts), Value::I64(value)]);
    }
    table
}

fn build_dedup_table(rows: &[StringInput]) -> MemTable {
    let schema = string_schema();
    let avg_payload = 4 + 8 + (4 + 5) + (4 + 20);
    let chunk_size = 64 * 1024;
    let total_bytes = rows.len() * avg_payload;
    let num_chunks = ((total_bytes / chunk_size) + 2).max(2);
    let mut raw = vec![0u8; MemTable::required_size(&schema, chunk_size, num_chunks)];
    {
        let mut writer =
            MemTableWriter::init(&mut raw, &schema, chunk_size as u32, num_chunks as u32).dedup();
        for &(ts, level, msg) in rows {
            writer.push_row(&[Value::I64(ts), Value::Str(level), Value::Str(msg)]);
        }
    }
    MemTable::from_buf(raw).expect("dedup build should produce valid buffer")
}

fn build_string_table(rows: &[StringInput]) -> MemTable {
    let schema = string_schema();
    let approx_bytes_per_row = 4 + 8 + (4 + 5) + (4 + 20);
    let chunk_size = 64 * 1024;
    let total_bytes = rows.len() * approx_bytes_per_row;
    let num_chunks = ((total_bytes / chunk_size) + 2).max(2);
    let mut table = MemTable::new(&schema, chunk_size as u32, num_chunks as u32);
    for &(ts, level, msg) in rows {
        table.push_row(&[Value::I64(ts), Value::Str(level), Value::Str(msg)]);
    }
    table
}

fn bench_push_row_fixed(rows: &[FixedInput]) -> u64 {
    let schema = fixed_schema();
    let bytes_per_row = 4 + 8 + 8;
    let chunk_size = 64 * 1024;
    let total_bytes = rows.len() * bytes_per_row;
    let num_chunks = ((total_bytes / chunk_size) + 2).max(2);
    let mut table = MemTable::new(&schema, chunk_size as u32, num_chunks as u32);
    for &(ts, value) in rows {
        table.push_row(&[Value::I64(ts), Value::I64(value)]);
    }
    black_box(sum_rows(&table));
    rows.len() as u64
}

fn bench_row_writer_fixed(rows: &[FixedInput]) -> u64 {
    let schema = fixed_schema();
    let bytes_per_row = 4 + 8 + 8;
    let chunk_size = 64 * 1024;
    let total_bytes = rows.len() * bytes_per_row;
    let num_chunks = ((total_bytes / chunk_size) + 2).max(2);
    let mut table = MemTable::new(&schema, chunk_size as u32, num_chunks as u32);
    for &(ts, value) in rows {
        table.row_writer().put_i64(ts).put_i64(value).finish();
    }
    black_box(sum_rows(&table));
    rows.len() as u64
}

fn bench_memcpy_fixed(encoded_rows: &[u8], dst: &mut [u8]) -> u64 {
    dst.copy_from_slice(encoded_rows);
    black_box(dst[dst.len() - 1]);
    (encoded_rows.len() / 20) as u64
}

fn bench_flat_encode_fixed(rows: &[FixedInput], dst: &mut [u8]) -> u64 {
    let mut off = 0usize;
    for &(ts, value) in rows {
        dst[off..off + 4].copy_from_slice(&16u32.to_le_bytes());
        dst[off + 4..off + 12].copy_from_slice(&ts.to_le_bytes());
        dst[off + 12..off + 20].copy_from_slice(&value.to_le_bytes());
        off += 20;
    }
    black_box(dst[off - 1]);
    rows.len() as u64
}

fn bench_raw_append_fixed(encoded_rows: &[u8], dst: &mut [u8], chunk_size: usize) -> u64 {
    let row_size = 20usize;
    let mut write_off = 0usize;
    for row in encoded_rows.chunks_exact(row_size) {
        let chunk_off = write_off % chunk_size;
        if chunk_off + row_size > chunk_size {
            write_off += chunk_size - chunk_off;
        }
        dst[write_off..write_off + row_size].copy_from_slice(row);
        write_off += row_size;
    }
    black_box(dst[write_off - 1]);
    (encoded_rows.len() / row_size) as u64
}

fn bench_scan_fixed_cursor(table: &MemTable) -> u64 {
    let rows = sum_rows(&table) as u64;
    let mut checksum = 0i64;
    for chunk in 0..table.num_chunks() {
        for row in table.rows(chunk) {
            let mut c: RowCursor<'_> = row.cursor();
            checksum += c.next_i64();
            checksum += c.next_i64();
        }
    }
    black_box(checksum);
    rows
}

fn bench_push_row_strings(rows: &[StringInput]) -> u64 {
    let schema = string_schema();
    let approx_bytes_per_row = 4 + 8 + (4 + 5) + (4 + 20);
    let chunk_size = 64 * 1024;
    let total_bytes = rows.len() * approx_bytes_per_row;
    let num_chunks = ((total_bytes / chunk_size) + 2).max(2);
    let mut table = MemTable::new(&schema, chunk_size as u32, num_chunks as u32);
    for &(ts, level, msg) in rows {
        table.push_row(&[Value::I64(ts), Value::Str(level), Value::Str(msg)]);
    }
    black_box(sum_rows(&table));
    rows.len() as u64
}

fn bench_push_row_unchecked_fixed(rows: &[FixedInput]) -> u64 {
    let schema = fixed_schema();
    let bytes_per_row = 4 + 8 + 8;
    let chunk_size = 64 * 1024;
    let total_bytes = rows.len() * bytes_per_row;
    let num_chunks = ((total_bytes / chunk_size) + 2).max(2);
    let mut table = MemTable::new(&schema, chunk_size as u32, num_chunks as u32);
    for &(ts, value) in rows {
        table.push_row_unchecked(&[Value::I64(ts), Value::I64(value)]);
    }
    black_box(sum_rows(&table));
    rows.len() as u64
}

fn bench_solo_push_row_fixed(rows: &[FixedInput]) -> u64 {
    let schema = fixed_schema();
    let bytes_per_row = 4 + 8 + 8;
    let chunk_size = 64 * 1024;
    let total_bytes = rows.len() * bytes_per_row;
    let num_chunks = ((total_bytes / chunk_size) + 2).max(2);
    let size = MemTable::required_size(&schema, chunk_size, num_chunks);
    let mut buf = vec![0u8; size];
    let mut sw =
        MemTableWriter::init(&mut buf, &schema, chunk_size as u32, num_chunks as u32).solo();
    for &(ts, value) in rows {
        sw.push_row_unchecked(&[Value::I64(ts), Value::I64(value)]);
    }
    black_box(sw.num_chunks());
    rows.len() as u64
}

fn bench_solo_row_writer_fixed(rows: &[FixedInput]) -> u64 {
    let schema = fixed_schema();
    let bytes_per_row = 4 + 8 + 8;
    let chunk_size = 64 * 1024;
    let total_bytes = rows.len() * bytes_per_row;
    let num_chunks = ((total_bytes / chunk_size) + 2).max(2);
    let size = MemTable::required_size(&schema, chunk_size, num_chunks);
    let mut buf = vec![0u8; size];
    let mut sw =
        MemTableWriter::init(&mut buf, &schema, chunk_size as u32, num_chunks as u32).solo();
    for &(ts, value) in rows {
        sw.row_writer().put_i64(ts).put_i64(value).finish();
    }
    black_box(sw.num_chunks());
    rows.len() as u64
}

fn bench_dedup_push_row_strings(rows: &[StringInput]) -> u64 {
    let schema = string_schema();
    let approx_bytes_per_row = 4 + 8 + (4 + 5) + (4 + 20);
    let chunk_size = 64 * 1024;
    let total_bytes = rows.len() * approx_bytes_per_row;
    let num_chunks = ((total_bytes / chunk_size) + 2).max(2);
    let mut raw = vec![0u8; MemTable::required_size(&schema, chunk_size, num_chunks)];
    let mut writer =
        MemTableWriter::init(&mut raw, &schema, chunk_size as u32, num_chunks as u32).dedup();
    for &(ts, level, msg) in rows {
        writer.push_row(&[Value::I64(ts), Value::Str(level), Value::Str(msg)]);
    }
    black_box(writer.num_chunks());
    rows.len() as u64
}

fn bench_dedup_push_row_strings_min8(rows: &[StringInput]) -> u64 {
    let schema = string_schema();
    let approx_bytes_per_row = 4 + 8 + (4 + 5) + (4 + 20);
    let chunk_size = 64 * 1024;
    let total_bytes = rows.len() * approx_bytes_per_row;
    let num_chunks = ((total_bytes / chunk_size) + 2).max(2);
    let mut raw = vec![0u8; MemTable::required_size(&schema, chunk_size, num_chunks)];
    let mut writer =
        MemTableWriter::init(&mut raw, &schema, chunk_size as u32, num_chunks as u32).dedup();
    writer.set_min_dedup_len(8);
    for &(ts, level, msg) in rows {
        writer.push_row(&[Value::I64(ts), Value::Str(level), Value::Str(msg)]);
    }
    black_box(writer.num_chunks());
    rows.len() as u64
}

fn bench_cached_read_strings(table: &MemTable) -> u64 {
    let view = MemTableView::new(table.as_bytes()).expect("valid table");
    let rows = (0..view.num_chunks())
        .map(|c| view.num_rows(c))
        .sum::<usize>() as u64;
    let mut cache = CachedReader::new(view.as_bytes(), 256);
    let mut checksum = 0i64;
    for chunk in 0..view.num_chunks() {
        for row in view.rows(chunk) {
            let mut c = cache.cursor(&row);
            checksum += c.next_i64();
            checksum += c.next_str().len() as i64;
            checksum += c.next_str().len() as i64;
        }
    }
    black_box(checksum);
    rows
}

fn bench_cached_read_dedup(table: &MemTable) -> u64 {
    bench_cached_read_strings(table)
}

fn payload_used_bytes(table: &MemTable) -> usize {
    (0..table.num_chunks()).map(|c| table.chunk_used(c)).sum()
}

fn non_empty_chunks(table: &MemTable) -> usize {
    (0..table.num_chunks())
        .filter(|&chunk| table.num_rows(chunk) > 0)
        .count()
}

fn dedup_stats(plain: &MemTable, dedup: &MemTable) -> DedupStats {
    DedupStats {
        plain_payload_bytes: payload_used_bytes(plain),
        dedup_payload_bytes: payload_used_bytes(dedup),
        plain_non_empty_chunks: non_empty_chunks(plain),
        dedup_non_empty_chunks: non_empty_chunks(dedup),
        total_rows: sum_rows(plain),
    }
}

fn get_result<'a>(results: &'a [BenchResult], name: &str) -> &'a BenchResult {
    results
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("missing benchmark result: {name}"))
}

fn format_ref(name: Option<&str>) -> String {
    name.unwrap_or("-").to_string()
}

fn format_pct(value: Option<f64>) -> String {
    match value {
        Some(v) => format!("{v:.1}%"),
        None => "-".to_string(),
    }
}

fn print_section(results: &[BenchResult], title: &str, rows: &[(&str, Option<&str>)]) {
    println!("{title}");
    println!(
        "{:<28} {:>10} {:>10} {:>12} {:>12} {:>10} {:>18} {:>9} {:>9}",
        "case", "iters", "rows/iter", "ns/row", "M rows/s", "MiB/s", "ref", "keep", "loss"
    );
    println!("{}", "-".repeat(128));
    for &(name, ref_name) in rows {
        let r = get_result(results, name);
        let reference = ref_name.map(|n| get_result(results, n));
        let keep = reference.map(|base| r.mib_per_sec() / base.mib_per_sec() * 100.0);
        let loss = keep.map(|v| 100.0 - v);
        println!(
            "{:<28} {:>10} {:>10} {:>12.1} {:>12.1} {:>10.1} {:>18} {:>9} {:>9}",
            r.name,
            r.iterations,
            r.rows_per_iter,
            r.ns_per_row(),
            r.mrows_per_sec(),
            r.mib_per_sec(),
            format_ref(ref_name),
            format_pct(keep),
            format_pct(loss),
        );
    }
    println!();
}

fn print_dedup_stats(stats: &DedupStats) {
    let saved_bytes = stats
        .plain_payload_bytes
        .saturating_sub(stats.dedup_payload_bytes);
    let saved_pct = if stats.plain_payload_bytes == 0 {
        0.0
    } else {
        saved_bytes as f64 / stats.plain_payload_bytes as f64 * 100.0
    };
    let plain_rows_per_chunk = stats.total_rows as f64 / stats.plain_non_empty_chunks as f64;
    let dedup_rows_per_chunk = stats.total_rows as f64 / stats.dedup_non_empty_chunks as f64;
    let rows_per_chunk_gain = (dedup_rows_per_chunk / plain_rows_per_chunk - 1.0) * 100.0;
    let plain_bytes_per_row = stats.plain_payload_bytes as f64 / stats.total_rows as f64;
    let dedup_bytes_per_row = stats.dedup_payload_bytes as f64 / stats.total_rows as f64;
    let chunk_delta = stats.dedup_non_empty_chunks as isize - stats.plain_non_empty_chunks as isize;
    let chunk_delta_pct = chunk_delta as f64 / stats.plain_non_empty_chunks as f64 * 100.0;

    println!("dedup efficiency");
    println!(
        "{:<20} {:>14} {:>14} {:>14}",
        "metric", "plain", "dedup", "delta"
    );
    println!("{}", "-".repeat(68));
    println!(
        "{:<20} {:>14.1} {:>14.1} {:>13.1}%",
        "payload MiB",
        stats.plain_payload_bytes as f64 / (1024.0 * 1024.0),
        stats.dedup_payload_bytes as f64 / (1024.0 * 1024.0),
        -saved_pct
    );
    println!(
        "{:<20} {:>14.1} {:>14.1} {:>13.1}%",
        "bytes/row",
        plain_bytes_per_row,
        dedup_bytes_per_row,
        -(plain_bytes_per_row - dedup_bytes_per_row) / plain_bytes_per_row * 100.0
    );
    println!(
        "{:<20} {:>14} {:>14} {:>14}",
        "saved bytes",
        "-",
        "-",
        format!("{saved_bytes} B"),
    );
    println!(
        "{:<20} {:>14} {:>14} {:>13.1}%",
        "rows/nonempty_chunk",
        format!("{plain_rows_per_chunk:.1}"),
        format!("{dedup_rows_per_chunk:.1}"),
        rows_per_chunk_gain
    );
    println!(
        "{:<20} {:>14} {:>14} {:>14}",
        "nonempty_chunks",
        stats.plain_non_empty_chunks,
        stats.dedup_non_empty_chunks,
        format!("{chunk_delta} ({chunk_delta_pct:.1}%)")
    );
    println!();
}

fn print_report(results: &[BenchResult]) {
    println!();
    println!("probing-memtable benchmark report");
    println!();

    print_section(
        results,
        "fixed write ladder",
        &[
            ("baseline_memcpy_fixed", None),
            ("baseline_raw_append", Some("baseline_memcpy_fixed")),
            ("baseline_flat_encode", Some("baseline_raw_append")),
            ("solo_row_writer_fixed", Some("baseline_flat_encode")),
            ("row_writer_fixed", Some("baseline_flat_encode")),
            ("solo_push_row_fixed", Some("solo_row_writer_fixed")),
            ("push_row_unchecked_fixed", Some("row_writer_fixed")),
            ("push_row_fixed", Some("row_writer_fixed")),
        ],
    );

    print_section(
        results,
        "fixed read path",
        &[("scan_fixed_cursor", Some("baseline_memcpy_fixed"))],
    );

    print_section(
        results,
        "string path",
        &[
            ("push_row_strings", None),
            ("dedup_push_row_strings", Some("push_row_strings")),
            ("dedup_min8_strings", Some("push_row_strings")),
            ("cached_read_strings", None),
            ("cached_read_dedup", Some("cached_read_strings")),
        ],
    );

    println!("note: keep/loss are computed from MiB/s against the named ref in each section");
    println!();
}

fn main() {
    let fixed_bytes = (4 + 8 + 8) as u64;
    let string_bytes = (4 + 8 + (4 + 5) + (4 + 20)) as u64;
    let fixed_write_inputs = fixed_inputs(FIXED_ROWS);
    let fixed_read_inputs = fixed_inputs(FIXED_READ_ROWS);
    let string_write_inputs = string_inputs(STRING_ROWS);
    let dedup_read_inputs = string_inputs(DEDUP_READ_ROWS);
    let fixed_encoded_rows = encoded_fixed_rows(&fixed_write_inputs);
    let mut memcpy_scratch = vec![0u8; fixed_encoded_rows.len()];
    let mut flat_encode_scratch = vec![0u8; fixed_encoded_rows.len()];
    let raw_append_chunk_size = 64 * 1024;
    let raw_append_chunks = ((fixed_encoded_rows.len() / raw_append_chunk_size) + 2).max(2);
    let mut raw_append_scratch = vec![0u8; raw_append_chunks * raw_append_chunk_size];
    let fixed_read_table = build_fixed_table(&fixed_read_inputs);
    let plain_string_read_table = build_string_table(&dedup_read_inputs);
    let dedup_read_table = build_dedup_table(&dedup_read_inputs);
    let dedup_report_stats = dedup_stats(&plain_string_read_table, &dedup_read_table);

    let results = vec![
        run_case(
            "baseline_memcpy_fixed",
            FIXED_ROWS as u64,
            FIXED_ROWS as u64 * fixed_bytes,
            || bench_memcpy_fixed(&fixed_encoded_rows, &mut memcpy_scratch),
        ),
        run_case(
            "baseline_flat_encode",
            FIXED_ROWS as u64,
            FIXED_ROWS as u64 * fixed_bytes,
            || bench_flat_encode_fixed(&fixed_write_inputs, &mut flat_encode_scratch),
        ),
        run_case(
            "baseline_raw_append",
            FIXED_ROWS as u64,
            FIXED_ROWS as u64 * fixed_bytes,
            || {
                bench_raw_append_fixed(
                    &fixed_encoded_rows,
                    &mut raw_append_scratch,
                    raw_append_chunk_size,
                )
            },
        ),
        run_case(
            "push_row_fixed",
            FIXED_ROWS as u64,
            FIXED_ROWS as u64 * fixed_bytes,
            || bench_push_row_fixed(&fixed_write_inputs),
        ),
        run_case(
            "push_row_unchecked_fixed",
            FIXED_ROWS as u64,
            FIXED_ROWS as u64 * fixed_bytes,
            || bench_push_row_unchecked_fixed(&fixed_write_inputs),
        ),
        run_case(
            "solo_push_row_fixed",
            FIXED_ROWS as u64,
            FIXED_ROWS as u64 * fixed_bytes,
            || bench_solo_push_row_fixed(&fixed_write_inputs),
        ),
        run_case(
            "solo_row_writer_fixed",
            FIXED_ROWS as u64,
            FIXED_ROWS as u64 * fixed_bytes,
            || bench_solo_row_writer_fixed(&fixed_write_inputs),
        ),
        run_case(
            "row_writer_fixed",
            FIXED_ROWS as u64,
            FIXED_ROWS as u64 * fixed_bytes,
            || bench_row_writer_fixed(&fixed_write_inputs),
        ),
        run_case(
            "scan_fixed_cursor",
            FIXED_READ_ROWS as u64,
            FIXED_READ_ROWS as u64 * fixed_bytes,
            || bench_scan_fixed_cursor(&fixed_read_table),
        ),
        run_case(
            "push_row_strings",
            STRING_ROWS as u64,
            STRING_ROWS as u64 * string_bytes,
            || bench_push_row_strings(&string_write_inputs),
        ),
        run_case(
            "dedup_push_row_strings",
            STRING_ROWS as u64,
            STRING_ROWS as u64 * string_bytes,
            || bench_dedup_push_row_strings(&string_write_inputs),
        ),
        run_case(
            "dedup_min8_strings",
            STRING_ROWS as u64,
            STRING_ROWS as u64 * string_bytes,
            || bench_dedup_push_row_strings_min8(&string_write_inputs),
        ),
        run_case(
            "cached_read_strings",
            DEDUP_READ_ROWS as u64,
            DEDUP_READ_ROWS as u64 * string_bytes,
            || bench_cached_read_strings(&plain_string_read_table),
        ),
        run_case(
            "cached_read_dedup",
            DEDUP_READ_ROWS as u64,
            DEDUP_READ_ROWS as u64 * string_bytes,
            || bench_cached_read_dedup(&dedup_read_table),
        ),
    ];

    print_report(&results);
    print_dedup_stats(&dedup_report_stats);
}
