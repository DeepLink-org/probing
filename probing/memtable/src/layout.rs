//! Low-level layout: header, column descriptors, chunk headers, byte helpers.
//!
//! ## Header v3 binary layout (64 bytes, 1 cache line)
//!
//! ```text
//! offset  size  field               notes
//! ──────────────────────────────────────────────────────────
//!  0       4    magic               0x4D454D54 ("MEMT" in LE)
//!  4       2    version             3
//!  6       2    header_size         64 (validation only)
//!  8       2    byte_order          BOM: written as [0x01, 0x02]
//! 10       2    ts_col              timestamp column index + 1 (0 = none)
//! 12       4    flags               feature bits (see FLAG_*)
//! 16       4    num_cols
//! 20       4    num_chunks
//! 24       4    chunk_size
//! 28       4    data_offset         (64-aligned)
//! ─── 32 byte boundary (cold/hot split) ─────────────────
//! 32       4    write_chunk         AtomicU32
//! 36       4    write_lock          AtomicU32: 0 = unlocked, else holder PID
//! 40       4    refcount            AtomicU32
//! 44       4    creator_pid         PID of creating process
//! 48       8    creator_start_time  process start time (platform-specific)
//! 56       8    lock_owner_start    AtomicU64: lock holder's start time
//! ──────────────────────────────────────────────────────────
//! ```
//!
//! All multi-byte fields are little-endian.  The `byte_order` BOM
//! allows readers to detect endianness mismatch without guessing.

use std::mem;
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

// ── C-style layout structs ──────────────────────────────────────────

/// Magic number for MEMT (ring-buffer time-series table): bytes `M E M T` in little-endian.
pub const MAGIC_MEMT: u32 = 0x4D45_4D54;
pub(crate) const MAGIC: u32 = MAGIC_MEMT;

/// Header format version for MEMT.
///
/// v3: `_pad0` became `ts_col`, `_reserved` became `lock_owner_start`,
/// `write_lock` stores the holder PID (was 0/1), and `ChunkHeader` grew
/// `min_ts`/`max_ts` (24 → 40 bytes).
pub(crate) const VERSION: u16 = 3;

/// Byte-order mark: written as raw bytes `[0x01, 0x02]`.
/// On a LE host, `u16::from_ne_bytes([0x01, 0x02])` == `0x0201`.
pub(crate) const BYTE_ORDER_MARK: [u8; 2] = [0x01, 0x02];

/// Feature flag: dedup back-references may appear in Str/Bytes columns.
///
/// Set when dedup is enabled.  When absent, `validate_buf`
/// rejects any negative length-prefix (dedup ref) as invalid.
pub(crate) const FLAG_DEDUP: u32 = 1 << 0;
// Reserved for future use:
// pub const FLAG_CHECKSUM:  u32 = 1 << 1;
// pub const FLAG_COMPRESSED: u32 = 1 << 2;
// pub const FLAG_SORTED:    u32 = 1 << 3;

/// Bits that this version of the library understands.
pub(crate) const FLAGS_KNOWN: u32 = FLAG_DEDUP;

/// Fixed header at the start of every MemTable buffer (64 bytes).
///
/// **Cold zone** (bytes 0–31): immutable after init — `magic`, `version`,
/// schema dimensions, layout offsets.
///
/// **Hot zone** (bytes 32–63): atomically mutated at runtime —
/// `write_chunk`, `write_lock`, `refcount`.  Separated from the cold
/// zone to avoid false-sharing on different cache lines.
#[repr(C)]
pub(crate) struct Header {
    // ── cold zone (read-only after init) ─────────────────
    pub magic: u32,
    pub version: u16,
    /// Size of this header in bytes (always 64 in v2).
    ///
    /// Used for validation only — column descriptors always start at
    /// offset `size_of::<Header>()` (compile-time constant).  If a
    /// future version extends the header, it will bump `version` and
    /// `header_size` together so that older readers can detect the
    /// mismatch and reject the buffer cleanly.
    pub header_size: u16,
    /// Byte-order mark, written as `BYTE_ORDER_MARK`.
    pub byte_order: u16,
    /// Designated timestamp column **index + 1** (0 = no timestamp column).
    ///
    /// Set at init when the schema contains an `I64` column named
    /// `"timestamp"`. The writer maintains per-chunk `min_ts`/`max_ts`
    /// from this column so readers can prune chunks by time range.
    pub ts_col: u16,
    /// Feature flags (see `FLAG_*` constants).
    pub flags: u32,
    pub num_cols: u32,
    pub num_chunks: u32,
    pub chunk_size: u32,
    /// Byte offset where chunk data begins (64-aligned).
    pub data_offset: u32,

    // ── hot zone (atomically mutated) ────────────────────
    /// Ring buffer: index of the chunk currently being written.
    pub write_chunk: AtomicU32,
    /// Robust writer spinlock: 0 = unlocked, otherwise the **PID** of the
    /// holding process. A waiter that has spun past
    /// [`LOCK_STEAL_TIMEOUT`] checks the holder's liveness and steals the
    /// lock from a dead process (see [`acquire_write_lock`]).
    pub write_lock: AtomicU32,
    /// Reference count for shared lifetime management.
    pub refcount: AtomicU32,
    /// PID of the process that created this table (for cross-process discovery).
    pub creator_pid: u32,
    /// Process start time — for PID-recycling detection.
    /// Linux: clock ticks since boot (`/proc/<pid>/stat` field 22).
    /// macOS: microseconds since epoch (via `sysctl`).
    /// Other: 0 (falls back to PID-only liveness check).
    pub creator_start_time: u64,
    /// Start time of the current lock holder (0 = unknown / not written
    /// yet). Written by the holder right after acquiring; lets waiters
    /// detect PID recycling before stealing. Advisory only.
    pub lock_owner_start: AtomicU64,
}

/// Per-column descriptor, immediately following the Header.
#[repr(C)]
pub(crate) struct ColumnDesc {
    /// Column name, length-prefixed: `[u16 len][utf8 bytes][padding]`.
    pub name: [u8; 56],
    /// `DType` value as `u32`.
    pub dtype: u32,
    /// For fixed-size types: byte size. For `Str`/`Bytes`: 0 (variable-length).
    pub elem_size: u32,
}

impl ColumnDesc {
    pub fn name_str(&self) -> &str {
        let len = u16::from_le_bytes([self.name[0], self.name[1]]) as usize;
        if len == 0 {
            return "";
        }
        std::str::from_utf8(&self.name[2..2 + len]).unwrap_or("")
    }

    pub fn set_name(&mut self, s: &str) {
        self.name = [0u8; 56];
        let b = s.as_bytes();
        let n = b.len().min(54);
        self.name[0..2].copy_from_slice(&(n as u16).to_le_bytes());
        self.name[2..2 + n].copy_from_slice(&b[..n]);
    }
}

/// Sentinel for `ChunkHeader.min_ts` when the chunk holds no rows.
pub(crate) const TS_MIN_INIT: i64 = i64::MAX;
/// Sentinel for `ChunkHeader.max_ts` when the chunk holds no rows.
pub(crate) const TS_MAX_INIT: i64 = i64::MIN;

/// Per-chunk metadata, at the start of every chunk's byte region (40 bytes).
#[repr(C)]
pub(crate) struct ChunkHeader {
    /// Incremented each time the chunk is recycled (ring wrap).
    /// Readers capture this to detect stale reads.
    pub generation: AtomicU64,
    /// Bytes of row data written (excluding this header).
    pub used: AtomicU32,
    /// Number of committed rows in this chunk.
    pub row_count: AtomicU32,
    /// Chunk lifecycle state (see `ChunkState`).
    pub state: AtomicU32,
    pub _reserved: u32,
    /// Smallest value of the designated timestamp column in this chunk
    /// ([`TS_MIN_INIT`] when empty or no `Header::ts_col`). Maintained by
    /// the writer; readers must validate against `generation` snapshots.
    pub min_ts: AtomicI64,
    /// Largest timestamp in this chunk ([`TS_MAX_INIT`] when empty).
    pub max_ts: AtomicI64,
}

/// Chunk lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub(crate) enum ChunkState {
    Empty = 0,
    Writing = 1,
    Sealed = 2,
}

pub(crate) const CHUNK_HEADER_SIZE: usize = mem::size_of::<ChunkHeader>();

const _: () = {
    assert!(mem::size_of::<Header>() == 64);
    assert!(mem::size_of::<ColumnDesc>() == 64);
    assert!(mem::size_of::<ChunkHeader>() == 40);
};
// ── struct accessors ────────────────────────────────────────────────

pub(crate) fn header(buf: &[u8]) -> &Header {
    debug_assert!(buf.len() >= mem::size_of::<Header>());
    unsafe { &*(buf.as_ptr() as *const Header) }
}

pub(crate) fn header_mut(buf: &mut [u8]) -> &mut Header {
    debug_assert!(buf.len() >= mem::size_of::<Header>());
    unsafe { &mut *(buf.as_mut_ptr() as *mut Header) }
}

pub(crate) fn col_desc(buf: &[u8], col: usize) -> &ColumnDesc {
    let off = mem::size_of::<Header>() + col * mem::size_of::<ColumnDesc>();
    debug_assert!(buf.len() >= off + mem::size_of::<ColumnDesc>());
    unsafe { &*(buf[off..].as_ptr() as *const ColumnDesc) }
}

pub(crate) fn col_desc_mut(buf: &mut [u8], col: usize) -> &mut ColumnDesc {
    let off = mem::size_of::<Header>() + col * mem::size_of::<ColumnDesc>();
    debug_assert!(buf.len() >= off + mem::size_of::<ColumnDesc>());
    unsafe { &mut *(buf[off..].as_mut_ptr() as *mut ColumnDesc) }
}

// ── chunk header accessor ───────────────────────────────────────────

pub(crate) fn chunk_header(buf: &[u8], cs: usize) -> &ChunkHeader {
    debug_assert!(cs % 8 == 0 && buf.len() >= cs + CHUNK_HEADER_SIZE);
    unsafe { &*(buf[cs..].as_ptr() as *const ChunkHeader) }
}

/// How long a waiter spins before checking whether the lock holder is
/// still alive (and stealing the lock from a dead process).
///
/// Writers hold the lock for nanoseconds–microseconds; even a descheduled
/// holder resumes within milliseconds. Reaching this timeout in practice
/// means the holder crashed while holding the lock.
pub(crate) const LOCK_STEAL_TIMEOUT: Duration = Duration::from_millis(500);

/// `true` when a process with `pid` exists (it may belong to another user).
fn process_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    if unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
        return true;
    }
    // EPERM: the process exists but we may not signal it.
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// This process's kernel start time, cached per PID (reads `/proc` on Linux).
///
/// **Fork safety:** the cache is keyed on the live PID, not a one-shot
/// `OnceLock`. A child inheriting a parent's cached value would otherwise
/// record the *parent's* start time in `lock_owner_start`, and a waiter
/// comparing against the child's real start time would mistake the live child
/// for a recycled PID and steal its lock — exactly the hazard fork-heavy
/// workloads (e.g. PyTorch DataLoader) trigger. Re-reading whenever the PID
/// changes makes every post-fork caller observe its own start time.
fn my_start_time() -> u64 {
    static MY_PID: AtomicU32 = AtomicU32::new(0);
    static MY_START: AtomicU64 = AtomicU64::new(0);

    let pid = std::process::id();
    if MY_PID.load(Ordering::Acquire) == pid {
        let cached = MY_START.load(Ordering::Acquire);
        if cached != 0 {
            return cached;
        }
    }
    let start = crate::raw::process_start_time(pid);
    // Publish start before PID: a reader that observes the matching PID is then
    // guaranteed to also observe the start written for it.
    MY_START.store(start, Ordering::Release);
    MY_PID.store(pid, Ordering::Release);
    start
}

/// Decide whether the lock can be stolen from `holder`, and try to.
///
/// Steal conditions (either):
/// - `holder` no longer exists (crashed / killed while holding the lock);
/// - `holder` exists but its kernel start time does not match the one the
///   real holder recorded in `lock_owner_start` — the PID was recycled by
///   an unrelated process. Re-checked after a grace period to rule out
///   the transient window where a fresh holder has not yet recorded its
///   start time.
///
/// Stealing is safe with respect to data: rows only become visible via the
/// `used`/`row_count` Release stores at the end of a write, so a row half
/// written by the dead holder stays uncommitted and is simply overwritten.
#[cold]
#[inline(never)]
fn try_steal_lock(h: &Header, holder: u32, me: u32) -> bool {
    if process_alive(holder) {
        let owner_start = h.lock_owner_start.load(Ordering::Relaxed);
        let actual_start = crate::raw::process_start_time(holder);
        if owner_start == 0 || actual_start == 0 || actual_start == owner_start {
            return false; // genuinely alive (or cannot tell) — keep waiting
        }
        std::thread::sleep(Duration::from_millis(10));
        if h.write_lock.load(Ordering::Relaxed) != holder
            || h.lock_owner_start.load(Ordering::Relaxed) != owner_start
        {
            return false; // lock changed hands meanwhile — not stale
        }
    }
    if h.write_lock
        .compare_exchange(holder, me, Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
    {
        h.lock_owner_start.store(my_start_time(), Ordering::Relaxed);
        return true;
    }
    false
}

/// Acquire the **robust** writer spinlock with exponential back-off.
///
/// The lock word holds the owner's PID (0 = unlocked). First few failures
/// use `spin_loop()` (pause instruction), then escalate to `yield_now()`.
/// A waiter stuck past [`LOCK_STEAL_TIMEOUT`] verifies the holder's
/// liveness and steals the lock from a dead process (see
/// [`try_steal_lock`]), so a writer crashing inside the critical section
/// cannot deadlock other writer processes forever.
///
/// SAFETY NOTE: the buffer parameter is `&mut [u8]` (not `&[u8]`) so that
/// LLVM does **not** mark the pointer `readonly`. With `&[u8]` LLVM may
/// legally eliminate the atomic store inside `release_write_lock`, turning
/// the spin loop into an infinite loop in optimised (release) builds.
pub(crate) fn acquire_write_lock(buf: &mut [u8]) {
    let ptr = buf.as_mut_ptr() as *const Header;
    let h = unsafe { &*ptr };
    let me = std::process::id();
    let mut spins = 0u32;
    let mut waiting_since: Option<Instant> = None;
    loop {
        match h
            .write_lock
            .compare_exchange_weak(0, me, Ordering::Acquire, Ordering::Relaxed)
        {
            Ok(_) => {
                h.lock_owner_start.store(my_start_time(), Ordering::Relaxed);
                return;
            }
            Err(holder) if holder != 0 => {
                let since = *waiting_since.get_or_insert_with(Instant::now);
                if spins >= 16 && since.elapsed() >= LOCK_STEAL_TIMEOUT {
                    if try_steal_lock(h, holder, me) {
                        return;
                    }
                    waiting_since = Some(Instant::now());
                }
            }
            Err(_) => {} // spurious failure with lock free — retry CAS
        }
        if spins < 16 {
            for _ in 0..1 << spins.min(4) {
                std::hint::spin_loop();
            }
        } else {
            std::thread::yield_now();
        }
        spins += 1;
    }
}

/// Release the writer spinlock. See [`acquire_write_lock`] for why `&mut`.
///
/// Clears `lock_owner_start` *before* the lock word so that waiters never
/// pair the next holder's PID with this holder's start time.
pub(crate) fn release_write_lock(buf: &mut [u8]) {
    let ptr = buf.as_mut_ptr() as *const Header;
    unsafe {
        (*ptr).lock_owner_start.store(0, Ordering::Relaxed);
        (*ptr).write_lock.store(0, Ordering::Release);
    }
}
pub(crate) fn r32(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(buf[off..off + 4].try_into().unwrap())
}

pub(crate) fn w32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

pub(crate) fn align64(n: usize) -> usize {
    (n + 63) & !63
}

// ── layout helpers ──────────────────────────────────────────────────

pub(crate) fn compute_data_offset(num_cols: usize) -> usize {
    align64(mem::size_of::<Header>() + num_cols * mem::size_of::<ColumnDesc>())
}

pub(crate) fn chunk_start_off(buf: &[u8], chunk: usize) -> usize {
    let h = header(buf);
    h.data_offset as usize + chunk * h.chunk_size as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem;

    #[test]
    fn struct_sizes() {
        assert_eq!(mem::size_of::<Header>(), 64);
        assert_eq!(mem::size_of::<ColumnDesc>(), 64);
        assert_eq!(mem::size_of::<ChunkHeader>(), 40);
    }

    #[test]
    fn byte_order_mark_sanity() {
        let bom = u16::from_ne_bytes(BYTE_ORDER_MARK);
        let expected_le = u16::from_le_bytes(BYTE_ORDER_MARK);
        assert_eq!(bom, expected_le);
    }

    /// Fork safety: after `fork()`, `my_start_time()` must return the *child's*
    /// own kernel start time, not a value cached for the parent before the
    /// fork. With the old `OnceLock` cache the child returned the parent's
    /// start time; a waiter then compared it against the child's real start
    /// time and stole the lock from a live holder. The test process has run
    /// long enough that its start tick differs from a freshly-forked child's,
    /// so the stale value would be observably wrong.
    ///
    /// Linux-only: kernel start times come from `/proc`. On platforms without
    /// it `process_start_time` returns 0, the PID-recycle steal path is inert,
    /// and there is no fork hazard to guard against.
    #[cfg(target_os = "linux")]
    #[test]
    fn my_start_time_refreshes_after_fork() {
        // Warm the per-PID cache for the parent (mimics the leaked OnceLock).
        let parent = my_start_time();
        assert_ne!(parent, 0, "parent start time should be readable");

        unsafe {
            let pid = libc::fork();
            assert!(pid >= 0, "fork failed");
            if pid == 0 {
                // Child: the cached value must equal a fresh read for THIS pid.
                let cached = my_start_time();
                let fresh = crate::raw::process_start_time(std::process::id());
                libc::_exit(if cached == fresh && cached != 0 { 0 } else { 1 });
            }
            let mut status = 0;
            libc::waitpid(pid, &mut status, 0);
            assert!(
                libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
                "child my_start_time() must reflect its own process, not the parent's cache",
            );
        }
    }
}
