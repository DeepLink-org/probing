use crate::layout::r32;
use crate::row::Row;
use std::collections::{HashMap, HashSet, VecDeque};

/// Cache key: (byte offset, chunk generation).
///
/// Including the generation prevents stale cache hits after ring-buffer wrap.
type CacheKey = (usize, u64);

/// Stateful reader with a generation-aware string cache.
///
/// Caches the most recent `window` resolved strings plus any strings
/// that were resolved via dedup references (pinned). Pinned entries
/// are capped at `max_pinned` to prevent unbounded cache growth.
///
/// Cache keys include the chunk generation so that a recycled chunk
/// at the same offset never produces a stale hit.
pub struct CachedReader<'a> {
    buf: &'a [u8],
    cache: HashMap<CacheKey, &'a [u8]>,
    recent: VecDeque<CacheKey>,
    pinned: HashSet<CacheKey>,
    pinned_order: VecDeque<CacheKey>,
    window: usize,
    max_pinned: usize,
}

impl<'a> CachedReader<'a> {
    pub fn new(buf: &'a [u8], window: usize) -> Self {
        Self::with_limits(buf, window, 4 * window)
    }

    pub fn with_limits(buf: &'a [u8], window: usize, max_pinned: usize) -> Self {
        Self {
            buf,
            cache: HashMap::new(),
            recent: VecDeque::new(),
            pinned: HashSet::new(),
            pinned_order: VecDeque::new(),
            window,
            max_pinned,
        }
    }

    /// Resolve a dedup reference (the target is pinned in cache).
    pub fn resolve_ref(&mut self, data_off: usize, generation: u64) -> &'a [u8] {
        let key = (data_off, generation);
        let cached = self.cache.get(&key).copied();
        if let Some(b) = cached {
            if self.pinned.insert(key) {
                self.pinned_order.push_back(key);
                self.evict();
            }
            return b;
        }
        let len = r32(self.buf, data_off) as usize;
        let end = data_off.saturating_add(4).saturating_add(len);
        if end > self.buf.len() {
            panic!("stale CachedReader: dedup resolve out of bounds");
        }
        let b = &self.buf[data_off + 4..end];
        self.cache.insert(key, b);
        self.recent.push_back(key);
        self.pinned.insert(key);
        self.pinned_order.push_back(key);
        self.evict();
        b
    }

    /// Cache an inline entry (evictable unless later pinned).
    pub fn cache_inline(&mut self, abs_off: usize, data: &'a [u8], generation: u64) {
        let key = (abs_off, generation);
        if !self.cache.contains_key(&key) {
            self.cache.insert(key, data);
            self.recent.push_back(key);
            self.evict();
        }
    }

    fn evict(&mut self) {
        while self.recent.len() > self.window {
            let old = self.recent.pop_front().unwrap();
            if !self.pinned.contains(&old) {
                self.cache.remove(&old);
            }
        }
        while self.pinned.len() > self.max_pinned {
            if let Some(oldest) = self.pinned_order.pop_front() {
                if self.pinned.remove(&oldest) && !self.recent.contains(&oldest) {
                    self.cache.remove(&oldest);
                }
            } else {
                break;
            }
        }
    }

    pub fn stats(&self) -> (usize, usize) {
        (self.cache.len(), self.pinned.len())
    }

    pub fn cursor(&mut self, row: &Row<'a>) -> CachedCursor<'a, '_> {
        let abs_base = row.data.as_ptr() as usize - row.buf.as_ptr() as usize;
        CachedCursor {
            data: row.data,
            pos: 0,
            abs_base,
            chunk_start: row.chunk_start,
            generation: row.generation,
            cache: self,
        }
    }
}

/// Sequential cursor with generation-aware cached string resolution.
pub struct CachedCursor<'a, 'c> {
    data: &'a [u8],
    pos: usize,
    abs_base: usize,
    chunk_start: usize,
    generation: u64,
    cache: &'c mut CachedReader<'a>,
}

impl<'a> CachedCursor<'a, '_> {
    fn read_fixed<const N: usize>(&mut self) -> [u8; N] {
        let v: [u8; N] = self.data[self.pos..self.pos + N].try_into().unwrap();
        self.pos += N;
        v
    }

    fn read_lp_cached(&mut self) -> &'a [u8] {
        let abs_off = self.abs_base + self.pos;
        let raw = i32::from_le_bytes(self.read_fixed::<4>());
        if raw < 0 {
            let data_off = self.chunk_start + (-raw) as usize;
            self.cache.resolve_ref(data_off, self.generation)
        } else {
            let len = raw as usize;
            let b = &self.data[self.pos..self.pos + len];
            self.pos += len;
            self.cache.cache_inline(abs_off, b, self.generation);
            b
        }
    }

    pub fn next_u8(&mut self) -> u8 {
        self.read_fixed::<1>()[0]
    }
    pub fn next_u32(&mut self) -> u32 {
        u32::from_le_bytes(self.read_fixed())
    }
    pub fn next_i32(&mut self) -> i32 {
        i32::from_le_bytes(self.read_fixed())
    }
    pub fn next_i64(&mut self) -> i64 {
        i64::from_le_bytes(self.read_fixed())
    }
    pub fn next_f32(&mut self) -> f32 {
        f32::from_le_bytes(self.read_fixed())
    }
    pub fn next_f64(&mut self) -> f64 {
        f64::from_le_bytes(self.read_fixed())
    }
    pub fn next_u64(&mut self) -> u64 {
        u64::from_le_bytes(self.read_fixed())
    }
    pub fn next_str(&mut self) -> &'a str {
        let b = self.read_lp_cached();
        if b.is_empty() {
            ""
        } else {
            std::str::from_utf8(b).unwrap_or("")
        }
    }
    pub fn next_bytes(&mut self) -> &'a [u8] {
        self.read_lp_cached()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buf::init_buf;
    use crate::memtable::DedupWriter;
    use crate::memtable::{MemTable, MemTableMut, MemTableView};
    use crate::schema::{DType, Schema};
    use crate::value::Value;
    use std::sync::atomic::Ordering;

    #[test]
    fn cached_reader_basic() {
        let schema = Schema::new().col("id", DType::I64).col("tag", DType::Str);
        let size = MemTable::required_size(&schema, 4096, 1);
        let mut buf = vec![0u8; size];
        {
            let mut dw = DedupWriter::init(&mut buf, &schema, 4096, 1);
            for i in 0..10i64 {
                dw.row_writer().put_i64(i).put_str("same_tag").finish();
            }
        }

        let view = MemTableView::new(&buf).unwrap();
        let mut cache = CachedReader::new(view.as_bytes(), 64);

        for (i, row) in view.rows(0).enumerate() {
            let mut c = cache.cursor(&row);
            assert_eq!(c.next_i64(), i as i64);
            assert_eq!(c.next_str(), "same_tag");
        }

        let (cached, pinned) = cache.stats();
        assert!(pinned > 0, "referenced strings should be pinned");
        assert!(cached > 0);
    }

    #[test]
    fn cached_reader_window_eviction() {
        let schema = Schema::new().col("name", DType::Str);
        let size = MemTable::required_size(&schema, 16384, 1);
        let mut buf = vec![0u8; size];
        {
            let mut dw = DedupWriter::init(&mut buf, &schema, 16384, 1);
            // Write 100 unique strings, no dedup → all inline
            for i in 0..100 {
                dw.push_row(&[Value::Str(&format!("unique_{i}"))]);
            }
        }

        // window=8: should evict old entries
        let view = MemTableView::new(&buf).unwrap();
        let mut cache = CachedReader::new(view.as_bytes(), 8);

        for (i, row) in view.rows(0).enumerate() {
            let mut c = cache.cursor(&row);
            assert_eq!(c.next_str(), format!("unique_{i}"));
        }

        let (cached, pinned) = cache.stats();
        assert!(cached <= 8, "cache should respect window: got {cached}");
        assert_eq!(pinned, 0, "no dedup refs → no pinned entries");
    }

    #[test]
    fn cached_reader_pinned_not_evicted() {
        let schema = Schema::new()
            .col("level", DType::Str)
            .col("seq", DType::I32);
        let size = MemTable::required_size(&schema, 8192, 1);
        let mut buf = vec![0u8; size];
        {
            let mut dw = DedupWriter::init(&mut buf, &schema, 8192, 1);
            // "INFO" repeated → dedup refs after first
            for i in 0..20 {
                dw.push_row(&[Value::Str("INFO"), Value::I32(i)]);
            }
        }

        // window=4 but pinned entries survive eviction
        let view = MemTableView::new(&buf).unwrap();
        let mut cache = CachedReader::new(view.as_bytes(), 4);

        for (i, row) in view.rows(0).enumerate() {
            let mut c = cache.cursor(&row);
            assert_eq!(c.next_str(), "INFO");
            assert_eq!(c.next_i32(), i as i32);
        }

        let (_cached, pinned) = cache.stats();
        assert!(pinned > 0, "dedup target should be pinned");
    }

    #[test]
    fn cached_reader_pinned_limit() {
        let schema = Schema::new().col("tag", DType::Str).col("seq", DType::I32);
        let size = MemTable::required_size(&schema, 65536, 1);
        let mut buf = vec![0u8; size];
        {
            let mut dw = DedupWriter::init(&mut buf, &schema, 65536, 1);
            // 100 unique tags, each repeated twice → 100 pinned targets
            for i in 0..100 {
                let tag = format!("tag_{i}");
                dw.push_row(&[Value::Str(&tag), Value::I32(i as i32)]);
                dw.push_row(&[Value::Str(&tag), Value::I32(i as i32 + 1000)]);
            }
        }

        // max_pinned=10 → pinned entries should be capped
        let view = MemTableView::new(&buf).unwrap();
        let mut cache = CachedReader::with_limits(view.as_bytes(), 8, 10);

        for row in view.rows(0) {
            let mut c = cache.cursor(&row);
            let tag = c.next_str();
            let _seq = c.next_i32();
            assert!(tag.starts_with("tag_"));
        }

        let (_cached, pinned) = cache.stats();
        assert!(
            pinned <= 10,
            "pinned should be capped at max_pinned=10, got {pinned}"
        );
    }

    #[test]
    fn stress_concurrent_dedup_write_cached_read() {
        use std::alloc;
        use std::sync::atomic::AtomicBool;
        use std::sync::{Arc, Barrier};
        use std::thread;

        let schema = Schema::new().col("key", DType::Str).col("val", DType::I64);
        let size = MemTable::required_size(&schema, 16384, 4);
        let layout = alloc::Layout::from_size_align(size, 64).unwrap();
        let ptr = unsafe { alloc::alloc_zeroed(layout) };
        assert!(!ptr.is_null());

        unsafe {
            let buf = std::slice::from_raw_parts_mut(ptr, size);
            init_buf(buf, &schema, 16384, 4);
        }

        let addr = ptr as usize;
        let num_writers = 4;
        let rows_per_writer = 300;
        let num_readers = 4;
        let done = Arc::new(AtomicBool::new(false));
        let barrier = Arc::new(Barrier::new(1 + num_readers));

        let reader_handles: Vec<_> = (0..num_readers)
            .map(|_| {
                let done = done.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let mut reads = 0usize;
                    let keys = ["k_a", "k_b", "k_c", "k_d", "k_e"];
                    while !done.load(Ordering::Acquire) {
                        let buf = unsafe { std::slice::from_raw_parts(addr as *const u8, size) };
                        let view = MemTableView::new(buf).unwrap();
                        let mut cache = CachedReader::new(buf, 32);
                        for chunk in 0..view.num_chunks() {
                            for row in view.rows(chunk) {
                                let mut c = cache.cursor(&row);
                                let key = c.next_str();
                                let val = c.next_i64();
                                assert!(
                                    keys.contains(&key) || key.is_empty(),
                                    "corrupt key: {key}"
                                );
                                assert!(val >= 0, "corrupt val: {val}");
                                reads += 1;
                            }
                        }
                        thread::yield_now();
                    }
                    reads
                })
            })
            .collect();

        let writer = {
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                let buf = unsafe { std::slice::from_raw_parts_mut(addr as *mut u8, size) };
                let mut mt = MemTableMut::new(buf).unwrap();
                let keys = ["k_a", "k_b", "k_c", "k_d", "k_e"];
                for tid in 0..num_writers {
                    for seq in 0..rows_per_writer as i64 {
                        mt.push_row(&[
                            Value::Str(keys[seq as usize % keys.len()]),
                            Value::I64(tid as i64 * 10000 + seq),
                        ]);
                    }
                }
            })
        };

        writer.join().unwrap();
        done.store(true, Ordering::Release);

        let mut total_reads = 0usize;
        for h in reader_handles {
            total_reads += h.join().unwrap();
        }
        assert!(total_reads > 0, "readers should have read some rows");

        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            let view = MemTableView::new(buf).unwrap();
            let total: usize = (0..view.num_chunks()).map(|c| view.num_rows(c)).sum();
            assert_eq!(total, num_writers * rows_per_writer);
            alloc::dealloc(ptr as *mut u8, layout);
        }
    }

    #[test]
    fn stress_cached_reader_high_cardinality_with_dedup() {
        let schema = Schema::new()
            .col("host", DType::Str)
            .col("path", DType::Str)
            .col("status", DType::I32);
        let size = MemTable::required_size(&schema, 65536, 2);
        let mut buf = vec![0u8; size];
        let hosts: Vec<String> = (0..10).map(|i| format!("host-{i}.example.com")).collect();
        let paths: Vec<String> = (0..50).map(|i| format!("/api/v1/resource/{i}")).collect();

        {
            let mut dw = DedupWriter::init(&mut buf, &schema, 65536, 2);
            for i in 0..1000 {
                dw.push_row(&[
                    Value::Str(&hosts[i % hosts.len()]),
                    Value::Str(&paths[i % paths.len()]),
                    Value::I32((200 + (i % 5) * 100) as i32),
                ]);
            }
        }

        // small window → heavy eviction pressure; pinned hosts/paths survive
        let view = MemTableView::new(&buf).unwrap();
        let mut cache = CachedReader::new(view.as_bytes(), 16);
        let mut count = 0;

        for chunk in 0..view.num_chunks() {
            for row in view.rows(chunk) {
                let mut c = cache.cursor(&row);
                let host = c.next_str();
                let path = c.next_str();
                let status = c.next_i32();
                assert!(host.starts_with("host-"), "bad host: {host}");
                assert!(path.starts_with("/api/"), "bad path: {path}");
                assert!(
                    [200, 300, 400, 500, 600].contains(&status),
                    "bad status: {status}"
                );
                count += 1;
            }
        }
        assert_eq!(count, 1000);

        let (cached, pinned) = cache.stats();
        assert!(pinned > 0, "should have pinned entries from dedup");
        assert!(cached > 0);
    }

    #[test]
    fn cached_reader_does_not_reuse_old_entry_after_generation_change() {
        let schema = Schema::new().col("s", DType::Str);
        let mut buf = vec![0u8; 4096];
        init_buf(&mut buf, &schema, 512, 2); // 2 chunks: wrap happens fast

        // Shared-memory style: reader holds &[u8] via raw pointer,
        // writer holds &mut [u8] — mirrors real cross-thread usage.
        let reader_buf: &[u8] = unsafe { std::slice::from_raw_parts(buf.as_ptr(), buf.len()) };

        // Phase 1: write "hello" into chunk 0
        {
            let mut m = MemTableMut::new(&mut buf).unwrap();
            m.push_row(&[Value::Str("hello")]);
        }
        let gen0 = MemTableView::new(reader_buf).unwrap().chunk_generation(0);

        // Phase 2: read with cache — populates cache for chunk 0
        let mut cache = CachedReader::new(reader_buf, 64);
        let view = MemTableView::new(reader_buf).unwrap();
        for row in view.rows(0) {
            let mut c = cache.cursor(&row);
            assert_eq!(c.next_str(), "hello");
        }
        let (cache_sz_before, _) = cache.stats();
        assert!(cache_sz_before > 0);

        // Phase 3: advance twice to recycle chunk 0 (0→1→0), write "world"
        {
            let mut m = MemTableMut::new(&mut buf).unwrap();
            m.advance_chunk(); // 0→1
            m.advance_chunk(); // 1→0 (chunk 0 recycled, generation bumped)
            m.push_row(&[Value::Str("world")]);
        }
        let gen0_new = view.chunk_generation(0);
        assert_ne!(gen0, gen0_new);

        // Phase 4: read chunk 0 again — must see "world", not cached "hello"
        for row in view.rows(0) {
            let mut c = cache.cursor(&row);
            assert_eq!(c.next_str(), "world");
        }
    }
}
