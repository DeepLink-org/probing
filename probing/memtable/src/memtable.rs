use crate::dedup::DedupState;
use crate::layout::{
    chunk_header, chunk_start_off, col_desc, compute_data_offset, header, header_mut, w32,
    CHUNK_HEADER_SIZE, FLAG_DEDUP,
};
use crate::raw::{
    advance_chunk_raw, init_buf, note_row_ts, row_ts, validate_buf, validate_row_schema,
    write_row_bytes,
};
use crate::refcount::refcount;
use crate::row::RowIter;
use crate::schema::{Col, DType, Schema, Value};
use crate::writer::RowWriter;
use memmap2::MmapMut;
use std::fmt;
use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

// ── Shared read-only accessor methods (expands inside each impl) ─────

macro_rules! impl_table_reader {
    () => {
        pub fn num_cols(&self) -> usize {
            header(self.as_bytes()).num_cols as usize
        }
        pub fn num_chunks(&self) -> usize {
            header(self.as_bytes()).num_chunks as usize
        }
        pub fn write_chunk(&self) -> usize {
            header(self.as_bytes()).write_chunk.load(Ordering::Acquire) as usize
        }
        pub fn data_offset(&self) -> usize {
            header(self.as_bytes()).data_offset as usize
        }
        pub fn chunk_size(&self) -> usize {
            header(self.as_bytes()).chunk_size as usize
        }
        pub fn refcount(&self) -> u32 {
            refcount(self.as_bytes())
        }
        pub fn col_name(&self, i: usize) -> &str {
            col_desc(self.as_bytes(), i).name_str()
        }
        pub fn col_dtype(&self, i: usize) -> Option<DType> {
            DType::from_u32(col_desc(self.as_bytes(), i).dtype)
        }
        pub fn col_elem_size(&self, i: usize) -> usize {
            col_desc(self.as_bytes(), i).elem_size as usize
        }
        pub fn chunk_used(&self, chunk: usize) -> usize {
            let buf = self.as_bytes();
            let cs = chunk_start_off(buf, chunk);
            chunk_header(buf, cs).used.load(Ordering::Acquire) as usize
        }
        pub fn chunk_generation(&self, chunk: usize) -> u64 {
            let buf = self.as_bytes();
            let cs = chunk_start_off(buf, chunk);
            chunk_header(buf, cs).generation.load(Ordering::Acquire)
        }
        pub fn chunk_state(&self, chunk: usize) -> u32 {
            let buf = self.as_bytes();
            let cs = chunk_start_off(buf, chunk);
            chunk_header(buf, cs).state.load(Ordering::Acquire)
        }
        /// Index of the designated timestamp column ([`None`] when the
        /// schema has no `I64` column named `timestamp` / `ts`).
        pub fn ts_col(&self) -> Option<usize> {
            match header(self.as_bytes()).ts_col as usize {
                0 => None,
                idx => Some(idx - 1),
            }
        }
        /// `(min, max)` of the designated timestamp column over the rows
        /// committed in `chunk`; [`None`] when the chunk is empty or the
        /// table has no timestamp column.
        ///
        /// The `used` Acquire load pairs with the writer's Release store
        /// that publishes each row, so the returned range covers every row
        /// visible to this reader. Like all chunk metadata the snapshot is
        /// racy: callers pruning by time must bracket it between two
        /// [`chunk_generation`](Self::chunk_generation) reads.
        pub fn chunk_ts_range(&self, chunk: usize) -> Option<(i64, i64)> {
            self.ts_col()?;
            let buf = self.as_bytes();
            let cs = chunk_start_off(buf, chunk);
            let ch = chunk_header(buf, cs);
            let _used = ch.used.load(Ordering::Acquire);
            let min = ch.min_ts.load(Ordering::Relaxed);
            let max = ch.max_ts.load(Ordering::Relaxed);
            if min > max {
                None // sentinel values: no committed rows
            } else {
                Some((min, max))
            }
        }
        pub fn rows(&self, chunk: usize) -> RowIter<'_> {
            let buf = self.as_bytes();
            let cs = chunk_start_off(buf, chunk);
            let ch = chunk_header(buf, cs);
            RowIter {
                buf,
                chunk_start: cs,
                pos: cs + CHUNK_HEADER_SIZE,
                end: cs + CHUNK_HEADER_SIZE + ch.used.load(Ordering::Acquire) as usize,
                generation: ch.generation.load(Ordering::Acquire),
            }
        }
        pub fn num_rows(&self, chunk: usize) -> usize {
            let buf = self.as_bytes();
            let cs = chunk_start_off(buf, chunk);
            chunk_header(buf, cs).row_count.load(Ordering::Acquire) as usize
        }
        /// Chunk indices in **logical (oldest → newest) write order**.
        ///
        /// The ring writes chunks in `(generation, index)` order: chunk 0 at
        /// generation 1, then chunks 1..N-1 at generation 1, then wraps back
        /// to chunk 0 at generation 2, and so on.  Sorting non-empty chunks
        /// by `(generation, index)` therefore recovers temporal order
        /// regardless of the current wrap position.
        ///
        /// Chunks that were never written (generation 0) or hold no
        /// committed rows are skipped.  The snapshot is racy by design:
        /// callers that read concurrently with a writer must re-check
        /// [`chunk_generation`](Self::chunk_generation) after consuming a
        /// chunk and discard it on mismatch.
        pub fn chunks_logical(&self) -> Vec<usize> {
            let mut order: Vec<(u64, usize)> = (0..self.num_chunks())
                .filter_map(|i| {
                    let generation = self.chunk_generation(i);
                    if generation == 0 || self.num_rows(i) == 0 {
                        None
                    } else {
                        Some((generation, i))
                    }
                })
                .collect();
            order.sort_unstable();
            order.into_iter().map(|(_, i)| i).collect()
        }
        pub fn creator_pid(&self) -> u32 {
            header(self.as_bytes()).creator_pid
        }
        pub fn creator_start_time(&self) -> u64 {
            header(self.as_bytes()).creator_start_time
        }
        pub fn schema(&self) -> Schema {
            let buf = self.as_bytes();
            let nc = header(buf).num_cols as usize;
            let mut s = Schema::new();
            for i in 0..nc {
                let cd = col_desc(buf, i);
                if let Some(dtype) = DType::from_u32(cd.dtype) {
                    s.cols.push(Col {
                        name: cd.name_str().to_string(),
                        dtype,
                        elem_size: cd.elem_size as usize,
                        doc: None,
                    });
                }
            }
            s
        }
    };
}

// ── Write helpers ────────────────────────────────────────────────────

fn make_row_writer<'a>(buf: &'a mut [u8], dedup: Option<&'a mut DedupState>) -> RowWriter<'a> {
    let h = header(buf);
    let wc = h.write_chunk.load(Ordering::Relaxed) as usize;
    let csz = h.chunk_size as usize;
    let doff = h.data_offset as usize;
    let ts_col = h.ts_col;
    let cs = doff + wc * csz;
    let used = chunk_header(buf, cs).used.load(Ordering::Relaxed) as usize;
    RowWriter {
        buf,
        dedup,
        chunk_start: cs,
        chunk_size: csz,
        row_start: cs + CHUNK_HEADER_SIZE + used,
        pos: cs + CHUNK_HEADER_SIZE + used + 4,
        overflow: false,
        done: false,
        col_idx: 0,
        ts_col,
        pending_ts: None,
    }
}

fn row_data_size(values: &[Value]) -> usize {
    values.iter().map(|v| v.encoded_size()).sum()
}

pub(crate) fn push_plain_row(buf: &mut [u8], values: &[Value]) -> bool {
    let row_data = row_data_size(values);
    if write_row_bytes(buf, values, row_data) {
        return true;
    }
    advance_chunk_raw(buf);
    if write_row_bytes(buf, values, row_data) {
        return true;
    }
    log::warn!("push_plain_row: row exceeds chunk capacity");
    false
}

const MAX_DEDUP_COLS: usize = 64;

fn append_row_dedup_bytes(buf: &mut [u8], state: &mut DedupState, values: &[Value]) -> bool {
    debug_assert!(
        validate_row_schema(buf, values),
        "value types do not match schema"
    );

    let n = values.len();
    assert!(n <= MAX_DEDUP_COLS, "column count exceeds MAX_COLS");

    let h = header(buf);
    let wc = h.write_chunk.load(Ordering::Relaxed) as usize;
    let csz = h.chunk_size as usize;
    let cs = h.data_offset as usize + wc * csz;
    let used = chunk_header(buf, cs).used.load(Ordering::Relaxed) as usize;

    let mut lookups = [None::<usize>; MAX_DEDUP_COLS];
    let mut row_data = 0usize;
    for (i, v) in values.iter().enumerate() {
        let dup = match v {
            Value::Str(s) => state.lookup(i, s.as_bytes()),
            Value::Bytes(b) => state.lookup(i, b),
            _ => None,
        };
        lookups[i] = dup;
        row_data += if dup.is_some() { 4 } else { v.encoded_size() };
    }

    let total = 4 + row_data;
    if CHUNK_HEADER_SIZE + used + total > csz {
        return false;
    }

    let row_start = cs + CHUNK_HEADER_SIZE + used;
    w32(buf, row_start, row_data as u32);
    let mut off = row_start + 4;
    for (i, v) in values.iter().enumerate() {
        let var_data = match v {
            Value::Str(s) => Some(s.as_bytes()),
            Value::Bytes(b) => Some(*b),
            _ => None,
        };
        if let Some(data) = var_data {
            if let Some(ref_off) = lookups[i] {
                buf[off..off + 4].copy_from_slice(&(-(ref_off as i32)).to_le_bytes());
                off += 4;
            } else {
                let chunk_off = off - cs;
                let n = v.encode(&mut buf[off..]);
                state.insert(i, data, chunk_off);
                off += n;
            }
        } else {
            off += v.encode(&mut buf[off..]);
        }
    }
    if let Some(ts) = row_ts(header(buf), values) {
        note_row_ts(chunk_header(buf, cs), ts);
    }
    chunk_header(buf, cs)
        .used
        .store((used + total) as u32, Ordering::Release);
    chunk_header(buf, cs)
        .row_count
        .fetch_add(1, Ordering::Release);
    true
}

// ── MemTable (owned buffer: heap or mmap'd shared memory) ───────────

/// Which kind of storage backs a [`MemTable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackingKind {
    /// Process-private heap allocation.
    Heap,
    /// POSIX shared memory object (`shm_open`) — memory-only.
    #[cfg(unix)]
    Shm,
    /// mmap'd regular file — disk-backed.
    File,
}

/// Storage behind a [`MemTable`].
enum Backing {
    /// Process-private heap allocation. Invisible to other processes;
    /// freed on drop.
    Heap(Vec<u8>),
    /// POSIX shared memory object (`shm_open` + `mmap`). Memory-only:
    /// never touches disk, gone after reboot. Other processes attach by
    /// name. When `unlink_on_drop`, the creator removes the name on drop
    /// (existing mappings stay valid until unmapped).
    #[cfg(unix)]
    Shm {
        mmap: MmapMut,
        name: String,
        unlink_on_drop: bool,
    },
    /// mmap'd regular file. Disk-backed: contents persist after drop /
    /// reboot unless `unlink_on_drop` is set (used by the discoverable
    /// `<data_dir>/<pid>/<name>` convention, where `dir` is the parent
    /// `<pid>/` directory to remove when it becomes empty).
    File {
        mmap: MmapMut,
        path: PathBuf,
        dir: Option<PathBuf>,
        unlink_on_drop: bool,
    },
}

impl Backing {
    #[inline]
    fn bytes(&self) -> &[u8] {
        match self {
            Backing::Heap(v) => v,
            #[cfg(unix)]
            Backing::Shm { mmap, .. } => mmap,
            Backing::File { mmap, .. } => mmap,
        }
    }

    #[inline]
    fn bytes_mut(&mut self) -> &mut [u8] {
        match self {
            Backing::Heap(v) => v,
            #[cfg(unix)]
            Backing::Shm { mmap, .. } => mmap,
            Backing::File { mmap, .. } => mmap,
        }
    }
}

/// Normalise a POSIX shm name: must start with `/`, no other slashes.
/// Keep names short — macOS limits them to 31 bytes (`PSHMNAMLEN`).
#[cfg(unix)]
fn shm_name_cstring(name: &str) -> io::Result<std::ffi::CString> {
    let normalised = if name.starts_with('/') {
        name.to_string()
    } else {
        format!("/{name}")
    };
    if normalised[1..].contains('/') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "shm name must not contain '/' (apart from the leading one)",
        ));
    }
    std::ffi::CString::new(normalised)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "shm name contains NUL"))
}

#[cfg(unix)]
fn cstring_to_shm_name(cname: std::ffi::CString) -> io::Result<String> {
    cname.into_string().map_err(|e| {
        log::error!("shm name is not valid UTF-8: {e:?}");
        io::Error::new(io::ErrorKind::InvalidInput, "shm name is not valid UTF-8")
    })
}

/// `shm_open` wrapper returning an owned [`std::fs::File`].
#[cfg(unix)]
fn shm_open_file(name: &std::ffi::CString, oflag: libc::c_int) -> io::Result<std::fs::File> {
    use std::os::fd::FromRawFd;
    let fd = unsafe { libc::shm_open(name.as_ptr(), oflag, 0o600 as libc::c_uint) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

/// Ring-buffer table that owns its storage. Three backings, one API:
///
/// | Constructor | Backing | Cross-process | Survives crash | Survives reboot |
/// |-------------|---------|--------------|----------------|-----------------|
/// | [`new`](Self::new) / [`from_buf`](Self::from_buf) | heap | no | no | no |
/// | [`shm`](Self::shm) / [`open_shm`](Self::open_shm) | POSIX shared memory | by name | yes¹ | no |
/// | [`file_at`](Self::file_at) / [`open_file`](Self::open_file) | mmap'd file | by path | yes | yes |
/// | [`shared`](Self::shared) / [`shared_in`](Self::shared_in) | mmap'd file under `<data_dir>/<pid>/` | discovery + SQL catalog | yes¹ | — |
///
/// ¹ until the name/file is unlinked (creator drop or stale-pid cleanup).
///
/// On Linux the discoverable `shared` flavour lives in `/dev/shm` (tmpfs),
/// so it is effectively shared memory *with* a browsable path; `shm` is the
/// portable memory-only variant (on macOS, shm objects have no filesystem
/// path at all).
pub struct MemTable {
    backing: Backing,
}

impl MemTable {
    pub fn required_size(schema: &Schema, chunk_size: usize, num_chunks: usize) -> usize {
        compute_data_offset(schema.cols.len()) + chunk_size * num_chunks
    }

    /// Create a **heap-backed** (process-private) table.
    pub fn new(schema: &Schema, chunk_size: u32, num_chunks: u32) -> Self {
        let size = Self::required_size(schema, chunk_size as usize, num_chunks as usize);
        let mut buf = vec![0u8; size];
        init_buf(&mut buf, schema, chunk_size, num_chunks);
        Self {
            backing: Backing::Heap(buf),
        }
    }

    /// Adopt an existing heap buffer (validates the MEMT layout).
    pub fn from_buf(buf: Vec<u8>) -> Result<Self, crate::error::MemtableError> {
        validate_buf(&buf)?;
        Ok(Self {
            backing: Backing::Heap(buf),
        })
    }

    // ── POSIX shared memory (memory-only) ────────────────────────────

    /// Create a **POSIX shared-memory** table (`shm_open`).
    ///
    /// Memory-only: never hits disk, vanishes on reboot. Other processes
    /// attach with [`open_shm`](Self::open_shm) using the same `name`
    /// (normalised to a leading `/`; keep it short — macOS caps shm names
    /// at 31 bytes). The creator unlinks the name on drop; attached
    /// processes keep a valid mapping until they unmap.
    ///
    /// Fails with `AlreadyExists` if the name is taken.
    #[cfg(unix)]
    pub fn shm(name: &str, schema: &Schema, chunk_size: u32, num_chunks: u32) -> io::Result<Self> {
        let cname = shm_name_cstring(name)?;
        let size = Self::required_size(schema, chunk_size as usize, num_chunks as usize);

        let file = shm_open_file(&cname, libc::O_CREAT | libc::O_EXCL | libc::O_RDWR)?;
        file.set_len(size as u64)?;

        let mut mmap = unsafe { MmapMut::map_mut(&file)? };
        init_buf(&mut mmap, schema, chunk_size, num_chunks);

        Ok(Self {
            backing: Backing::Shm {
                mmap,
                name: cstring_to_shm_name(cname)?,
                unlink_on_drop: true,
            },
        })
    }

    /// Attach to an existing POSIX shared-memory table created by
    /// [`shm`](Self::shm) (validates the MEMT layout).
    ///
    /// The returned handle does **not** unlink the name on drop.
    #[cfg(unix)]
    pub fn open_shm(name: &str) -> io::Result<Self> {
        let cname = shm_name_cstring(name)?;
        let file = shm_open_file(&cname, libc::O_RDWR)?;

        let mmap = unsafe { MmapMut::map_mut(&file)? };
        validate_buf(&mmap)?;

        Ok(Self {
            backing: Backing::Shm {
                mmap,
                name: cstring_to_shm_name(cname)?,
                unlink_on_drop: false,
            },
        })
    }

    // ── mmap'd file (disk-backed, persistent) ────────────────────────

    /// Create a table backed by an **mmap'd regular file** at `path`.
    ///
    /// Disk-backed and persistent: the file is **kept** on drop and can be
    /// reopened later with [`open_file`](Self::open_file) — including
    /// after a process crash or reboot. Truncates any existing file.
    pub fn file_at(
        path: impl AsRef<Path>,
        schema: &Schema,
        chunk_size: u32,
        num_chunks: u32,
    ) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let size = Self::required_size(schema, chunk_size as usize, num_chunks as usize);

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        file.set_len(size as u64)?;

        let mut mmap = unsafe { MmapMut::map_mut(&file)? };
        init_buf(&mut mmap, schema, chunk_size, num_chunks);

        Ok(Self {
            backing: Backing::File {
                mmap,
                path,
                dir: None,
                unlink_on_drop: false,
            },
        })
    }

    /// Reopen an existing mmap'd-file table read-write (validates the
    /// MEMT layout). The file is kept on drop.
    pub fn open_file(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new().read(true).write(true).open(&path)?;

        let mmap = unsafe { MmapMut::map_mut(&file)? };
        validate_buf(&mmap)?;

        Ok(Self {
            backing: Backing::File {
                mmap,
                path,
                dir: None,
                unlink_on_drop: false,
            },
        })
    }

    // ── discoverable file (data-dir convention) ──────────────────────

    /// Create a **discoverable** mmap'd-file table in the
    /// [`default_dir`](crate::discover::default_dir), at
    /// `<data_dir>/<pid>/<name>`.
    ///
    /// This is the flavour the SQL catalog and cross-process discovery
    /// scan for. On Linux the default dir is `/dev/shm` (tmpfs), making
    /// this shared memory with a browsable path. The file is unlinked on
    /// drop; after a crash it stays readable until stale-pid cleanup.
    pub fn shared(
        name: &str,
        schema: &Schema,
        chunk_size: u32,
        num_chunks: u32,
    ) -> crate::error::Result<Self> {
        Self::shared_in(
            &crate::discover::default_dir(),
            name,
            schema,
            chunk_size,
            num_chunks,
        )
    }

    /// Like [`shared`](Self::shared), under a custom base directory
    /// (file at `<base_dir>/<pid>/<name>`).
    pub fn shared_in(
        base_dir: &Path,
        name: &str,
        schema: &Schema,
        chunk_size: u32,
        num_chunks: u32,
    ) -> crate::error::Result<Self> {
        let dir = base_dir.join(std::process::id().to_string());
        std::fs::create_dir_all(&dir)?;

        let path = dir.join(name);
        let size = Self::required_size(schema, chunk_size as usize, num_chunks as usize);

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        file.set_len(size as u64)?;

        let mut mmap = unsafe { MmapMut::map_mut(&file)? };
        init_buf(&mut mmap, schema, chunk_size, num_chunks);

        Ok(Self {
            backing: Backing::File {
                mmap,
                path,
                dir: Some(dir),
                unlink_on_drop: true,
            },
        })
    }

    // ── backing introspection ─────────────────────────────────────────

    /// Which backend stores this table.
    pub fn backing_kind(&self) -> BackingKind {
        match &self.backing {
            Backing::Heap(_) => BackingKind::Heap,
            #[cfg(unix)]
            Backing::Shm { .. } => BackingKind::Shm,
            Backing::File { .. } => BackingKind::File,
        }
    }

    /// `true` when other processes can attach (shm or mmap'd file).
    pub fn is_shared(&self) -> bool {
        match &self.backing {
            Backing::Heap(_) => false,
            #[cfg(unix)]
            Backing::Shm { .. } => true,
            Backing::File { .. } => true,
        }
    }

    /// File path of the mapping; [`None`] for heap and shm backings
    /// (POSIX shm objects have no portable filesystem path).
    pub fn path(&self) -> Option<&Path> {
        match &self.backing {
            Backing::File { path, .. } => Some(path),
            _ => None,
        }
    }

    /// POSIX shm name (with leading `/`); [`None`] for other backings.
    pub fn shm_name(&self) -> Option<&str> {
        match &self.backing {
            #[cfg(unix)]
            Backing::Shm { name, .. } => Some(name),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.backing.bytes()
    }

    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        self.backing.bytes_mut()
    }

    pub fn view(&self) -> MemTableView<'_> {
        MemTableView {
            buf: self.backing.bytes(),
        }
    }

    impl_table_reader!();

    pub fn row_writer(&mut self) -> RowWriter<'_> {
        make_row_writer(self.backing.bytes_mut(), None)
    }
    pub fn append_row(&mut self, values: &[Value]) -> bool {
        if !validate_row_schema(self.backing.bytes(), values) {
            log::warn!("append_row: value types do not match schema");
            return false;
        }
        write_row_bytes(self.backing.bytes_mut(), values, row_data_size(values))
    }
    pub fn advance_chunk(&mut self) {
        advance_chunk_raw(self.backing.bytes_mut())
    }

    /// Append a row, auto-advancing to the next chunk when full.
    ///
    /// MEMT is single-writer: the `&mut self` borrow guarantees exclusive
    /// access, so no lock is taken.
    pub fn push_row(&mut self, values: &[Value]) -> bool {
        if !validate_row_schema(self.backing.bytes(), values) {
            log::warn!("push_row: value types do not match schema");
            return false;
        }
        self.push_row_unchecked(values)
    }
    pub fn push_row_unchecked(&mut self, values: &[Value]) -> bool {
        push_plain_row(self.backing.bytes_mut(), values)
    }
}

impl Drop for MemTable {
    fn drop(&mut self) {
        match &self.backing {
            Backing::Heap(_) => {}
            #[cfg(unix)]
            Backing::Shm {
                name,
                unlink_on_drop: true,
                ..
            } => {
                if let Ok(cname) = std::ffi::CString::new(name.as_str()) {
                    unsafe { libc::shm_unlink(cname.as_ptr()) };
                }
            }
            #[cfg(unix)]
            Backing::Shm { .. } => {}
            Backing::File {
                path,
                dir,
                unlink_on_drop: true,
                ..
            } => {
                let _ = std::fs::remove_file(path);
                if let Some(dir) = dir {
                    let _ = std::fs::remove_dir(dir); // succeeds only if empty
                }
            }
            Backing::File { .. } => {}
        }
    }
}

impl fmt::Display for MemTable {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let kind = match self.backing_kind() {
            BackingKind::Heap => "heap",
            #[cfg(unix)]
            BackingKind::Shm => "shm",
            BackingKind::File => "file",
        };
        write!(
            f,
            "MemTable({kind}, {} cols, {} chunks × {} bytes)",
            self.num_cols(),
            self.num_chunks(),
            self.chunk_size()
        )
    }
}

// ── MemTableView (borrowed, read-only) ───────────────────────────────

pub struct MemTableView<'a> {
    buf: &'a [u8],
}

impl<'a> MemTableView<'a> {
    pub fn new(buf: &'a [u8]) -> Result<Self, crate::error::MemtableError> {
        validate_buf(buf)?;
        Ok(Self { buf })
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.buf
    }

    impl_table_reader!();
}

impl fmt::Display for MemTableView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "MemTableView({} cols, {} chunks × {} bytes)",
            self.num_cols(),
            self.num_chunks(),
            self.chunk_size()
        )
    }
}

// ── MemTableWriter (borrowed, configurable write modes) ──────────────

/// Unified writer for external buffers (`&mut [u8]`).
///
/// MEMT is single-writer: the `&mut [u8]` borrow guarantees exclusive
/// access, so no lock is taken. Two modes via builder methods:
///
/// | Mode | Construction |
/// |------|-------------|
/// | Plain | `MemTableWriter::new(buf)?` |
/// | Dedup | `MemTableWriter::new(buf)?.dedup()` |
///
/// **Dedup**: per-chunk, hash-based string/bytes dedup.  Repeated values
/// are stored as 4-byte back-references within the same chunk.
pub struct MemTableWriter<'a> {
    buf: &'a mut [u8],
    dedup: Option<DedupState>,
}

impl<'a> MemTableWriter<'a> {
    pub fn new(buf: &'a mut [u8]) -> Result<Self, crate::error::MemtableError> {
        validate_buf(buf)?;
        Ok(Self { buf, dedup: None })
    }

    pub fn init(buf: &'a mut [u8], schema: &Schema, chunk_size: u32, num_chunks: u32) -> Self {
        init_buf(buf, schema, chunk_size, num_chunks);
        Self { buf, dedup: None }
    }

    /// Enable per-chunk string/bytes dedup.  Sets `FLAG_DEDUP` in header.
    pub fn dedup(mut self) -> Self {
        header_mut(self.buf).flags |= FLAG_DEDUP;
        self.dedup = Some(DedupState::new());
        self
    }

    pub fn set_min_dedup_len(&mut self, len: usize) {
        if let Some(ref mut s) = self.dedup {
            s.set_min_dedup_len(len);
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.buf
    }
    pub fn view(&self) -> MemTableView<'_> {
        MemTableView { buf: self.buf }
    }

    impl_table_reader!();

    pub fn row_writer(&mut self) -> RowWriter<'_> {
        make_row_writer(self.buf, self.dedup.as_mut())
    }

    pub fn push_row(&mut self, values: &[Value]) -> bool {
        if !validate_row_schema(self.buf, values) {
            log::warn!("push_row: value types do not match schema");
            return false;
        }
        self.push_inner(values)
    }

    pub fn push_row_unchecked(&mut self, values: &[Value]) -> bool {
        self.push_inner(values)
    }

    pub fn advance_chunk(&mut self) {
        advance_chunk_raw(self.buf);
        if let Some(ref mut s) = self.dedup {
            s.clear();
        }
    }

    pub fn append_row(&mut self, values: &[Value]) -> bool {
        if !validate_row_schema(self.buf, values) {
            log::warn!("append_row: value types do not match schema");
            return false;
        }
        if let Some(ref mut state) = self.dedup {
            append_row_dedup_bytes(self.buf, state, values)
        } else {
            write_row_bytes(self.buf, values, row_data_size(values))
        }
    }

    fn push_inner(&mut self, values: &[Value]) -> bool {
        if let Some(ref mut state) = self.dedup {
            if append_row_dedup_bytes(self.buf, state, values) {
                return true;
            }
            advance_chunk_raw(self.buf);
            state.clear();
            if append_row_dedup_bytes(self.buf, state, values) {
                return true;
            }
            log::warn!("push_row: row exceeds chunk capacity");
            false
        } else {
            push_plain_row(self.buf, values)
        }
    }
}

impl fmt::Display for MemTableWriter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mode = if self.dedup.is_some() {
            "dedup"
        } else {
            "plain"
        };
        write!(
            f,
            "MemTableWriter({} cols, {} chunks × {} bytes, {mode})",
            self.num_cols(),
            self.num_chunks(),
            self.chunk_size()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{BackingKind, MemTable, MemTableView, MemTableWriter};
    use crate::layout::{col_desc, header, header_mut, MAGIC, VERSION};
    use crate::raw::init_buf;
    use crate::refcount::{acquire_ref, refcount, release_ref};
    use crate::schema::{DType, Schema, Value};
    use std::sync::atomic::Ordering;

    #[test]
    fn create_and_read_schema() {
        let schema = Schema::new()
            .col("ts", DType::I64)
            .col("value", DType::F64)
            .col("tag", DType::I32);
        let t = MemTable::new(&schema, 4096, 4);
        assert_eq!(t.num_cols(), 3);
        assert_eq!(t.num_chunks(), 4);
        assert_eq!(t.chunk_size(), 4096);
        assert_eq!(t.col_name(0), "ts");
        assert_eq!(t.col_dtype(0), Some(DType::I64));
    }

    #[test]
    fn schema_reconstruct() {
        let schema = Schema::new().col("a", DType::I32).col("b", DType::F64);
        let t = MemTable::new(&schema, 1024, 1);
        let s = t.schema();
        assert_eq!(s.cols.len(), 2);
        assert_eq!(s.cols[0].name, "a");
        assert_eq!(s.cols[1].dtype, DType::F64);
    }

    #[test]
    fn write_and_read_fixed_row() {
        let schema = Schema::new().col("id", DType::I64).col("val", DType::F64);
        let mut t = MemTable::new(&schema, 1024, 1);
        t.push_row(&[Value::I64(42), Value::F64(3.14)]);
        t.push_row(&[Value::I64(100), Value::F64(2.72)]);
        let rows: Vec<_> = t.rows(0).collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].col_i64(0), 42);
        assert_eq!(rows[0].col_f64(1), 3.14);
        assert_eq!(rows[1].col_i64(0), 100);
    }

    #[test]
    fn write_and_read_str_column() {
        let schema = Schema::new().col("ts", DType::I64).col("msg", DType::Str);
        let mut t = MemTable::new(&schema, 4096, 1);
        t.push_row(&[Value::I64(1000), Value::Str("hello world")]);
        t.push_row(&[Value::I64(2000), Value::Str("")]);
        t.push_row(&[Value::I64(3000), Value::Str("foo")]);
        let rows: Vec<_> = t.rows(0).collect();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].col_str(1), "hello world");
        assert_eq!(rows[1].col_str(1), "");
        assert_eq!(rows[2].col_str(1), "foo");
    }

    #[test]
    fn bytes_column() {
        let schema = Schema::new().col("data", DType::Bytes);
        let mut t = MemTable::new(&schema, 1024, 1);
        t.push_row(&[Value::Bytes(&[0xDE, 0xAD, 0xBE, 0xEF])]);
        t.push_row(&[Value::Bytes(&[])]);
        let rows: Vec<_> = t.rows(0).collect();
        assert_eq!(rows[0].col_bytes(0), &[0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(rows[1].col_bytes(0), &[]);
    }

    #[test]
    fn u32_column() {
        let schema = Schema::new().col("x", DType::U32);
        let mut t = MemTable::new(&schema, 1024, 1);
        t.push_row(&[Value::U32(0xDEAD_BEEF)]);
        assert_eq!(t.rows(0).next().unwrap().col_u32(0), 0xDEAD_BEEF);
    }

    #[test]
    fn mixed_fixed_and_variable() {
        let schema = Schema::new()
            .col("id", DType::U64)
            .col("name", DType::Str)
            .col("value", DType::F64)
            .col("payload", DType::Bytes);
        let mut t = MemTable::new(&schema, 4096, 1);
        t.push_row(&[
            Value::U64(42),
            Value::Str("test_event"),
            Value::F64(3.14),
            Value::Bytes(&[1, 2, 3]),
        ]);
        let row = t.rows(0).next().unwrap();
        assert_eq!(row.col_u64(0), 42);
        assert_eq!(row.col_str(1), "test_event");
        assert_eq!(row.col_f64(2), 3.14);
        assert_eq!(row.col_bytes(3), &[1, 2, 3]);
    }

    #[test]
    fn variable_length_rows() {
        let schema = Schema::new().col("msg", DType::Str);
        let mut t = MemTable::new(&schema, 4096, 1);
        t.push_row(&[Value::Str("short")]);
        t.push_row(&[Value::Str("a much longer string that takes more space")]);
        t.push_row(&[Value::Str("x")]);
        let rows: Vec<_> = t.rows(0).collect();
        assert_eq!(rows.len(), 3);
        assert_ne!(rows[0].as_bytes().len(), rows[1].as_bytes().len());
    }

    #[test]
    fn chunk_used_tracking() {
        let schema = Schema::new().col("x", DType::I32);
        let mut t = MemTable::new(&schema, 1024, 2);
        assert_eq!(t.chunk_used(0), 0);
        t.push_row(&[Value::I32(1)]);
        assert_eq!(t.chunk_used(0), 8); // 4 (row_len) + 4 (i32)
        t.push_row(&[Value::I32(2)]);
        assert_eq!(t.chunk_used(0), 16);
    }

    #[test]
    fn append_row_returns_false_when_full() {
        let schema = Schema::new().col("x", DType::I64);
        // ChunkHeader=40, each I64 row=12 → 64-40=24 data bytes → 2 rows fit
        let mut t = MemTable::new(&schema, 64, 1);
        assert!(t.append_row(&[Value::I64(1)]));
        assert!(t.append_row(&[Value::I64(2)]));
        assert!(!t.append_row(&[Value::I64(3)]));
        assert_eq!(t.num_rows(0), 2);
    }

    #[test]
    fn ring_buffer_wrap() {
        let schema = Schema::new().col("v", DType::I32);
        // ChunkHeader=40, each I32 row=8 → 96-40=56 data bytes → 7 rows fit
        let mut t = MemTable::new(&schema, 96, 3);
        for i in 0..7 {
            t.push_row(&[Value::I32(i)]);
        }
        assert_eq!(t.write_chunk(), 0);
        assert_eq!(t.num_rows(0), 7);
        t.push_row(&[Value::I32(100)]);
        assert_eq!(t.write_chunk(), 1);
        for i in 0..14 {
            t.push_row(&[Value::I32(200 + i)]);
        }
        assert_eq!(t.write_chunk(), 0);
        assert_eq!(t.rows(0).next().unwrap().col_i32(0), 213);
    }

    #[test]
    fn heap_backing_is_private() {
        let schema = Schema::new().col("x", DType::I32);
        let mut t = MemTable::new(&schema, 1024, 2);
        assert!(!t.is_shared());
        assert_eq!(t.backing_kind(), BackingKind::Heap);
        assert!(t.path().is_none());
        assert!(t.shm_name().is_none());
        t.push_row(&[Value::I32(7)]);
        assert_eq!(t.rows(0).next().unwrap().col_i32(0), 7);
    }

    #[test]
    #[cfg(unix)]
    fn shm_backing_roundtrip_and_unlink() {
        // Short name: macOS caps shm names at 31 bytes.
        let name = format!("/pbg_t{}", std::process::id() % 1_000_000);
        // In case a previous failed run leaked the name.
        if let Ok(c) = std::ffi::CString::new(name.as_str()) {
            unsafe { libc::shm_unlink(c.as_ptr()) };
        }

        let schema = Schema::new().col("ts", DType::I64).col("msg", DType::Str);
        let mut creator = MemTable::shm(&name, &schema, 4096, 2).unwrap();
        assert_eq!(creator.backing_kind(), BackingKind::Shm);
        assert!(creator.is_shared());
        assert!(creator.path().is_none());
        assert_eq!(creator.shm_name(), Some(name.as_str()));

        creator.push_row(&[Value::I64(1), Value::Str("alpha")]);

        // Second attachment (what another process would do) sees the data…
        let mut attached = MemTable::open_shm(&name).unwrap();
        assert_eq!(attached.num_rows(0), 1);
        assert_eq!(attached.rows(0).next().unwrap().col_str(1), "alpha");

        // …and writes through it are visible to the creator (same memory).
        attached.push_row(&[Value::I64(2), Value::Str("beta")]);
        assert_eq!(creator.num_rows(0), 2);

        // Name collision is rejected.
        assert!(MemTable::shm(&name, &schema, 4096, 2).is_err());

        // Creator drop unlinks the name; the attached mapping stays valid.
        drop(creator);
        assert!(MemTable::open_shm(&name).is_err());
        assert_eq!(attached.num_rows(0), 2);
    }

    #[test]
    fn file_backing_persists_across_reopen() {
        let dir = std::env::temp_dir().join(format!(
            "probing_mt_file_test_{}_{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("persistent.mt");

        let schema = Schema::new().col("v", DType::I64);
        {
            let mut t = MemTable::file_at(&path, &schema, 4096, 2).unwrap();
            assert_eq!(t.backing_kind(), BackingKind::File);
            assert_eq!(t.path(), Some(path.as_path()));
            t.push_row(&[Value::I64(42)]);
        }
        // Unlike `shared`, the file survives drop…
        assert!(path.is_file());

        // …and can be reopened read-write with data intact.
        let mut t = MemTable::open_file(&path).unwrap();
        assert_eq!(t.num_rows(0), 1);
        assert_eq!(t.rows(0).next().unwrap().col_i64(0), 42);
        t.push_row(&[Value::I64(43)]);
        assert_eq!(t.num_rows(0), 2);

        // Reopening garbage fails validation.
        let bad = dir.join("garbage.mt");
        std::fs::write(&bad, vec![0u8; 256]).unwrap();
        assert!(MemTable::open_file(&bad).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shared_backing_roundtrip_and_cleanup() {
        let base = std::env::temp_dir().join(format!(
            "probing_mt_shared_test_{}_{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&base);

        let schema = Schema::new().col("ts", DType::I64).col("msg", DType::Str);
        let path = {
            let mut t = MemTable::shared_in(&base, "shm_tbl", &schema, 4096, 2).unwrap();
            assert!(t.is_shared());
            let path = t.path().unwrap().to_path_buf();
            assert!(path.is_file());

            t.push_row(&[Value::I64(1), Value::Str("alpha")]);
            t.push_row(&[Value::I64(2), Value::Str("beta")]);

            // Same write/read API as the heap backing
            assert_eq!(t.num_rows(0), 2);
            assert_eq!(t.chunks_logical(), vec![0]);

            // Another handle (separate mmap of the same file) sees the data —
            // this is what a cross-process reader does.
            let bytes = std::fs::read(&path).unwrap();
            let view = MemTableView::new(&bytes).unwrap();
            assert_eq!(view.num_rows(0), 2);
            let row = view.rows(0).next().unwrap();
            assert_eq!(row.col_i64(0), 1);
            assert_eq!(row.col_str(1), "alpha");

            path
        };
        // Drop unlinks the file and the (now empty) <pid>/ directory.
        assert!(!path.exists());
        assert!(!path.parent().unwrap().exists());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn shared_and_heap_share_write_semantics_across_wrap() {
        let base = std::env::temp_dir().join(format!(
            "probing_mt_shared_test_{}_{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&base);

        let schema = Schema::new().col("v", DType::I32);
        let mut heap = MemTable::new(&schema, 80, 3);
        let mut shm = MemTable::shared_in(&base, "wrap_tbl", &schema, 80, 3).unwrap();

        for i in 0..20 {
            heap.push_row(&[Value::I32(i)]);
            shm.push_row(&[Value::I32(i)]);
        }

        let collect = |t: &MemTable| -> Vec<i32> {
            t.chunks_logical()
                .into_iter()
                .flat_map(|c| t.rows(c).map(|r| r.col_i32(0)).collect::<Vec<_>>())
                .collect()
        };
        assert_eq!(collect(&heap), collect(&shm));

        drop(shm);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn chunks_logical_pre_wrap() {
        let schema = Schema::new().col("v", DType::I32);
        let mut t = MemTable::new(&schema, 80, 3);
        // No data yet: only chunk 0 is Writing (gen 1) but has no rows
        assert!(t.chunks_logical().is_empty());

        t.push_row(&[Value::I32(1)]);
        assert_eq!(t.chunks_logical(), vec![0]);

        t.advance_chunk();
        t.push_row(&[Value::I32(2)]);
        assert_eq!(t.chunks_logical(), vec![0, 1]);
    }

    #[test]
    fn chunks_logical_post_wrap() {
        let schema = Schema::new().col("v", DType::I32);
        let mut t = MemTable::new(&schema, 80, 2);
        t.push_row(&[Value::I32(10)]); // chunk 0, gen 1
        t.advance_chunk();
        t.push_row(&[Value::I32(20)]); // chunk 1, gen 1
        t.advance_chunk(); // wraps: chunk 0 recycled → gen 2, zeroed
        t.push_row(&[Value::I32(30)]); // chunk 0, gen 2

        // Logical order: oldest surviving data (chunk 1, gen 1) first,
        // then the recycled chunk 0 (gen 2).
        let order = t.chunks_logical();
        assert_eq!(order, vec![1, 0]);

        let values: Vec<i32> = order
            .iter()
            .flat_map(|&c| t.rows(c).map(|r| r.col_i32(0)).collect::<Vec<_>>())
            .collect();
        assert_eq!(values, vec![20, 30]);
    }

    #[test]
    fn ring_buffer_with_str() {
        let schema = Schema::new().col("msg", DType::Str);
        let mut t = MemTable::new(&schema, 256, 2);
        for msg in &["alpha", "beta", "gamma", "delta"] {
            t.push_row(&[Value::Str(msg)]);
        }
        assert_eq!(t.rows(0).next().unwrap().col_str(0), "alpha");
    }

    #[test]
    fn view_from_bytes() {
        let schema = Schema::new().col("x", DType::I32).col("s", DType::Str);
        let mut t = MemTable::new(&schema, 4096, 1);
        t.push_row(&[Value::I32(99), Value::Str("view_test")]);
        let view = MemTableView::new(t.as_bytes()).unwrap();
        assert_eq!(view.num_cols(), 2);
        let row = view.rows(0).next().unwrap();
        assert_eq!(row.col_i32(0), 99);
        assert_eq!(row.col_str(1), "view_test");
    }

    #[test]
    fn mut_view_on_external_buf() {
        let schema = Schema::new().col("a", DType::I64).col("b", DType::Str);
        let size = MemTable::required_size(&schema, 4096, 2);
        let mut buf = vec![0u8; size];
        let mut w = MemTableWriter::init(&mut buf, &schema, 4096, 2);
        w.push_row(&[Value::I64(42), Value::Str("ext_test")]);
        let row = w.rows(0).next().unwrap();
        assert_eq!(row.col_i64(0), 42);
        assert_eq!(row.col_str(1), "ext_test");
    }

    #[test]
    fn num_rows_count() {
        let schema = Schema::new().col("x", DType::I32);
        let mut t = MemTable::new(&schema, 4096, 1);
        for i in 0..10 {
            t.push_row(&[Value::I32(i)]);
        }
        assert_eq!(t.num_rows(0), 10);
    }

    #[test]
    fn required_size_calculation() {
        let schema = Schema::new().col("a", DType::I64).col("b", DType::F32);
        let size = MemTable::required_size(&schema, 4096, 4);
        assert_eq!(size, 192 + 4096 * 4);
    }

    #[test]
    fn display_format() {
        let schema = Schema::new().col("a", DType::I32);
        let t = MemTable::new(&schema, 1024, 2);
        assert_eq!(
            format!("{t}"),
            "MemTable(heap, 1 cols, 2 chunks × 1024 bytes)"
        );
    }

    #[test]
    fn header_direct_access() {
        use crate::layout::{BYTE_ORDER_MARK, FLAGS_KNOWN};
        let schema = Schema::new().col("x", DType::I32);
        let t = MemTable::new(&schema, 1024, 4);
        let h = header(t.as_bytes());
        assert_eq!(h.magic, MAGIC);
        assert_eq!(h.version, VERSION);
        assert_eq!(
            h.header_size as usize,
            std::mem::size_of::<crate::layout::Header>()
        );
        assert_eq!(h.byte_order, u16::from_ne_bytes(BYTE_ORDER_MARK));
        assert_eq!(h.flags & !FLAGS_KNOWN, 0);
        assert_eq!(h.num_cols, 1);
        assert_eq!(h.num_chunks, 4);
        assert_eq!(h.chunk_size, 1024);
        assert_eq!(h.write_chunk.load(Ordering::Relaxed), 0);
        assert_eq!(h.refcount.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn col_desc_direct_access() {
        let schema = Schema::new()
            .col("count", DType::U32)
            .col("tag", DType::Str);
        let t = MemTable::new(&schema, 1024, 1);
        let cd0 = col_desc(t.as_bytes(), 0);
        assert_eq!(cd0.name_str(), "count");
        assert_eq!(cd0.elem_size, 4);
        let cd1 = col_desc(t.as_bytes(), 1);
        assert_eq!(cd1.name_str(), "tag");
        assert_eq!(cd1.elem_size, 0);
    }

    #[test]
    fn invalid_magic_rejected() {
        let buf = vec![0u8; 64];
        assert!(MemTableView::new(&buf).is_err());
        assert!(MemTable::from_buf(buf).is_err());
    }

    #[test]
    fn empty_chunk_iteration() {
        let schema = Schema::new().col("x", DType::I32);
        let t = MemTable::new(&schema, 1024, 2);
        assert_eq!(t.rows(0).count(), 0);
        assert_eq!(t.rows(1).count(), 0);
    }

    #[test]
    fn expose_register_via_pointer() {
        use std::alloc;

        let schema = Schema::new().col("ts", DType::I64).col("msg", DType::Str);
        let size = MemTable::required_size(&schema, 4096, 4);
        let layout = alloc::Layout::from_size_align(size, 64).unwrap();
        let ptr = unsafe { alloc::alloc_zeroed(layout) };
        assert!(!ptr.is_null());

        // ── Component A: expose (init + write) ──
        unsafe {
            let buf = std::slice::from_raw_parts_mut(ptr, size);
            init_buf(buf, &schema, 4096, 4);
            assert_eq!(refcount(buf), 1);

            let mut producer = MemTableWriter::new(buf).unwrap();
            for i in 0..10i64 {
                producer
                    .row_writer()
                    .put_i64(i * 100)
                    .put_str(&format!("event_{i}"))
                    .finish();
            }
        }

        // ── Component B: register (receives ptr + size, acquires ref) ──
        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            acquire_ref(buf);
            assert_eq!(refcount(buf), 2);

            let consumer = MemTableView::new(buf).unwrap();
            assert_eq!(consumer.num_cols(), 2);
            assert_eq!(consumer.col_name(0), "ts");

            let mut count = 0i64;
            for row in consumer.rows(0) {
                let mut c = row.cursor();
                assert_eq!(c.next_i64(), count * 100);
                assert_eq!(c.next_str(), format!("event_{count}"));
                count += 1;
            }
            assert_eq!(count, 10);

            assert_eq!(release_ref(buf), 1);
        }

        // ── Component A releases last ref ──
        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            assert_eq!(release_ref(buf), 0);
            alloc::dealloc(ptr, layout);
        }
    }

    #[test]
    fn producer_consumer_threaded() {
        use std::alloc;
        use std::thread;

        let schema = Schema::new().col("seq", DType::I64).col("data", DType::Str);
        let size = MemTable::required_size(&schema, 4096, 4);
        let layout = alloc::Layout::from_size_align(size, 64).unwrap();
        let ptr = unsafe { alloc::alloc_zeroed(layout) };
        assert!(!ptr.is_null());

        // init
        unsafe {
            let buf = std::slice::from_raw_parts_mut(ptr, size);
            init_buf(buf, &schema, 4096, 4);
        }
        // acquire ref for the consumer before spawning
        unsafe { acquire_ref(std::slice::from_raw_parts(ptr, size)) };

        let addr = ptr as usize;
        let producer = thread::spawn(move || {
            let buf = unsafe { std::slice::from_raw_parts_mut(addr as *mut u8, size) };
            let mut mt = MemTableWriter::new(buf).unwrap();
            for i in 0..50i64 {
                mt.push_row(&[Value::I64(i), Value::Str("msg")]);
            }
            release_ref(buf);
        });

        producer.join().unwrap();

        // consumer reads after producer is done
        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            let view = MemTableView::new(buf).unwrap();
            let total: usize = (0..view.num_chunks()).map(|c| view.num_rows(c)).sum();
            assert_eq!(total, 50);

            // verify data
            let mut c = view.rows(0).next().unwrap().cursor();
            assert_eq!(c.next_i64(), 0);
            assert_eq!(c.next_str(), "msg");

            let remaining = release_ref(buf);
            assert_eq!(remaining, 0);
            alloc::dealloc(ptr, layout);
        }
    }

    #[test]
    fn single_writer_concurrent_readers() {
        use std::alloc;
        use std::sync::atomic::{AtomicBool, AtomicUsize};
        use std::sync::{Arc, Barrier};
        use std::thread;

        use std::time::Duration;

        // The production model: one writer feeds the ring while N lock-free
        // readers continuously scan it. Readers must never observe a torn or
        // corrupt row.
        let schema = Schema::new().col("val", DType::I64);
        let chunk_size = 4096u32;
        let num_chunks = 4u32;
        let size = MemTable::required_size(&schema, chunk_size as usize, num_chunks as usize);
        let layout = alloc::Layout::from_size_align(size, 64).unwrap();
        let ptr = unsafe { alloc::alloc_zeroed(layout) };
        assert!(!ptr.is_null());

        unsafe {
            let buf = std::slice::from_raw_parts_mut(ptr, size);
            init_buf(buf, &schema, chunk_size, num_chunks);
        }

        let total_rows = 400i64;
        let num_readers = 4;
        let addr = ptr as usize;
        let done = Arc::new(AtomicBool::new(false));
        let total_reads = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(1 + num_readers));

        // Readers continuously scan all chunks while the writer is active.
        let reader_handles: Vec<_> = (0..num_readers)
            .map(|_| {
                let done = done.clone();
                let total_reads = total_reads.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let mut local_reads = 0usize;
                    while !done.load(Ordering::Acquire) {
                        let buf = unsafe { std::slice::from_raw_parts(addr as *const u8, size) };
                        let view = MemTableView::new(buf).unwrap();
                        for chunk in 0..view.num_chunks() {
                            for row in view.rows(chunk) {
                                let mut c = row.cursor();
                                let v = c.next_i64();
                                assert!(v >= 0, "read corrupt value: {v}");
                                local_reads += 1;
                            }
                        }
                        thread::yield_now();
                    }
                    total_reads.fetch_add(local_reads, Ordering::Relaxed);
                })
            })
            .collect();

        let writer = {
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                let buf = unsafe { std::slice::from_raw_parts_mut(addr as *mut u8, size) };
                let mut mt = MemTableWriter::new(buf).unwrap();
                for seq in 0..total_rows {
                    mt.push_row(&[Value::I64(seq)]);
                    if seq % 32 == 0 {
                        thread::yield_now();
                    }
                }
            })
        };

        writer.join().unwrap();
        // Let readers observe completed rows before they exit the scan loop.
        thread::sleep(Duration::from_millis(20));
        done.store(true, Ordering::Release);

        for h in reader_handles {
            h.join().unwrap();
        }

        assert!(
            total_reads.load(Ordering::Relaxed) > 0,
            "readers should have observed rows"
        );

        // Final consistency: every written row is present.
        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            let view = MemTableView::new(buf).unwrap();
            let total: usize = (0..view.num_chunks()).map(|c| view.num_rows(c)).sum();
            assert_eq!(total, total_rows as usize);
            alloc::dealloc(ptr, layout);
        }
    }

    #[test]
    fn version_field_set() {
        let schema = Schema::new().col("x", DType::I32);
        let t = MemTable::new(&schema, 1024, 1);
        assert_eq!(header(t.as_bytes()).version, VERSION);
    }

    // ── chunk header tests ──────────────────────────────────────────

    #[test]
    fn chunk_header_init_state() {
        let schema = Schema::new().col("x", DType::I32);
        let t = MemTable::new(&schema, 1024, 4);
        // Chunk 0 = Writing, generation 1
        assert_eq!(t.chunk_state(0), 1);
        assert_eq!(t.chunk_generation(0), 1);
        assert_eq!(t.num_rows(0), 0);
        // Other chunks = Empty, generation 0
        assert_eq!(t.chunk_state(1), 0);
        assert_eq!(t.chunk_generation(1), 0);
    }

    #[test]
    fn chunk_state_transitions() {
        let schema = Schema::new().col("v", DType::I32);
        let mut t = MemTable::new(&schema, 1024, 3);

        // Write some rows to chunk 0
        t.push_row(&[Value::I32(1)]);
        t.push_row(&[Value::I32(2)]);
        assert_eq!(t.chunk_state(0), 1);
        assert_eq!(t.num_rows(0), 2);

        // Advance: chunk 0 → Sealed, chunk 1 → Writing
        t.advance_chunk();
        assert_eq!(t.chunk_state(0), 2);
        assert_eq!(t.chunk_state(1), 1);
        assert_eq!(t.chunk_generation(1), 1);
    }

    #[test]
    fn chunk_generation_increments_on_wrap() {
        let schema = Schema::new().col("v", DType::I32);
        // 96 bytes per chunk → 7 I32 rows per chunk (ChunkHeader=40)
        let mut t = MemTable::new(&schema, 96, 2);

        assert_eq!(t.chunk_generation(0), 1);
        assert_eq!(t.chunk_generation(1), 0);

        // Fill chunk 0 (7 rows), then push_row triggers advance to chunk 1
        for i in 0..8 {
            t.push_row(&[Value::I32(i)]);
        }
        assert_eq!(t.write_chunk(), 1);
        assert_eq!(t.chunk_generation(1), 1);

        // Fill chunk 1 (7 rows), then advance back to chunk 0
        for i in 0..8 {
            t.push_row(&[Value::I32(100 + i)]);
        }
        assert_eq!(t.write_chunk(), 0);
        // Chunk 0 was recycled: generation bumped from 1 to 2
        assert_eq!(t.chunk_generation(0), 2);
        assert_eq!(t.chunk_state(0), 1);
    }

    #[test]
    fn num_rows_matches_iteration() {
        let schema = Schema::new().col("id", DType::I64).col("msg", DType::Str);
        let mut t = MemTable::new(&schema, 4096, 2);
        for i in 0..20i64 {
            t.push_row(&[Value::I64(i), Value::Str("hello")]);
        }
        assert_eq!(t.num_rows(0), t.rows(0).count());
    }

    #[test]
    fn stress_dedup_savings_measurement() {
        let schema = Schema::new()
            .col("region", DType::Str)
            .col("service", DType::Str)
            .col("counter", DType::I64);

        let regions = ["us-east-1", "us-west-2", "eu-west-1", "ap-southeast-1"];
        let services = ["gateway", "auth", "billing", "inventory", "shipping"];
        let n = 500;

        // with dedup
        let size = MemTable::required_size(&schema, 65536, 1);
        let mut buf_dedup = vec![0u8; size];
        {
            let mut dw = MemTableWriter::init(&mut buf_dedup, &schema, 65536, 1).dedup();
            for i in 0..n {
                dw.push_row(&[
                    Value::Str(regions[i % regions.len()]),
                    Value::Str(services[i % services.len()]),
                    Value::I64(i as i64),
                ]);
            }
        }
        let dedup_used = {
            let v = MemTableView::new(&buf_dedup).unwrap();
            v.chunk_used(0)
        };

        // without dedup
        let mut buf_plain = vec![0u8; size];
        {
            let mut mt = MemTableWriter::init(&mut buf_plain, &schema, 65536, 1);
            for i in 0..n {
                mt.push_row(&[
                    Value::Str(regions[i % regions.len()]),
                    Value::Str(services[i % services.len()]),
                    Value::I64(i as i64),
                ]);
            }
        }
        let plain_used = {
            let v = MemTableView::new(&buf_plain).unwrap();
            v.chunk_used(0)
        };

        assert!(
            dedup_used < plain_used,
            "dedup should save space: {dedup_used} vs {plain_used}"
        );
        let savings_pct = (1.0 - dedup_used as f64 / plain_used as f64) * 100.0;
        assert!(
            savings_pct > 20.0,
            "expected >20% savings, got {savings_pct:.1}%"
        );

        // both should produce identical logical data
        let v_dedup = MemTableView::new(&buf_dedup).unwrap();
        let v_plain = MemTableView::new(&buf_plain).unwrap();
        assert_eq!(v_dedup.num_rows(0), v_plain.num_rows(0));
        for (rd, rp) in v_dedup.rows(0).zip(v_plain.rows(0)) {
            let mut cd = rd.cursor();
            let mut cp = rp.cursor();
            assert_eq!(cd.next_str(), cp.next_str());
            assert_eq!(cd.next_str(), cp.next_str());
            assert_eq!(cd.next_i64(), cp.next_i64());
        }
    }

    #[test]
    fn from_buf_roundtrip() {
        let schema = Schema::new().col("x", DType::I32);
        let mut t = MemTable::new(&schema, 1024, 1);
        t.push_row(&[Value::I32(7)]);
        let raw = t.as_bytes().to_vec();
        let t2 = MemTable::from_buf(raw).expect("valid buffer");
        assert_eq!(t2.num_rows(0), 1);
        assert_eq!(t2.rows(0).next().unwrap().col_i32(0), 7);
    }

    #[test]
    fn from_buf_rejects_bad_version() {
        let schema = Schema::new().col("x", DType::U32);
        let t = MemTable::new(&schema, 256, 2);
        let mut raw = t.as_bytes().to_vec();
        header_mut(&mut raw).version = 99;
        assert!(MemTable::from_buf(raw).is_err());
    }

    #[test]
    fn from_buf_rejects_bad_data_offset() {
        let schema = Schema::new().col("x", DType::U32);
        let t = MemTable::new(&schema, 256, 2);
        let mut raw = t.as_bytes().to_vec();
        header_mut(&mut raw).data_offset = 7;
        assert!(MemTable::from_buf(raw).is_err());
    }

    #[test]
    fn push_row_rejects_wrong_column_count() {
        let schema = Schema::new().col("a", DType::U32).col("b", DType::I64);
        let mut t = MemTable::new(&schema, 256, 2);
        assert!(!t.push_row(&[Value::U32(1)])); // only 1 value for 2 columns
        assert_eq!(t.num_rows(0), 0);
    }

    #[test]
    fn push_row_rejects_wrong_dtype() {
        let schema = Schema::new().col("a", DType::U32);
        let mut t = MemTable::new(&schema, 256, 2);
        assert!(!t.push_row(&[Value::Str("oops")])); // Str instead of U32
        assert_eq!(t.num_rows(0), 0);
    }

    // ── MemTableWriter tests ──────────────────────────

    #[test]
    fn mem_table_writer_basic() {
        let schema = Schema::new().col("ts", DType::I64).col("val", DType::F64);
        let size = MemTable::required_size(&schema, 4096, 2);
        let mut buf = vec![0u8; size];
        let mut sw = MemTableWriter::init(&mut buf, &schema, 4096, 2);

        sw.push_row(&[Value::I64(100), Value::F64(3.14)]);
        sw.push_row(&[Value::I64(200), Value::F64(2.72)]);

        assert_eq!(sw.num_rows(0), 2);
        let mut rows = sw.rows(0);
        let mut c = rows.next().unwrap().cursor();
        assert_eq!(c.next_i64(), 100);
        assert_eq!(c.next_f64(), 3.14);
    }

    #[test]
    fn mem_table_writer_row_writer() {
        let schema = Schema::new().col("id", DType::I32).col("msg", DType::Str);
        let size = MemTable::required_size(&schema, 4096, 1);
        let mut buf = vec![0u8; size];
        let mut sw = MemTableWriter::init(&mut buf, &schema, 4096, 1);

        sw.row_writer().put_i32(1).put_str("hello").finish();
        sw.row_writer().put_i32(2).put_str("world").finish();

        let rows: Vec<_> = sw.rows(0).collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].col_i32(0), 1);
        assert_eq!(rows[0].col_str(1), "hello");
        assert_eq!(rows[1].col_str(1), "world");
    }

    #[test]
    fn mem_table_writer_dedup() {
        let schema = Schema::new().col("tag", DType::Str).col("seq", DType::I32);
        let size = MemTable::required_size(&schema, 8192, 1);
        let mut buf = vec![0u8; size];
        let mut sw = MemTableWriter::init(&mut buf, &schema, 8192, 1).dedup();

        for i in 0..20 {
            sw.push_row(&[Value::Str("repeat"), Value::I32(i)]);
        }

        let used_dedup = sw.chunk_used(0);

        // Compare with a plain writer
        let mut buf2 = vec![0u8; size];
        let mut sw2 = MemTableWriter::init(&mut buf2, &schema, 8192, 1);
        for i in 0..20 {
            sw2.push_row(&[Value::Str("repeat"), Value::I32(i)]);
        }
        let used_plain = sw2.chunk_used(0);

        assert!(
            used_dedup < used_plain,
            "dedup should save: {used_dedup} vs {used_plain}"
        );

        for (i, row) in sw.rows(0).enumerate() {
            let mut c = row.cursor();
            assert_eq!(c.next_str(), "repeat");
            assert_eq!(c.next_i32(), i as i32);
        }
    }

    #[test]
    fn mem_table_writer_auto_advance() {
        let schema = Schema::new().col("v", DType::I64);
        let size = MemTable::required_size(&schema, 64, 4);
        let mut buf = vec![0u8; size];
        let mut sw = MemTableWriter::init(&mut buf, &schema, 64, 4);

        for i in 0..50i64 {
            sw.push_row_unchecked(&[Value::I64(i)]);
        }

        let mut total = 0;
        for chunk in 0..sw.num_chunks() {
            total += sw.num_rows(chunk);
        }
        assert!(total > 0, "should have rows across chunks");
    }

    // ── designated timestamp column / chunk ts range ──────────────────

    #[test]
    fn ts_col_detection() {
        let t = MemTable::new(
            &Schema::new()
                .col("v", DType::F64)
                .col("timestamp", DType::I64),
            1024,
            1,
        );
        assert_eq!(t.ts_col(), Some(1));

        let t = MemTable::new(&Schema::new().col("ts", DType::I64), 1024, 1);
        assert_eq!(t.ts_col(), Some(0));

        // Wrong dtype or name → no designated column
        let t = MemTable::new(&Schema::new().col("timestamp", DType::F64), 1024, 1);
        assert_eq!(t.ts_col(), None);
        let t = MemTable::new(&Schema::new().col("when", DType::I64), 1024, 1);
        assert_eq!(t.ts_col(), None);
        assert_eq!(t.chunk_ts_range(0), None);
    }

    #[test]
    fn chunk_ts_range_tracks_min_max() {
        let schema = Schema::new().col("ts", DType::I64).col("v", DType::I32);
        let mut t = MemTable::new(&schema, 1024, 2);
        assert_eq!(t.chunk_ts_range(0), None, "empty chunk has no range");

        t.push_row(&[Value::I64(500), Value::I32(1)]);
        t.push_row(&[Value::I64(100), Value::I32(2)]); // out-of-order ts
        t.push_row(&[Value::I64(900), Value::I32(3)]);
        assert_eq!(t.chunk_ts_range(0), Some((100, 900)));

        // Advance: new chunk starts with a fresh range
        t.advance_chunk();
        assert_eq!(t.chunk_ts_range(1), None);
        t.push_row(&[Value::I64(1000), Value::I32(4)]);
        assert_eq!(t.chunk_ts_range(1), Some((1000, 1000)));
        assert_eq!(
            t.chunk_ts_range(0),
            Some((100, 900)),
            "old chunk keeps range"
        );
    }

    #[test]
    fn chunk_ts_range_resets_on_wrap() {
        let schema = Schema::new().col("ts", DType::I64);
        // ChunkHeader=40, I64 row=12 → 64-40=24 → 2 rows per chunk
        let mut t = MemTable::new(&schema, 64, 2);
        t.push_row(&[Value::I64(10)]);
        t.push_row(&[Value::I64(20)]);
        t.push_row(&[Value::I64(30)]); // → chunk 1
        t.push_row(&[Value::I64(40)]);
        t.push_row(&[Value::I64(50)]); // wrap → chunk 0 recycled
        assert_eq!(t.write_chunk(), 0);
        assert_eq!(t.chunk_ts_range(0), Some((50, 50)), "recycled range resets");
        assert_eq!(t.chunk_ts_range(1), Some((30, 40)));
    }

    #[test]
    fn row_writer_maintains_ts_range() {
        let schema = Schema::new()
            .col("timestamp", DType::I64)
            .col("m", DType::Str);
        let mut t = MemTable::new(&schema, 4096, 1);
        t.row_writer().put_i64(300).put_str("a").finish();
        t.row_writer().put_i64(100).put_str("b").finish();
        assert_eq!(t.chunk_ts_range(0), Some((100, 300)));
    }

    #[test]
    fn dedup_writer_maintains_ts_range() {
        let schema = Schema::new().col("ts", DType::I64).col("tag", DType::Str);
        let size = MemTable::required_size(&schema, 4096, 1);
        let mut buf = vec![0u8; size];
        let mut w = MemTableWriter::init(&mut buf, &schema, 4096, 1).dedup();
        w.push_row(&[Value::I64(7), Value::Str("x")]);
        w.push_row(&[Value::I64(3), Value::Str("x")]);
        assert_eq!(w.chunk_ts_range(0), Some((3, 7)));
    }

    #[test]
    fn validate_rejects_bad_ts_col() {
        let schema = Schema::new().col("ts", DType::I64).col("v", DType::F64);
        let mut t = MemTable::new(&schema, 1024, 1);
        header_mut(t.as_bytes_mut()).ts_col = 3; // out of range (2 cols)
        assert!(MemTableView::new(t.as_bytes()).is_err());
        header_mut(t.as_bytes_mut()).ts_col = 2; // col 1 is F64, not I64
        assert!(MemTableView::new(t.as_bytes()).is_err());
        header_mut(t.as_bytes_mut()).ts_col = 1; // col 0 is I64 → ok
        assert!(MemTableView::new(t.as_bytes()).is_ok());
    }
}
