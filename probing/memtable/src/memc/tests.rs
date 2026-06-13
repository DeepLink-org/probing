//! End-to-end tests for the MEMC cold segment format and store.

use super::*;
use crate::schema::{DType, Schema, Value};
use crate::MemTable;
use std::time::Duration;

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "memc-test-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn metrics_cols() -> Vec<(String, DType)> {
    vec![
        ("timestamp".to_string(), DType::I64),
        ("value".to_string(), DType::F64),
        ("tag".to_string(), DType::Str),
    ]
}

#[test]
fn segment_roundtrip_sealed() {
    let dir = tmp_dir("roundtrip");
    let path = dir.join("seg.memc");

    let mut w = SegmentWriter::create(&path).unwrap();
    let tid = w.register_table("metrics", &metrics_cols()).unwrap();
    w.append_page(
        tid,
        &[
            ColumnData::I64(vec![100, 200, 300]),
            ColumnData::F64(vec![1.0, 2.0, 3.0]),
            ColumnData::Str(vec!["a".into(), "b".into(), "c".into()]),
        ],
        7,
        0,
    )
    .unwrap();
    w.append_page(
        tid,
        &[
            ColumnData::I64(vec![400, 500]),
            ColumnData::F64(vec![4.0, 5.0]),
            ColumnData::Str(vec!["d".into(), "e".into()]),
        ],
        8,
        1,
    )
    .unwrap();
    w.seal().unwrap();

    let r = SegmentReader::open(&path).unwrap();
    assert!(r.is_sealed());
    assert_eq!(r.ts_range(), Some((100, 500)));
    assert_eq!(r.pages().len(), 2);

    let id = r.table_id_by_name("metrics").unwrap();
    let def = r.table_def(id).unwrap();
    assert_eq!(def.cols.len(), 3);
    assert_eq!(def.ts_col, Some(0));

    let cols = r.read_page(0).unwrap();
    assert_eq!(cols[0], ColumnData::I64(vec![100, 200, 300]));
    assert_eq!(cols[2], ColumnData::Str(vec!["a".into(), "b".into(), "c".into()]));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn size_bytes_tracks_growth_for_roll_decisions() {
    let dir = tmp_dir("sizehint");
    let path = dir.join("seg.memc");

    let mut w = SegmentWriter::create(&path).unwrap();
    let base = w.size_bytes();
    assert_eq!(base, 64, "starts at the 64-byte header");
    assert_eq!(w.ts_span(), None);

    let tid = w
        .register_table("m", &[("timestamp".to_string(), DType::I64)])
        .unwrap();
    let after_reg = w.size_bytes();
    assert!(after_reg > base, "table block advances the offset");

    w.append_page(tid, &[ColumnData::I64(vec![10, 20, 30])], 0, 0)
        .unwrap();
    assert!(w.size_bytes() > after_reg, "page advances the offset");
    assert_eq!(w.ts_span(), Some((10, 30)));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn multi_table_segment() {
    let dir = tmp_dir("multitable");
    let path = dir.join("seg.memc");

    let mut w = SegmentWriter::create(&path).unwrap();
    let metrics = w.register_table("metrics", &metrics_cols()).unwrap();
    let events = w
        .register_table(
            "events",
            &[("ts".to_string(), DType::I64), ("code".to_string(), DType::I32)],
        )
        .unwrap();

    w.append_page(
        metrics,
        &[
            ColumnData::I64(vec![10, 20]),
            ColumnData::F64(vec![0.1, 0.2]),
            ColumnData::Str(vec!["x".into(), "y".into()]),
        ],
        1,
        0,
    )
    .unwrap();
    w.append_page(
        events,
        &[ColumnData::I64(vec![15]), ColumnData::I32(vec![42])],
        1,
        0,
    )
    .unwrap();
    w.seal().unwrap();

    let r = SegmentReader::open(&path).unwrap();
    let mpages = r.pages_in_range(metrics, None, None);
    let epages = r.pages_in_range(events, None, None);
    assert_eq!(mpages.len(), 1);
    assert_eq!(epages.len(), 1);
    assert_eq!(r.read_page(epages[0]).unwrap()[1], ColumnData::I32(vec![42]));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn page_pruning_by_time_range() {
    let dir = tmp_dir("prune");
    let path = dir.join("seg.memc");

    let mut w = SegmentWriter::create(&path).unwrap();
    let tid = w.register_table("m", &[("timestamp".to_string(), DType::I64)]).unwrap();
    w.append_page(tid, &[ColumnData::I64(vec![0, 10, 20])], 0, 0)
        .unwrap();
    w.append_page(tid, &[ColumnData::I64(vec![100, 110, 120])], 0, 1)
        .unwrap();
    w.append_page(tid, &[ColumnData::I64(vec![200, 210])], 0, 2)
        .unwrap();
    w.seal().unwrap();

    let r = SegmentReader::open(&path).unwrap();
    // Window [105, 130] overlaps only the middle page.
    let hit = r.pages_in_range(tid, Some(105), Some(130));
    assert_eq!(hit.len(), 1);
    assert_eq!(r.read_page(hit[0]).unwrap()[0], ColumnData::I64(vec![100, 110, 120]));

    // Lower bound past everything → no pages.
    assert!(r.pages_in_range(tid, Some(1000), None).is_empty());
    // Unbounded → all three.
    assert_eq!(r.pages_in_range(tid, None, None).len(), 3);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unsealed_segment_recovers_via_forward_scan() {
    let dir = tmp_dir("unsealed");
    let path = dir.join("seg.memc");

    {
        let mut w = SegmentWriter::create(&path).unwrap();
        let tid = w.register_table("m", &[("timestamp".to_string(), DType::I64)]).unwrap();
        w.append_page(tid, &[ColumnData::I64(vec![1, 2, 3])], 0, 0)
            .unwrap();
        w.append_page(tid, &[ColumnData::I64(vec![4, 5, 6])], 0, 1)
            .unwrap();
        // Drop WITHOUT seal — simulates a crash before footer is written.
    }

    let r = SegmentReader::open(&path).unwrap();
    assert!(!r.is_sealed());
    assert_eq!(r.pages().len(), 2, "forward scan must recover both pages");
    let id = r.table_id_by_name("m").unwrap();
    assert_eq!(r.read_page(0).unwrap()[0], ColumnData::I64(vec![1, 2, 3]));
    assert_eq!(r.pages_in_range(id, None, None).len(), 2);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn torn_tail_block_is_dropped() {
    let dir = tmp_dir("torn");
    let path = dir.join("seg.memc");

    {
        let mut w = SegmentWriter::create(&path).unwrap();
        let tid = w.register_table("m", &[("timestamp".to_string(), DType::I64)]).unwrap();
        w.append_page(tid, &[ColumnData::I64(vec![1, 2, 3])], 0, 0)
            .unwrap();
        w.append_page(tid, &[ColumnData::I64(vec![4, 5, 6])], 0, 1)
            .unwrap();
    }
    // Find the second page's block, then truncate into the middle of its
    // payload (header intact) to mimic a partial write.
    let cut = {
        let r = SegmentReader::open(&path).unwrap();
        let p1 = &r.pages()[1];
        p1.block_off + (super::layout::BLOCK_HEADER_SIZE as u64) + 8
    };
    let f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    f.set_len(cut).unwrap();
    drop(f);

    let r = SegmentReader::open(&path).unwrap();
    assert_eq!(
        r.pages().len(),
        1,
        "torn tail page must be dropped, first page survives"
    );
    assert_eq!(r.read_page(0).unwrap()[0], ColumnData::I64(vec![1, 2, 3]));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cold_store_segment_creation_and_listing() {
    let dir = tmp_dir("store-create");
    let mut store = ColdStore::open(&dir).unwrap();

    for batch in 0..3 {
        let mut w = store.create_segment().unwrap();
        let tid = w.register_table("m", &[("timestamp".to_string(), DType::I64)]).unwrap();
        w.append_page(tid, &[ColumnData::I64(vec![batch, batch + 1])], 0, 0)
            .unwrap();
        w.seal().unwrap();
    }

    let segs = store.segment_paths();
    assert_eq!(segs.len(), 3);
    let stats = store.stats();
    assert_eq!(stats.segment_count, 3);
    assert!(stats.total_bytes > 0);

    // A fresh store over the same dir continues the sequence.
    let mut store2 = ColdStore::open(&dir).unwrap();
    let next = store2.next_segment_path();
    assert!(next.to_string_lossy().contains("-000004."));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn eviction_respects_byte_budget_and_keeps_newest() {
    let dir = tmp_dir("evict-bytes");
    let mut store = ColdStore::open(&dir).unwrap();

    let mut sizes = Vec::new();
    for i in 0..5i64 {
        let mut w = store.create_segment().unwrap();
        let tid = w.register_table("m", &[("timestamp".to_string(), DType::I64)]).unwrap();
        w.append_page(
            tid,
            &[ColumnData::I64((0..100).map(|x| x + i * 1000).collect())],
            0,
            0,
        )
        .unwrap();
        let path = w.seal().unwrap();
        sizes.push(std::fs::metadata(&path).unwrap().len());
        // Ensure distinct mtimes for deterministic oldest-first ordering.
        std::thread::sleep(Duration::from_millis(10));
    }

    let total: u64 = sizes.iter().sum();
    // Budget that should force dropping the oldest couple of segments.
    let budget = total - sizes[0] - sizes[1] + 1;
    let removed = store.enforce_limits(Some(budget), None);
    assert!(!removed.is_empty(), "expected some eviction");

    let remaining = store.segment_paths();
    assert!(remaining.len() < 5);
    assert!(store.stats().total_bytes <= budget);
    // Newest survives.
    assert!(remaining
        .last()
        .unwrap()
        .to_string_lossy()
        .contains("-000005."));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn eviction_by_ttl() {
    let dir = tmp_dir("evict-ttl");
    let mut store = ColdStore::open(&dir).unwrap();
    for _ in 0..3 {
        let mut w = store.create_segment().unwrap();
        let tid = w.register_table("m", &[("timestamp".to_string(), DType::I64)]).unwrap();
        w.append_page(tid, &[ColumnData::I64(vec![1, 2])], 0, 0)
            .unwrap();
        w.seal().unwrap();
        std::thread::sleep(Duration::from_millis(10));
    }
    // TTL of 0 → every segment except the protected newest is expired.
    let removed = store.enforce_limits(None, Some(Duration::from_millis(0)));
    assert_eq!(removed.len(), 2);
    assert_eq!(store.segment_paths().len(), 1);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pco_compresses_large_numeric_segment() {
    let dir = tmp_dir("compress");
    let path = dir.join("seg.memc");

    let n = 50_000i64;
    let mut w = SegmentWriter::create(&path).unwrap();
    let tid = w
        .register_table(
            "metrics",
            &[("timestamp".to_string(), DType::I64), ("value".to_string(), DType::F64)],
        )
        .unwrap();
    w.append_page(
        tid,
        &[
            ColumnData::I64((0..n).map(|i| 1_700_000_000_000 + i * 1000).collect()),
            ColumnData::F64((0..n).map(|i| (i as f64) * 0.5).collect()),
        ],
        0,
        0,
    )
    .unwrap();
    let sealed = w.seal().unwrap();

    let on_disk = std::fs::metadata(&sealed).unwrap().len();
    let raw = (n as u64) * (8 + 8);
    assert!(
        on_disk < raw / 3,
        "expected >3x compression: {on_disk} vs {raw}"
    );

    // And it still reads back exactly.
    let r = SegmentReader::open(&sealed).unwrap();
    let cols = r.read_page(0).unwrap();
    assert_eq!(cols[0].len(), n as usize);

    let _ = std::fs::remove_dir_all(&dir);
}

// ── Compactor (the roller) ───────────────────────────────────────────

fn hot_metrics(chunk_size: u32, num_chunks: u32) -> MemTable {
    let schema = Schema::new()
        .col("timestamp", DType::I64)
        .col("value", DType::F64)
        .col("tag", DType::Str);
    MemTable::new(&schema, chunk_size, num_chunks)
}

/// Total rows across all pages of every sealed segment in `dir`.
fn cold_row_count(dir: &std::path::Path) -> usize {
    let store = ColdStore::open(dir).unwrap();
    store
        .segment_paths()
        .iter()
        .map(|p| {
            let r = SegmentReader::open(p).unwrap();
            r.pages().iter().map(|pg| pg.row_count as usize).sum::<usize>()
        })
        .sum()
}

#[test]
fn compactor_drains_only_sealed_chunks() {
    let dir = tmp_dir("compact-basic");
    let mut t = hot_metrics(512, 4);

    for i in 0..3 {
        t.push_row(&[Value::I64(100 + i), Value::F64(i as f64), Value::Str("a")]);
    }
    t.advance_chunk(); // seal chunk 0 (3 rows)
    for i in 0..2 {
        t.push_row(&[Value::I64(200 + i), Value::F64(i as f64), Value::Str("b")]);
    }
    t.advance_chunk(); // seal chunk 1 (2 rows)
    // chunk 2 stays Writing — must NOT be drained
    t.push_row(&[Value::I64(999), Value::F64(9.0), Value::Str("c")]);

    let store = ColdStore::open(&dir).unwrap();
    let cfg = CompactorConfig {
        target_segment_bytes: 1 << 30, // never roll on size
        ..Default::default()
    };
    let mut c = Compactor::new(store, cfg);
    let rows = c.drain_view("metrics", &t.view()).unwrap();
    assert_eq!(rows, 5, "only the two sealed chunks drain");

    // Draining again is idempotent — nothing new sealed.
    assert_eq!(c.drain_view("metrics", &t.view()).unwrap(), 0);

    let sealed = c.flush().unwrap().expect("one segment sealed");
    let r = SegmentReader::open(&sealed).unwrap();
    assert!(r.is_sealed());
    assert_eq!(r.pages().len(), 2);
    assert_eq!(r.ts_range(), Some((100, 201)));

    let id = r.table_id_by_name("metrics").unwrap();
    assert_eq!(r.table_def(id).unwrap().ts_col, Some(0));
    assert_eq!(r.read_page(0).unwrap()[0], ColumnData::I64(vec![100, 101, 102]));

    assert_eq!(cold_row_count(&dir), 5);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn compactor_rolls_by_size_and_reregisters_table() {
    let dir = tmp_dir("compact-roll");
    let mut t = hot_metrics(512, 4);

    for c in 0..3 {
        for i in 0..2 {
            t.push_row(&[
                Value::I64(1000 * c + i),
                Value::F64(i as f64),
                Value::Str("x"),
            ]);
        }
        t.advance_chunk(); // seal each chunk
    }

    let store = ColdStore::open(&dir).unwrap();
    let cfg = CompactorConfig {
        target_segment_bytes: 1, // force a roll after every page
        ..Default::default()
    };
    let mut c = Compactor::new(store, cfg);
    let rows = c.drain_view("metrics", &t.view()).unwrap();
    assert_eq!(rows, 6);
    assert!(c.flush().unwrap().is_none(), "no open segment after size rolls");

    // Three sealed chunks → three one-page segments, each independently
    // carrying the table definition (re-registered on every roll).
    let store = ColdStore::open(&dir).unwrap();
    let paths = store.segment_paths();
    assert_eq!(paths.len(), 3);
    for p in &paths {
        let r = SegmentReader::open(p).unwrap();
        assert!(r.is_sealed());
        assert_eq!(r.pages().len(), 1);
        assert!(r.table_id_by_name("metrics").is_some());
    }
    assert_eq!(cold_row_count(&dir), 6);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn compactor_multi_table_shares_segments() {
    let dir = tmp_dir("compact-multi");
    let mut a = hot_metrics(512, 4);
    let mut b = hot_metrics(512, 4);
    for i in 0..2 {
        a.push_row(&[Value::I64(i), Value::F64(0.0), Value::Str("a")]);
        b.push_row(&[Value::I64(100 + i), Value::F64(1.0), Value::Str("b")]);
    }
    a.advance_chunk();
    b.advance_chunk();

    let store = ColdStore::open(&dir).unwrap();
    let mut c = Compactor::new(
        store,
        CompactorConfig {
            target_segment_bytes: 1 << 30,
            ..Default::default()
        },
    );
    c.drain_view("table_a", &a.view()).unwrap();
    c.drain_view("table_b", &b.view()).unwrap();
    c.flush().unwrap();

    // Both tables land in a single shared segment file.
    let store = ColdStore::open(&dir).unwrap();
    let paths = store.segment_paths();
    assert_eq!(paths.len(), 1);
    let r = SegmentReader::open(&paths[0]).unwrap();
    assert_eq!(r.table_defs().len(), 2);
    assert!(r.table_id_by_name("table_a").is_some());
    assert!(r.table_id_by_name("table_b").is_some());
    assert_eq!(r.pages().len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn compactor_background_thread_drains_on_stop() {
    let dir = tmp_dir("compact-spawn");
    let file = dir.join("hot.memt");
    let schema = Schema::new()
        .col("timestamp", DType::I64)
        .col("value", DType::F64);

    // Writer handle (application side) and an independent read handle the
    // compactor thread owns — same mmap'd file, lock-free reads.
    let mut writer = MemTable::file_at(&file, &schema, 512, 4).unwrap();
    let reader = MemTable::open_file(&file).unwrap();

    let store = ColdStore::open(&dir).unwrap();
    let handle = Compactor::new(
        store,
        CompactorConfig {
            target_segment_bytes: 1 << 30,
            poll_interval: Duration::from_millis(10),
            ..Default::default()
        },
    )
    .spawn(vec![("metrics".to_string(), reader)]);

    for i in 0..4 {
        writer.push_row(&[Value::I64(i), Value::F64(i as f64)]);
    }
    writer.advance_chunk();
    std::thread::sleep(Duration::from_millis(40));
    for i in 0..3 {
        writer.push_row(&[Value::I64(100 + i), Value::F64(i as f64)]);
    }
    writer.advance_chunk();

    // stop() performs a final drain + flush before joining.
    handle.stop();

    assert_eq!(cold_row_count(&dir), 7);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn compactor_enforce_evicts_oldest_segments() {
    let dir = tmp_dir("compact-evict");
    let mut t = hot_metrics(512, 8);
    for c in 0..5 {
        for i in 0..2 {
            t.push_row(&[Value::I64(c * 10 + i), Value::F64(0.0), Value::Str("x")]);
        }
        t.advance_chunk();
    }

    let store = ColdStore::open(&dir).unwrap();
    let mut c = Compactor::new(
        store,
        CompactorConfig {
            target_segment_bytes: 1, // one segment per page
            max_total_bytes: Some(1), // keep only the protected newest
            ..Default::default()
        },
    );
    c.drain_view("metrics", &t.view()).unwrap();
    c.flush().unwrap();
    assert_eq!(c.stats().segment_count, 5);

    let removed = c.enforce();
    assert!(!removed.is_empty(), "over-budget segments evicted");
    // enforce_limits never deletes the newest segment.
    assert!(c.stats().segment_count >= 1);
    assert!(c.stats().segment_count < 5);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn compactor_restart_dedup_via_prime() {
    let dir = tmp_dir("compact-restart");
    let mut t = hot_metrics(512, 4);
    for c in 0..2 {
        for i in 0..2 {
            t.push_row(&[Value::I64(c * 10 + i), Value::F64(0.0), Value::Str("x")]);
        }
        t.advance_chunk(); // seal chunks 0 and 1
    }

    let cfg = || CompactorConfig {
        target_segment_bytes: 1 << 30,
        ..Default::default()
    };

    // First run: drain the two sealed chunks into cold.
    {
        let mut c = Compactor::new(ColdStore::open(&dir).unwrap(), cfg());
        assert_eq!(c.drain_view("metrics", &t.view()).unwrap(), 4);
        c.flush().unwrap();
    }
    assert_eq!(cold_row_count(&dir), 4);

    // Simulated restart over the SAME cold dir. prime_from_cold rebuilds the
    // per-chunk watermark from persisted source_gen/source_chunk, so the same
    // still-resident sealed chunks are recognised as already compacted.
    {
        let mut c = Compactor::new(ColdStore::open(&dir).unwrap(), cfg());
        c.prime_from_cold().unwrap();
        assert_eq!(c.drain_view("metrics", &t.view()).unwrap(), 0);
        assert!(c.flush().unwrap().is_none(), "nothing new to seal");
    }
    assert_eq!(cold_row_count(&dir), 4, "exactly-once: no duplication");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn compactor_without_prime_redrains_on_restart() {
    // Negative control: this is precisely the duplication prime_from_cold
    // prevents. Without priming, a fresh compactor re-drains resident chunks.
    let dir = tmp_dir("compact-noprime");
    let mut t = hot_metrics(512, 4);
    for i in 0..2 {
        t.push_row(&[Value::I64(i), Value::F64(0.0), Value::Str("x")]);
    }
    t.advance_chunk();

    let cfg = || CompactorConfig {
        target_segment_bytes: 1 << 30,
        ..Default::default()
    };

    {
        let mut c = Compactor::new(ColdStore::open(&dir).unwrap(), cfg());
        assert_eq!(c.drain_view("metrics", &t.view()).unwrap(), 2);
        c.flush().unwrap();
    }
    {
        let mut c = Compactor::new(ColdStore::open(&dir).unwrap(), cfg());
        // No prime_from_cold → the resident sealed chunk is drained again.
        assert_eq!(c.drain_view("metrics", &t.view()).unwrap(), 2);
        c.flush().unwrap();
    }
    assert_eq!(cold_row_count(&dir), 4, "duplicated without priming");

    let _ = std::fs::remove_dir_all(&dir);
}
