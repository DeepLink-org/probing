//! Filesystem-based memtable discovery protocol.
//!
//! Each process exposes memtables as mmap'd files under a shared directory:
//!
//! ```text
//! /dev/shm/probing/<pid>/
//! ├── metrics              ← no dot → SQL ``memtable.metrics``
//! ├── pulsing.actors       ← first `.` splits schema/table → SQL ``pulsing.actors``
//! ├── acme.widgets         ← any prefix works the same → SQL ``acme.widgets``
//! └── foo.bar.baz          ← table name is ``bar.baz`` (rest after first ``.``)
//! ```
//!
//! Discovery is `readdir`; reading is `mmap` + [`MemTableView::new`].
//! The memtable header embeds `creator_pid` and `creator_start_time`,
//! allowing readers to detect whether the creating process is still alive.
//!
//! # Example
//!
//! ```rust,no_run
//! use probing_memtable::discover::{ExposedTable, discover};
//! use probing_memtable::{Schema, DType, Value};
//!
//! // Writer: expose a table as an mmap'd file
//! let schema = Schema::new().col("ts", DType::I64).col("cpu", DType::F64);
//! let mut table = ExposedTable::create("metrics", &schema, 4096, 8).unwrap();
//! {
//!     let mut w = table.writer();
//!     w.push_row(&[Value::I64(1000), Value::F64(0.85)]);
//! }
//!
//! // Reader (same or different process): discover and read
//! for t in discover().unwrap() {
//!     if t.is_alive() {
//!         let view = t.view().unwrap();
//!         for row in view.rows(view.write_chunk()) {
//!             let mut c = row.cursor();
//!             println!("{} {}", c.next_i64(), c.next_f64());
//!         }
//!     }
//! }
//! ```

use crate::layout::{header, MAGIC};
use crate::memh::layout::required_total_size as memh_required_size;
use crate::memh::table::init_buf as memh_init_buf;
use crate::memh::{MemhView, MemhWriter};
use crate::memtable::{MemTable, MemTableView, MemTableWriter};
use crate::raw::{init_buf, process_start_time, validate_buf};
use crate::schema::{Schema, Value};

use memmap2::{Mmap, MmapMut};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

/// Platform-appropriate shared-memory directory for memtable files.
///
/// - **Linux**: `/dev/shm/probing` (guaranteed tmpfs, memory-only).
/// - **Other**: `$TMPDIR/probing` (may be disk-backed).
///
/// Override with the `PROBING_DATA_DIR` environment variable.
pub fn default_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("PROBING_DATA_DIR") {
        return PathBuf::from(dir);
    }
    #[cfg(target_os = "linux")]
    {
        let shm = Path::new("/dev/shm");
        if shm.exists() {
            return shm.join("probing");
        }
    }
    std::env::temp_dir().join("probing")
}

/// Check whether the process identified by `(pid, start_time)` is still alive.
///
/// - Returns `false` if the PID does not exist.
/// - If `expected_start_time != 0`, also verifies that the current occupant
///   of that PID started at the expected time (detecting PID recycling).
pub fn is_creator_alive(pid: u32, expected_start_time: u64) -> bool {
    if pid == 0 {
        return false;
    }
    let ret = unsafe { libc::kill(pid as libc::c_int, 0) };
    if ret != 0 {
        let err = io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::EPERM) {
            return false;
        }
    }
    if expected_start_time != 0 {
        let actual = process_start_time(pid);
        if actual != 0 && actual != expected_start_time {
            return false;
        }
    }
    true
}

// ── ExposedTable ──────────────────────────────────────────────────────

/// A memtable backed by an mmap'd file, exposed for cross-process discovery.
///
/// On [`Drop`], the file is removed. If the parent `<pid>/` directory is
/// empty afterward, it is removed too.
pub struct ExposedTable {
    mmap: MmapMut,
    path: PathBuf,
    dir: PathBuf,
}

impl ExposedTable {
    /// Create a table in the [`default_dir`].
    pub fn create(
        name: &str,
        schema: &Schema,
        chunk_size: u32,
        num_chunks: u32,
    ) -> io::Result<Self> {
        Self::create_in(&default_dir(), name, schema, chunk_size, num_chunks)
    }

    /// Create a table in a custom base directory.
    ///
    /// The file will be at `<base_dir>/<pid>/<name>`.
    pub fn create_in(
        base_dir: &Path,
        name: &str,
        schema: &Schema,
        chunk_size: u32,
        num_chunks: u32,
    ) -> io::Result<Self> {
        let dir = base_dir.join(std::process::id().to_string());
        fs::create_dir_all(&dir)?;

        let path = dir.join(name);
        let size = MemTable::required_size(schema, chunk_size as usize, num_chunks as usize);

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        file.set_len(size as u64)?;

        let mut mmap = unsafe { MmapMut::map_mut(&file)? };
        init_buf(&mut mmap, schema, chunk_size, num_chunks);

        Ok(Self { mmap, path, dir })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.mmap
    }

    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        &mut self.mmap
    }

    /// File path of this table.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Create a [`MemTableWriter`] backed by the mmap'd region.
    ///
    /// **Note**: this re-validates the entire buffer on every call.
    /// Prefer [`push_row`](Self::push_row) for hot-path writes.
    pub fn writer(&mut self) -> MemTableWriter<'_> {
        MemTableWriter::new(&mut self.mmap).expect("mmap buffer validated at creation")
    }

    /// Append a row without re-validating the buffer.
    ///
    /// This is the fast path for high-frequency writes — it skips the
    /// O(rows × chunks) `validate_buf` that `writer()` performs on every call.
    /// Safe because the buffer was validated at `create()` time and only
    /// mutated through well-formed write operations.
    ///
    /// # Panic safety
    ///
    /// The spinlock is released even if the write panics (e.g. row exceeds
    /// chunk capacity), preventing a deadlocked mmap file.
    pub fn push_row(&mut self, values: &[Value]) {
        use crate::layout::{acquire_write_lock, release_write_lock};
        use crate::memtable::push_plain_row;
        use crate::raw::validate_row_schema;

        debug_assert!(
            validate_row_schema(&self.mmap, values),
            "value types do not match schema"
        );

        acquire_write_lock(&mut self.mmap);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            push_plain_row(&mut self.mmap, values);
        }));
        release_write_lock(&mut self.mmap);

        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    }

    /// Create a read-only [`MemTableView`].
    pub fn view(&self) -> MemTableView<'_> {
        MemTableView::new(&self.mmap).expect("mmap buffer validated at creation")
    }
}

impl Drop for ExposedTable {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir(&self.dir); // succeeds only if empty
    }
}

// ── ExposedHashTable ─────────────────────────────────────────────────

/// An MEMH hash table backed by an mmap'd file, exposed for cross-process discovery.
///
/// On [`Drop`], the file is removed (same lifecycle as [`ExposedTable`]).
pub struct ExposedHashTable {
    mmap: MmapMut,
    path: PathBuf,
    dir: PathBuf,
}

impl ExposedHashTable {
    /// Create a hash table in the [`default_dir`].
    pub fn create(
        name: &str,
        num_buckets: u32,
        arena_cap: usize,
        hash_seed: u64,
    ) -> io::Result<Self> {
        Self::create_in(&default_dir(), name, num_buckets, arena_cap, hash_seed)
    }

    /// Create a hash table in a custom base directory.
    pub fn create_in(
        base_dir: &Path,
        name: &str,
        num_buckets: u32,
        arena_cap: usize,
        hash_seed: u64,
    ) -> io::Result<Self> {
        let dir = base_dir.join(std::process::id().to_string());
        fs::create_dir_all(&dir)?;

        let path = dir.join(name);
        let size = memh_required_size(num_buckets, arena_cap);

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        file.set_len(size as u64)?;

        let mut mmap = unsafe { MmapMut::map_mut(&file)? };
        memh_init_buf(&mut mmap, num_buckets, arena_cap, hash_seed)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{e}")))?;

        Ok(Self { mmap, path, dir })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.mmap
    }

    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        &mut self.mmap
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn writer(&mut self) -> MemhWriter<'_> {
        MemhWriter::new(&mut self.mmap).expect("mmap buffer validated at creation")
    }

    pub fn view(&self) -> MemhView<'_> {
        MemhView::new(&self.mmap).expect("mmap buffer validated at creation")
    }
}

impl Drop for ExposedHashTable {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir(&self.dir);
    }
}

// ── DiscoveredTable ───────────────────────────────────────────────────

/// A memtable discovered on the filesystem (read-only mmap).
pub struct DiscoveredTable {
    mmap: Mmap,
    path: PathBuf,
    pid: u32,
    name: String,
}

impl DiscoveredTable {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.mmap
    }

    /// Wrap the mmap'd region as a [`MemTableView`].
    pub fn view(&self) -> Result<MemTableView<'_>, &'static str> {
        MemTableView::new(&self.mmap)
    }

    /// Check if the process that created this table is still alive.
    pub fn is_alive(&self) -> bool {
        let h = header(&self.mmap);
        is_creator_alive(h.creator_pid, h.creator_start_time)
    }
}

// ── Discovery ─────────────────────────────────────────────────────────

/// Discover all valid memtable files in the [`default_dir`].
pub fn discover() -> io::Result<Vec<DiscoveredTable>> {
    discover_in(&default_dir())
}

/// Discover all valid memtable files under `base_dir`.
///
/// Scans `<base_dir>/<pid>/<name>` entries, mmaps each file, and
/// validates the memtable header. Invalid files are silently skipped.
pub fn discover_in(base_dir: &Path) -> io::Result<Vec<DiscoveredTable>> {
    let mut tables = Vec::new();

    let entries = match fs::read_dir(base_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(tables),
        Err(e) => return Err(e),
    };

    for pid_entry in entries.flatten() {
        let pid_name = pid_entry.file_name().to_string_lossy().to_string();
        let pid: u32 = match pid_name.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };

        let pid_dir = pid_entry.path();
        if !pid_dir.is_dir() {
            continue;
        }

        let table_entries = match fs::read_dir(&pid_dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for table_entry in table_entries.flatten() {
            let table_path = table_entry.path();
            if !table_path.is_file() {
                continue;
            }

            let file = match File::open(&table_path) {
                Ok(f) => f,
                Err(_) => continue,
            };

            let mmap = match unsafe { Mmap::map(&file) } {
                Ok(m) => m,
                Err(_) => continue,
            };

            if validate_buf(&mmap).is_err() {
                continue;
            }

            let name = table_entry.file_name().to_string_lossy().to_string();

            tables.push(DiscoveredTable {
                mmap,
                path: table_path,
                pid,
                name,
            });
        }
    }

    Ok(tables)
}

// ── Cleanup ───────────────────────────────────────────────────────────

/// Remove stale entries (dead processes) from the [`default_dir`].
/// Returns the number of directories cleaned.
pub fn cleanup() -> io::Result<usize> {
    cleanup_in(&default_dir())
}

/// Remove stale entries (dead processes) from `base_dir`.
pub fn cleanup_in(base_dir: &Path) -> io::Result<usize> {
    let mut cleaned = 0;

    let entries = match fs::read_dir(base_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let pid: u32 = match name.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };

        let pid_dir = entry.path();
        if !pid_dir.is_dir() {
            continue;
        }

        let start_time = read_any_start_time(&pid_dir);
        if !is_creator_alive(pid, start_time) {
            let _ = fs::remove_dir_all(&pid_dir);
            cleaned += 1;
        }
    }

    Ok(cleaned)
}

fn read_any_start_time(dir: &Path) -> u64 {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    for entry in entries.flatten() {
        if let Ok(file) = File::open(entry.path()) {
            if let Ok(mmap) = unsafe { Mmap::map(&file) } {
                if mmap.len() >= std::mem::size_of::<crate::layout::Header>() {
                    let h = header(&mmap);
                    if h.magic == MAGIC {
                        return h.creator_start_time;
                    }
                }
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{DType, Value};
    use std::sync::atomic::{AtomicU32, Ordering as AtOrd};

    static TEST_SEQ: AtomicU32 = AtomicU32::new(0);

    fn test_dir() -> PathBuf {
        let seq = TEST_SEQ.fetch_add(1, AtOrd::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("probing_mt_test_{}_{}", std::process::id(), seq,));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn exposed_table_roundtrip() {
        let dir = test_dir();
        let schema = Schema::new().col("ts", DType::I64).col("val", DType::F64);

        {
            let mut table = ExposedTable::create_in(&dir, "metrics", &schema, 4096, 4).unwrap();
            assert!(table.path().exists());

            {
                let mut w = table.writer();
                w.push_row(&[Value::I64(1000), Value::F64(3.14)]);
                w.push_row(&[Value::I64(2000), Value::F64(2.72)]);
            }

            let v = table.view();
            assert_eq!(v.num_rows(0), 2);
            assert_eq!(v.creator_pid(), std::process::id());
            #[cfg(target_os = "linux")]
            assert_ne!(v.creator_start_time(), 0);
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_finds_table() {
        let dir = test_dir();
        let schema = Schema::new().col("x", DType::I32);

        let mut table = ExposedTable::create_in(&dir, "test_table", &schema, 1024, 2).unwrap();
        {
            let mut w = table.writer();
            w.push_row(&[Value::I32(42)]);
        }

        let found = discover_in(&dir).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name(), "test_table");
        assert_eq!(found[0].pid(), std::process::id());
        assert!(found[0].is_alive());

        let view = found[0].view().unwrap();
        let mut c = view.rows(0).next().unwrap().cursor();
        assert_eq!(c.next_i32(), 42);

        drop(table);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cleanup_removes_dead_pid() {
        let dir = test_dir();
        let fake_pid = 2_000_000_000u32; // almost certainly not a real PID

        let fake_dir = dir.join(fake_pid.to_string());
        fs::create_dir_all(&fake_dir).unwrap();

        let schema = Schema::new().col("x", DType::I32);
        let size = MemTable::required_size(&schema, 256, 1);
        let mut buf = vec![0u8; size];
        init_buf(&mut buf, &schema, 256, 1);
        crate::layout::header_mut(&mut buf).creator_pid = fake_pid;

        fs::write(fake_dir.join("data"), &buf).unwrap();

        let cleaned = cleanup_in(&dir).unwrap();
        assert_eq!(cleaned, 1);
        assert!(!fake_dir.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_empty_dir() {
        let dir = test_dir();
        let found = discover_in(&dir).unwrap();
        assert!(found.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_nonexistent_dir() {
        let dir = PathBuf::from("/tmp/probing_memtable_nonexistent_dir_12345");
        let found = discover_in(&dir).unwrap();
        assert!(found.is_empty());
    }

    #[test]
    fn is_creator_alive_current_process() {
        let pid = std::process::id();
        let start = process_start_time(pid);
        assert!(is_creator_alive(pid, start));
    }

    #[test]
    fn is_creator_alive_dead_pid() {
        assert!(!is_creator_alive(2_000_000_000, 0));
    }

    #[test]
    fn drop_cleans_up_file() {
        let dir = test_dir();
        let schema = Schema::new().col("x", DType::I32);
        let path;
        {
            let table = ExposedTable::create_in(&dir, "ephemeral", &schema, 256, 1).unwrap();
            path = table.path().to_owned();
            assert!(path.exists());
        }
        assert!(!path.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
