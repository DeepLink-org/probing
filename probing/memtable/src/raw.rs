use crate::error::{MemtableError, Result};
#[cfg(test)]
use crate::layout::compute_data_offset;
use crate::layout::{
    checked_data_offset, checked_lease_offset, chunk_header, col_desc, col_desc_mut, header,
    header_mut, r32, reader_lease, w32, ChunkHeader, ChunkState, Header, BYTE_ORDER_MARK,
    CHUNK_HEADER_SIZE, FLAGS_KNOWN, FLAG_DEDUP, MAGIC, READER_LEASE_SLOTS, TS_MAX_INIT,
    TS_MIN_INIT, VERSION,
};
use crate::row::{chunk_has_live_leases, pin_chunk, try_lock_chunk_for_recycle};
use crate::schema::{DType, Schema, Value};
use std::mem;
use std::sync::atomic::{AtomicU64, Ordering};

/// Column names recognised as the designated timestamp column (must be
/// `I64`). Matched at [`init_buf`] time and recorded in `Header::ts_col`.
pub(crate) const TS_COL_NAMES: [&str; 2] = ["timestamp", "ts"];

/// Kernel PID, deliberately bypassing any process-library cache so lease
/// ownership changes immediately after `fork`.
#[cfg(unix)]
pub(crate) fn current_process_id() -> u32 {
    unsafe { libc::getpid() as u32 }
}

#[cfg(not(unix))]
pub(crate) fn current_process_id() -> u32 {
    std::process::id()
}

/// Fold a committed row's timestamp into the chunk's `min_ts`/`max_ts`.
///
/// Called by the (single, lock-holding) writer **before** the `used`
/// Release store that publishes the row, so any reader that observes the
/// row also observes a covering ts range.
pub(crate) fn note_row_ts(ch: &ChunkHeader, ts: i64) {
    if ts < ch.min_ts.load(Ordering::Relaxed) {
        ch.min_ts.store(ts, Ordering::Relaxed);
    }
    if ts > ch.max_ts.load(Ordering::Relaxed) {
        ch.max_ts.store(ts, Ordering::Relaxed);
    }
}

/// Extract the designated timestamp from a row, per `Header::ts_col`.
#[inline]
pub(crate) fn row_ts(h: &Header, values: &[Value]) -> Option<i64> {
    match h.ts_col as usize {
        0 => None,
        idx => match values.get(idx - 1) {
            Some(Value::I64(ts)) => Some(*ts),
            _ => None,
        },
    }
}

/// Returns the kernel-reported start time of a process.
///
/// Used to populate [`Header::creator_start_time`] and to verify liveness
/// during discovery (detecting PID recycling).
///
/// - **Linux**: clock ticks since boot from `/proc/<pid>/stat` field 22.
/// - **macOS**: microseconds since epoch via `sysctl(KERN_PROC_PID)`.
/// - **Windows**: process creation time from `GetProcessTimes`.
/// - **Other**: returns 0 (graceful degradation to PID-only check).
#[cfg(target_os = "linux")]
pub(crate) fn process_start_time(pid: u32) -> u64 {
    let path = if pid == std::process::id() {
        "/proc/self/stat".to_string()
    } else {
        format!("/proc/{}/stat", pid)
    };
    if let Ok(stat) = std::fs::read_to_string(path) {
        if let Some(pos) = stat.rfind(')') {
            let rest = &stat[pos + 2..];
            if let Some(time_str) = rest.split_whitespace().nth(19) {
                if let Ok(time) = time_str.parse::<u64>() {
                    return time;
                }
            }
        }
    }
    0
}

#[cfg(target_os = "macos")]
pub(crate) fn process_start_time(pid: u32) -> u64 {
    let mut info = unsafe { std::mem::zeroed::<libc::proc_bsdinfo>() };
    let expected = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let written = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            expected,
        )
    };
    if written != expected {
        return 0;
    }
    info.pbi_start_tvsec
        .checked_mul(1_000_000)
        .and_then(|seconds| seconds.checked_add(info.pbi_start_tvusec))
        .unwrap_or(0)
}

#[cfg(windows)]
mod windows_process {
    use std::ffi::c_void;

    pub type Handle = *mut c_void;
    pub const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    pub const ERROR_INVALID_PARAMETER: u32 = 87;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct FileTime {
        pub low: u32,
        pub high: u32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        pub fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        pub fn GetProcessTimes(
            process: Handle,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
        pub fn CloseHandle(handle: Handle) -> i32;
        pub fn GetLastError() -> u32;
    }
}

#[cfg(windows)]
pub(crate) fn process_start_time(pid: u32) -> u64 {
    use windows_process::*;
    if pid == 0 {
        return 0;
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return 0;
    }
    let mut creation = FileTime::default();
    let mut exit = FileTime::default();
    let mut kernel = FileTime::default();
    let mut user = FileTime::default();
    let ok =
        unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) } != 0;
    unsafe {
        CloseHandle(handle);
    }
    if ok {
        ((creation.high as u64) << 32) | creation.low as u64
    } else {
        0
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub(crate) fn process_start_time(_pid: u32) -> u64 {
    0
}

fn new_instance_id() -> u64 {
    static SEQUENCE: AtomicU64 = AtomicU64::new(1);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|v| v.as_nanos() as u64)
        .unwrap_or(0);
    let seq = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id() as u64;
    // Mix time, PID, process start and a process-local sequence. This is an
    // identity token (not a secret); ensure zero remains reserved for legacy
    // or invalid buffers.
    let mut id = now ^ pid.rotate_left(17) ^ process_start_time(pid as u32).rotate_left(31) ^ seq;
    id ^= id >> 30;
    id = id.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    id ^= id >> 27;
    id = id.wrapping_mul(0x94d0_49bb_1331_11eb);
    id ^= id >> 31;
    if id == 0 {
        1
    } else {
        id
    }
}

#[cfg(unix)]
pub(crate) fn process_exists(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let ret = unsafe { libc::kill(pid as libc::c_int, 0) };
    ret == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
#[cfg(not(windows))]
pub(crate) fn process_exists(pid: u32) -> bool {
    pid == std::process::id()
}

#[cfg(windows)]
pub(crate) fn process_exists(pid: u32) -> bool {
    use windows_process::*;
    if pid == 0 {
        return false;
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        // Access-denied and protected-process failures must be treated as live:
        // a false negative here permits a writer to overwrite borrowed bytes.
        return unsafe { GetLastError() } != ERROR_INVALID_PARAMETER;
    }
    unsafe {
        CloseHandle(handle);
    }
    true
}

/// Conservatively test whether `(pid, start_time)` still names a live process.
///
/// If the platform cannot obtain a start time, a live PID is treated as the
/// same process. This may delay recovery after PID reuse, but never allows
/// recovery to overwrite memory still borrowed by a live reader.
pub(crate) fn process_instance_alive(pid: u32, expected_start_time: u64) -> bool {
    if !process_exists(pid) {
        return false;
    }
    if expected_start_time == 0 {
        return true;
    }
    let actual = process_start_time(pid);
    actual == 0 || actual == expected_start_time
}

/// Try to append a row whose total payload size (`row_data`) is already known.
/// Caller must validate schema and compute `row_data` before calling.
/// Returns `false` if the current chunk has no room.
pub(crate) fn write_row_bytes(buf: &mut [u8], values: &[Value], row_data: usize) -> bool {
    let ptr = buf.as_mut_ptr();
    let (wc, csz, doff) = unsafe {
        let h = &*(ptr as *const Header);
        (
            h.write_chunk.load(Ordering::Relaxed) as usize,
            h.chunk_size as usize,
            h.data_offset as usize,
        )
    };
    let cs = doff + wc * csz;
    let used = unsafe {
        let ch = &*(ptr.add(cs) as *const ChunkHeader);
        ch.used.load(Ordering::Relaxed) as usize
    };

    let total = 4 + row_data;
    if CHUNK_HEADER_SIZE + used + total > csz {
        return false;
    }

    let row_start = cs + CHUNK_HEADER_SIZE + used;
    w32(buf, row_start, row_data as u32);
    let mut off = row_start + 4;
    for v in values {
        off += v.encode(&mut buf[off..]);
    }
    unsafe {
        let ch = &*(ptr.add(cs) as *const ChunkHeader);
        if let Some(ts) = row_ts(&*(ptr as *const Header), values) {
            note_row_ts(ch, ts);
        }
        ch.used.store((used + total) as u32, Ordering::Release);
        ch.row_count.fetch_add(1, Ordering::Release);
    }
    true
}

/// Advance the ring buffer to the next chunk.
///
/// MEMT is single-writer, so no lock is taken. Takes `&mut [u8]` so that
/// LLVM does not mark the pointer `readonly` (which would let it elide the
/// atomic stores below in optimised builds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdvanceOutcome {
    Advanced,
    ReadersPinned,
}

pub(crate) fn advance_chunk_raw_detailed(buf: &mut [u8]) -> AdvanceOutcome {
    let ptr = buf.as_mut_ptr();
    unsafe {
        let h = &*(ptr as *const Header);
        let wc = h.write_chunk.load(Ordering::Relaxed);
        let csz = h.chunk_size as usize;
        let doff = h.data_offset as usize;
        let num_chunks = h.num_chunks;

        let cur_cs = doff + wc as usize * csz;
        let cur_ch = &*(ptr.add(cur_cs) as *const ChunkHeader);

        let new_wc = (wc + 1) % num_chunks;
        let cs = doff + new_wc as usize * csz;
        let new_ch = &*(ptr.add(cs) as *const ChunkHeader);
        let previous_state = new_ch.state.load(Ordering::SeqCst);
        // Block new pins before touching any byte in the old generation.
        new_ch
            .state
            .store(ChunkState::Empty as u32, Ordering::SeqCst);

        // A reader may have claimed a lease immediately before observing
        // Empty and will release it after validation fails. Dead owners are
        // reclaimed by `chunk_has_live_leases`.
        const PIN_DRAIN_SPINS: usize = 1024;
        for attempt in 0..PIN_DRAIN_SPINS {
            if !chunk_has_live_leases(buf, new_wc as usize) {
                break;
            }
            if attempt < 64 {
                std::hint::spin_loop();
            } else {
                std::thread::yield_now();
            }
        }
        if chunk_has_live_leases(buf, new_wc as usize) {
            new_ch.state.store(previous_state, Ordering::SeqCst);
            h.writes_blocked.fetch_add(1, Ordering::Relaxed);
            log::warn!("memtable row dropped: active readers still pin the recycle target");
            return AdvanceOutcome::ReadersPinned;
        }
        // A load-only "no live leases" observation is insufficient: a
        // reader that saw the old non-Empty state may claim immediately after
        // the check. Reserve every slot before resetting metadata; a racing
        // claimant makes this operation fail without touching the old bytes.
        let Some(_recycle_guard) = try_lock_chunk_for_recycle(buf, new_wc as usize) else {
            new_ch.state.store(previous_state, Ordering::SeqCst);
            h.writes_blocked.fetch_add(1, Ordering::Relaxed);
            log::warn!("memtable row dropped: reader raced with recycle ownership");
            return AdvanceOutcome::ReadersPinned;
        };

        if cur_cs != cs {
            cur_ch
                .state
                .store(ChunkState::Sealed as u32, Ordering::Release);
        }

        let rows_lost = new_ch.row_count.load(Ordering::Relaxed);
        if rows_lost > 0 {
            h.chunks_recycled.fetch_add(1, Ordering::Relaxed);
            h.rows_overwritten.fetch_add(rows_lost, Ordering::Relaxed);
            log::warn!(
                "memtable ring overwrite: lost {rows_lost} rows (total overwritten {})",
                h.rows_overwritten.load(Ordering::Relaxed)
            );
        }
        new_ch.used.store(0, Ordering::Relaxed);
        new_ch.row_count.store(0, Ordering::Relaxed);
        new_ch.min_ts.store(TS_MIN_INIT, Ordering::Relaxed);
        new_ch.max_ts.store(TS_MAX_INIT, Ordering::Relaxed);
        // Publish the new generation only after resetting every extent/range
        // field. A late reader can observe the same Writing state on both
        // sides of the Empty transition (state ABA); if it observes this new
        // generation, the Acquire generation load in `pin_chunk` must also
        // make `used == 0` and the other reset metadata visible.
        new_ch.generation.fetch_add(1, Ordering::AcqRel);
        new_ch
            .state
            .store(ChunkState::Writing as u32, Ordering::Release);

        (&*(ptr as *const Header))
            .write_chunk
            .store(new_wc, Ordering::Release);
        AdvanceOutcome::Advanced
    }
}

pub(crate) fn advance_chunk_raw(buf: &mut [u8]) -> bool {
    advance_chunk_raw_detailed(buf) == AdvanceOutcome::Advanced
}
/// Walk rows in a sealed chunk: verify row lengths stay within `used` and
/// dedup refs (negative var-length prefix) point inside the chunk data region.
///
/// When `has_dedup` is false, any negative length prefix is rejected as
/// invalid — the buffer was not written with dedup enabled.
fn validate_chunk_rows(
    buf: &[u8],
    cs: usize,
    used: usize,
    nc: usize,
    has_dedup: bool,
) -> Result<()> {
    let data_base = cs + CHUNK_HEADER_SIZE;
    let mut pos = 0usize;
    while pos + 4 <= used {
        let row_len = r32(buf, data_base + pos) as usize;
        if pos + 4 + row_len > used {
            return Err(MemtableError::InvalidBuffer(
                "row extends beyond chunk used region",
            ));
        }
        let row_start = data_base + pos + 4;
        let mut col_off = 0usize;
        for ci in 0..nc {
            if col_off >= row_len {
                break;
            }
            let Some(dt) = DType::from_u32(col_desc(buf, ci).dtype) else {
                break;
            };
            if let Some(sz) = dt.fixed_size() {
                col_off += sz;
            } else if col_off + 4 <= row_len {
                let raw = i32::from_le_bytes(
                    buf[row_start + col_off..row_start + col_off + 4]
                        .try_into()
                        .unwrap(),
                );
                if raw < 0 {
                    if !has_dedup {
                        return Err(MemtableError::InvalidBuffer("dedup ref in non-dedup table"));
                    }
                    let ref_off = (-raw) as usize;
                    if ref_off < CHUNK_HEADER_SIZE || ref_off >= CHUNK_HEADER_SIZE + used {
                        return Err(MemtableError::InvalidBuffer(
                            "dedup ref outside chunk data region",
                        ));
                    }
                    col_off += 4;
                } else {
                    col_off += 4 + raw as usize;
                }
            }
        }
        pos += 4 + row_len;
    }
    Ok(())
}

/// Structural validation of a MemTable buffer.
///
/// Checks magic, version, byte order, feature flags, layout offsets,
/// column dtypes, chunk states, used-within-payload bounds, row boundary
/// integrity, and dedup ref ranges.
///
/// All `from_buf` / `new` constructors funnel through this function.
pub fn validate_buf(buf: &[u8]) -> Result<()> {
    if buf.len() < mem::size_of::<Header>() {
        return Err(MemtableError::InvalidBuffer("buffer too small for header"));
    }
    let required_alignment = mem::align_of::<Header>();
    if !(buf.as_ptr() as usize).is_multiple_of(required_alignment) {
        return Err(MemtableError::InvalidBuffer(
            "buffer address does not satisfy header alignment",
        ));
    }
    let h = header(buf);
    if h.magic != MAGIC {
        return Err(MemtableError::InvalidBuffer("invalid magic"));
    }
    if h.version != VERSION {
        return Err(MemtableError::InvalidBuffer("unsupported version"));
    }
    if (h.header_size as usize) < mem::size_of::<Header>() {
        return Err(MemtableError::InvalidBuffer("header_size too small"));
    }
    let bom = u16::from_ne_bytes(BYTE_ORDER_MARK);
    if h.byte_order != bom {
        return Err(MemtableError::InvalidBuffer(
            "byte order mismatch (buffer written on different-endian host)",
        ));
    }
    if h.flags & !FLAGS_KNOWN != 0 {
        return Err(MemtableError::InvalidBuffer("unknown feature flags set"));
    }
    let has_dedup = h.flags & FLAG_DEDUP != 0;
    let nc = h.num_cols as usize;
    if h.num_chunks == 0 {
        return Err(MemtableError::InvalidBuffer("num_chunks must be > 0"));
    }
    let csz = h.chunk_size as usize;
    if csz < CHUNK_HEADER_SIZE + 8 {
        return Err(MemtableError::InvalidBuffer("chunk_size too small"));
    }
    if !csz.is_multiple_of(mem::align_of::<ChunkHeader>()) {
        return Err(MemtableError::InvalidBuffer(
            "chunk_size does not satisfy chunk header alignment",
        ));
    }
    let expected_lease_off =
        checked_lease_offset(nc).ok_or(MemtableError::InvalidBuffer("column layout overflow"))?;
    if h.lease_offset as usize != expected_lease_off || h.lease_slots as usize != READER_LEASE_SLOTS
    {
        return Err(MemtableError::InvalidBuffer("invalid reader lease layout"));
    }
    let expected_off = checked_data_offset(nc, h.num_chunks as usize)
        .ok_or(MemtableError::InvalidBuffer("column layout overflow"))?;
    if expected_off > u32::MAX as usize {
        return Err(MemtableError::InvalidBuffer("column layout exceeds format"));
    }
    if h.data_offset as usize != expected_off {
        return Err(MemtableError::InvalidBuffer("invalid data_offset"));
    }
    let required = csz
        .checked_mul(h.num_chunks as usize)
        .and_then(|chunks| expected_off.checked_add(chunks))
        .ok_or(MemtableError::InvalidBuffer("table size overflow"))?;
    if buf.len() < required {
        return Err(MemtableError::InvalidBuffer("buffer too small for data"));
    }
    for i in 0..nc {
        let desc = col_desc(buf, i);
        let name_len = u16::from_le_bytes([desc.name[0], desc.name[1]]) as usize;
        if name_len > 54 {
            return Err(MemtableError::InvalidBuffer(
                "column name length out of range",
            ));
        }
        let dt = desc.dtype;
        if !(1..=9).contains(&dt) {
            return Err(MemtableError::InvalidBuffer("invalid column dtype"));
        }
    }
    let ts_col = h.ts_col as usize;
    if ts_col != 0 {
        if ts_col > nc {
            return Err(MemtableError::InvalidBuffer("ts_col out of range"));
        }
        if DType::from_u32(col_desc(buf, ts_col - 1).dtype) != Some(DType::I64) {
            return Err(MemtableError::InvalidBuffer(
                "ts_col must reference an I64 column",
            ));
        }
    }
    let payload_cap = csz - CHUNK_HEADER_SIZE;
    for i in 0..h.num_chunks as usize {
        let cs = expected_off + i * csz;
        let ch = chunk_header(buf, cs);
        let state = ch.state.load(Ordering::Acquire);
        if state > 2 {
            return Err(MemtableError::InvalidBuffer("invalid chunk state"));
        }
        let used = ch.used.load(Ordering::Acquire) as usize;
        if used > payload_cap {
            return Err(MemtableError::InvalidBuffer(
                "chunk used exceeds payload capacity",
            ));
        }
        // Deep validation reads ordinary row bytes, so it must participate in
        // the same pin protocol as Row/RowIter. Generation re-check alone is
        // too late: a recycle could otherwise race the byte reads themselves.
        if let Some((_pin, _generation, snap_used)) = pin_chunk(buf, cs)? {
            if snap_used > 0 {
                validate_chunk_rows(buf, cs, snap_used, nc, has_dedup)?;
            }
        }
    }
    Ok(())
}

/// Check that `values` matches the table schema (column count + dtypes).
pub(crate) fn validate_row_schema(buf: &[u8], values: &[Value]) -> bool {
    let nc = header(buf).num_cols as usize;
    if values.len() != nc {
        return false;
    }
    for (i, v) in values.iter().enumerate() {
        let Some(dt) = DType::from_u32(col_desc(buf, i).dtype) else {
            return false;
        };
        let ok = matches!(
            (v, dt),
            (Value::U8(_), DType::U8)
                | (Value::U32(_), DType::U32)
                | (Value::I32(_), DType::I32)
                | (Value::I64(_), DType::I64)
                | (Value::F32(_), DType::F32)
                | (Value::F64(_), DType::F64)
                | (Value::U64(_), DType::U64)
                | (Value::Str(_), DType::Str)
                | (Value::Bytes(_), DType::Bytes)
        );
        if !ok {
            return false;
        }
    }
    true
}

// ── init ────────────────────────────────────────────────────────────

pub(crate) fn init_buf(
    buf: &mut [u8],
    schema: &Schema,
    chunk_size: u32,
    num_chunks: u32,
) -> Result<()> {
    let required_alignment = mem::align_of::<Header>();
    if !(buf.as_ptr() as usize).is_multiple_of(required_alignment) {
        return Err(MemtableError::InvalidBuffer(
            "buffer address does not satisfy header alignment",
        ));
    }
    if num_chunks == 0 {
        return Err(MemtableError::InvalidBuffer("num_chunks must be > 0"));
    }
    if !(chunk_size as usize).is_multiple_of(mem::align_of::<ChunkHeader>()) {
        return Err(MemtableError::InvalidBuffer(
            "chunk_size does not satisfy chunk header alignment",
        ));
    }
    let nc = schema.cols.len();
    if nc > u32::MAX as usize {
        return Err(MemtableError::InvalidBuffer("too many columns"));
    }
    let lease_off =
        checked_lease_offset(nc).ok_or(MemtableError::InvalidBuffer("column layout overflow"))?;
    let data_off = checked_data_offset(nc, num_chunks as usize)
        .ok_or(MemtableError::InvalidBuffer("column layout overflow"))?;
    if data_off > u32::MAX as usize {
        return Err(MemtableError::InvalidBuffer("column layout exceeds format"));
    }
    let required = (chunk_size as usize)
        .checked_mul(num_chunks as usize)
        .and_then(|chunks| data_off.checked_add(chunks))
        .ok_or(MemtableError::InvalidBuffer("table size overflow"))?;
    if buf.len() < required {
        return Err(MemtableError::InvalidBuffer("buffer too small"));
    }
    if (chunk_size as usize) < CHUNK_HEADER_SIZE + 8 {
        return Err(MemtableError::InvalidBuffer("chunk_size too small"));
    }

    // First I64 column with a recognised timestamp name becomes the
    // designated time column (index + 1; 0 = none).
    let ts_col = schema
        .cols
        .iter()
        .position(|c| c.dtype == DType::I64 && TS_COL_NAMES.contains(&c.name.as_str()))
        .map(|i| (i + 1) as u16)
        .unwrap_or(0);

    let h = header_mut(buf);
    h.magic = MAGIC;
    h.version = VERSION;
    h.header_size = mem::size_of::<Header>() as u16;
    h.byte_order = u16::from_ne_bytes(BYTE_ORDER_MARK);
    h.ts_col = ts_col;
    h.flags = 0;
    h.num_cols = nc as u32;
    h.num_chunks = num_chunks;
    h.chunk_size = chunk_size;
    h.data_offset = data_off as u32;
    h.write_chunk.store(0, Ordering::Relaxed);
    h.refcount.store(1, Ordering::Relaxed);
    h.creator_pid = std::process::id();
    h._pad0 = 0;
    h.creator_start_time = process_start_time(std::process::id());
    h.chunks_recycled.store(0, Ordering::Relaxed);
    h.rows_overwritten.store(0, Ordering::Relaxed);
    h.lease_offset = lease_off as u32;
    h.lease_slots = READER_LEASE_SLOTS as u32;
    h.instance_id = new_instance_id();
    h.writes_blocked.store(0, Ordering::Relaxed);
    h.reader_lease_failures.store(0, Ordering::Relaxed);
    h._reserved_v5 = [0; 4];

    for (i, col) in schema.cols.iter().enumerate() {
        let cd = col_desc_mut(buf, i);
        cd.set_name(&col.name);
        cd.dtype = col.dtype as u32;
        cd.elem_size = col.elem_size as u32;
    }

    for chunk in 0..num_chunks as usize {
        for slot in 0..READER_LEASE_SLOTS {
            let lease = reader_lease(buf, chunk, slot);
            lease.state.store(0, Ordering::Relaxed);
            lease.owner_start_time.store(0, Ordering::Relaxed);
        }
    }

    // Initialize all chunk headers
    for i in 0..num_chunks as usize {
        let cs = data_off + i * chunk_size as usize;
        let ch = chunk_header(buf, cs);
        ch.generation.store(0, Ordering::Relaxed);
        ch.used.store(0, Ordering::Relaxed);
        ch.row_count.store(0, Ordering::Relaxed);
        ch._reserved.store(0, Ordering::Relaxed);
        ch.min_ts.store(TS_MIN_INIT, Ordering::Relaxed);
        ch.max_ts.store(TS_MAX_INIT, Ordering::Relaxed);
        ch.state.store(ChunkState::Empty as u32, Ordering::Relaxed);
    }
    // Chunk 0 is the initial write target
    let ch0 = chunk_header(buf, data_off);
    ch0.generation.store(1, Ordering::Relaxed);
    ch0.state
        .store(ChunkState::Writing as u32, Ordering::Relaxed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memtable::push_plain_row;
    use crate::schema::{DType, Schema, Value};

    #[test]
    fn init_buf_rejects_small_buffer() {
        let schema = Schema::new().col("x", DType::I32);
        let mut buf = vec![0u8; 32]; // way too small
        assert!(init_buf(&mut buf, &schema, 1024, 1).is_err());
    }

    #[test]
    fn init_buf_rejects_zero_chunks_and_misalignment() {
        let schema = Schema::new().col("x", DType::I32);
        let size = compute_data_offset(1, 1) + 1024;
        let mut aligned = vec![0u8; size];
        assert!(init_buf(&mut aligned, &schema, 1024, 0).is_err());

        let mut storage = vec![0u8; size + 1];
        assert!(init_buf(&mut storage[1..], &schema, 1024, 1).is_err());
    }

    #[test]
    fn advance_chunk_counts_overwritten_rows() {
        let schema = Schema::new().col("x", DType::I32);
        let chunk_size = 128u32;
        let num_chunks = 2u32;
        let total = crate::layout::compute_data_offset(schema.cols.len(), num_chunks as usize)
            + chunk_size as usize * num_chunks as usize;
        let mut buf = vec![0u8; total];
        init_buf(&mut buf, &schema, chunk_size, num_chunks).unwrap();

        for i in 0..10_000i32 {
            assert!(push_plain_row(&mut buf, &[Value::I32(i)]));
            let (_, rows) = crate::layout::ring_overwrite_stats(&buf);
            if rows > 0 {
                assert!(rows >= 1);
                return;
            }
        }
        panic!("expected ring overwrite within 10_000 pushes");
    }

    #[test]
    fn advance_chunk_refuses_to_recycle_a_pinned_target() {
        let schema = Schema::new().col("x", DType::I32);
        let chunk_size = 128u32;
        let num_chunks = 2u32;
        let data_off = crate::layout::compute_data_offset(schema.cols.len(), num_chunks as usize);
        let mut buf = vec![0u8; data_off + chunk_size as usize * num_chunks as usize];
        init_buf(&mut buf, &schema, chunk_size, num_chunks).unwrap();

        assert!(advance_chunk_raw(&mut buf)); // target chunk 1
        let target = data_off;
        let lease = reader_lease(&buf, 0, 0);
        lease
            .owner_start_time
            .store(process_start_time(std::process::id()), Ordering::SeqCst);
        lease
            .state
            .store(((std::process::id() as u64) << 32) | 1, Ordering::SeqCst);

        assert!(!advance_chunk_raw(&mut buf));
        assert_eq!(header(&buf).write_chunk.load(Ordering::Acquire), 1);
        assert_eq!(
            chunk_header(&buf, target).state.load(Ordering::Acquire),
            ChunkState::Sealed as u32
        );

        reader_lease(&buf, 0, 0).state.store(0, Ordering::SeqCst);
        assert!(advance_chunk_raw(&mut buf));
        assert_eq!(header(&buf).write_chunk.load(Ordering::Acquire), 0);
    }

    #[test]
    fn advance_chunk_reclaims_dead_process_lease() {
        let schema = Schema::new().col("x", DType::I32);
        let chunk_size = 128u32;
        let num_chunks = 2u32;
        let data_off = crate::layout::compute_data_offset(schema.cols.len(), num_chunks as usize);
        let mut buf = vec![0u8; data_off + chunk_size as usize * num_chunks as usize];
        init_buf(&mut buf, &schema, chunk_size, num_chunks).unwrap();
        assert!(advance_chunk_raw(&mut buf));

        let dead_pid = 2_000_000_000u32;
        assert!(!process_exists(dead_pid));
        let lease = reader_lease(&buf, 0, 0);
        lease.owner_start_time.store(123, Ordering::SeqCst);
        lease
            .state
            .store(((dead_pid as u64) << 32) | 7, Ordering::SeqCst);

        assert!(advance_chunk_raw(&mut buf));
        assert_eq!(reader_lease(&buf, 0, 0).state.load(Ordering::SeqCst), 0);
    }

    #[test]
    #[cfg(unix)]
    fn crashed_reader_process_lease_is_reclaimed() {
        let schema = Schema::new().col("x", DType::I32);
        let chunk_size = 128u32;
        let size = crate::memtable::MemTable::required_size(&schema, chunk_size as usize, 2);
        let shared = crate::test_mmap::TestSharedFile::new(size);
        let mut writer_map = shared.map_mut();
        let mut writer =
            crate::memtable::MemTableWriter::init(&mut writer_map, &schema, chunk_size, 2).unwrap();
        assert!(writer.push_row(&[Value::I32(7)]));
        assert!(writer.advance_chunk());
        drop(writer);

        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("raw::tests::crash_reader_child")
            .arg("--ignored")
            .env("PROBING_CRASH_READER_PATH", shared.path())
            .status()
            .unwrap();
        assert!(status.success());
        assert!(!chunk_has_live_leases(&writer_map, 0));

        let mut writer = crate::memtable::MemTableWriter::new(&mut writer_map).unwrap();
        assert!(writer.advance_chunk());
        assert_eq!(writer.write_chunk(), 0);
    }

    #[test]
    #[ignore = "helper process for crashed_reader_process_lease_is_reclaimed"]
    #[cfg(unix)]
    fn crash_reader_child() {
        let Some(path) = std::env::var_os("PROBING_CRASH_READER_PATH") else {
            return;
        };
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        let map = unsafe { memmap2::MmapMut::map_mut(&file).unwrap() };
        let view = crate::memtable::MemTableView::new(&map).unwrap();
        let row = view.rows(0).next().unwrap();
        assert_eq!(row.col_i32(0), 7);
        std::process::exit(0);
    }
}
