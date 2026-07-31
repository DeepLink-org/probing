use crate::layout::{
    chunk_header, col_desc, header, r32, reader_lease, ChunkState, ReaderLease, CHUNK_HEADER_SIZE,
};
use crate::raw::{current_process_id, process_exists, process_instance_alive, process_start_time};
use crate::schema::DType;
use std::sync::atomic::{fence, AtomicPtr, AtomicU32, Ordering};
use std::sync::Arc;

fn try_read_i32(data: &[u8], off: usize) -> Option<i32> {
    Some(i32::from_le_bytes(
        data.get(off..off.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn try_read_u32(data: &[u8], off: usize) -> Option<u32> {
    if off + 4 > data.len() {
        return None;
    }
    Some(u32::from_le_bytes([
        data[off],
        data[off + 1],
        data[off + 2],
        data[off + 3],
    ]))
}

fn try_read_i64(data: &[u8], off: usize) -> Option<i64> {
    if off + 8 > data.len() {
        return None;
    }
    Some(i64::from_le_bytes([
        data[off],
        data[off + 1],
        data[off + 2],
        data[off + 3],
        data[off + 4],
        data[off + 5],
        data[off + 6],
        data[off + 7],
    ]))
}

fn try_read_u64(data: &[u8], off: usize) -> Option<u64> {
    if off + 8 > data.len() {
        return None;
    }
    Some(u64::from_le_bytes([
        data[off],
        data[off + 1],
        data[off + 2],
        data[off + 3],
        data[off + 4],
        data[off + 5],
        data[off + 6],
        data[off + 7],
    ]))
}

fn try_read_f32(data: &[u8], off: usize) -> Option<f32> {
    if off + 4 > data.len() {
        return None;
    }
    Some(f32::from_le_bytes([
        data[off],
        data[off + 1],
        data[off + 2],
        data[off + 3],
    ]))
}

fn try_read_f64(data: &[u8], off: usize) -> Option<f64> {
    if off + 8 > data.len() {
        return None;
    }
    Some(f64::from_le_bytes([
        data[off],
        data[off + 1],
        data[off + 2],
        data[off + 3],
        data[off + 4],
        data[off + 5],
        data[off + 6],
        data[off + 7],
    ]))
}

fn try_var_field_size(buf: &[u8], off: usize) -> Option<usize> {
    let raw = try_read_i32(buf, off)?;
    if raw < 0 {
        Some(4)
    } else {
        4usize.checked_add(raw as usize)
    }
}

fn try_resolve_var<'a>(
    row_data: &'a [u8],
    row_off: usize,
    buf: &'a [u8],
    chunk_start: usize,
) -> Option<&'a [u8]> {
    let raw = try_read_i32(row_data, row_off)?;
    if raw < 0 {
        let ref_delta = raw.checked_neg()? as usize;
        let ref_off = chunk_start.checked_add(ref_delta)?;
        let chunk_end = chunk_start.checked_add(header(buf).chunk_size as usize)?;
        let data_start = chunk_start.checked_add(CHUNK_HEADER_SIZE)?;
        if ref_off < data_start || ref_off.checked_add(4)? > chunk_end {
            return None;
        }
        let len = try_read_u32(buf, ref_off)? as usize;
        let value_start = ref_off.checked_add(4)?;
        let end = value_start.checked_add(len)?;
        if end > chunk_end || end > buf.len() {
            return None;
        }
        Some(&buf[value_start..end])
    } else {
        let len = raw as usize;
        let value_start = row_off.checked_add(4)?;
        let end = value_start.checked_add(len)?;
        Some(row_data.get(value_start..end)?)
    }
}

// ── Row / RowIter ───────────────────────────────────────────────────

/// Keeps a recyclable chunk's byte region immutable while rows borrow it.
pub(crate) struct ChunkReadPin<'a> {
    lease: AtomicPtr<ReaderLease>,
    owner_pid: AtomicU32,
    buf: &'a [u8],
    chunk: usize,
    chunk_start: usize,
    generation: u64,
}

impl Drop for ChunkReadPin<'_> {
    fn drop(&mut self) {
        let pid = current_process_id();
        // After fork, the child inherits the Arc allocation but did not
        // acquire the parent's shared ref. Unless it first adopted a child
        // lease in `ensure_current_process`, dropping it must not decrement
        // the parent's slot.
        if self.owner_pid.load(Ordering::Acquire) != pid {
            return;
        }
        let lease = self.lease.load(Ordering::Acquire);
        if !lease.is_null() {
            release_process_lease(unsafe { &*lease }, pid);
        }
    }
}

impl ChunkReadPin<'_> {
    /// Re-acquire the pin under the child PID after `fork`.
    ///
    /// The atomics in this Arc are process-private after fork, so changing
    /// them cannot disturb the parent's pin. The generation is checked after
    /// acquiring the child lease and before any inherited Row byte is read.
    fn ensure_current_process(&self) -> bool {
        const ADOPTING: u32 = 0;
        const INVALID: u32 = u32::MAX;
        let pid = current_process_id();
        loop {
            let owner = self.owner_pid.load(Ordering::Acquire);
            if owner == pid {
                return true;
            }
            if owner == INVALID {
                return false;
            }
            if owner == ADOPTING {
                std::hint::spin_loop();
                continue;
            }
            if self
                .owner_pid
                .compare_exchange(owner, ADOPTING, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                continue;
            }

            let Some(lease) = acquire_process_lease(self.buf, self.chunk) else {
                self.owner_pid.store(INVALID, Ordering::Release);
                return false;
            };
            let ch = chunk_header(self.buf, self.chunk_start);
            let valid = ch.state.load(Ordering::SeqCst) != ChunkState::Empty as u32
                && ch.generation.load(Ordering::SeqCst) == self.generation;
            if !valid {
                release_process_lease(lease, pid);
                self.owner_pid.store(INVALID, Ordering::Release);
                return false;
            }
            self.lease.store(
                lease as *const ReaderLease as *mut ReaderLease,
                Ordering::Release,
            );
            self.owner_pid.store(pid, Ordering::Release);
            return true;
        }
    }
}

fn release_process_lease(lease: &ReaderLease, owner_pid: u32) {
    loop {
        let state = lease.state.load(Ordering::SeqCst);
        let (owner, refs) = lease_parts(state);
        if owner != owner_pid || refs == 0 {
            return;
        }
        let next = if refs == 1 {
            0
        } else {
            pack_lease(owner, refs - 1)
        };
        if lease
            .state
            .compare_exchange(state, next, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            if next == 0 {
                fence(Ordering::SeqCst);
            }
            return;
        }
    }
}

#[inline]
fn pack_lease(owner_pid: u32, refs: u32) -> u64 {
    ((owner_pid as u64) << 32) | refs as u64
}

#[inline]
fn lease_parts(state: u64) -> (u32, u32) {
    ((state >> 32) as u32, state as u32)
}

fn acquire_process_lease(buf: &[u8], chunk: usize) -> Option<&ReaderLease> {
    let pid = current_process_id();
    let start_time = process_start_time(pid);
    let slots = header(buf).lease_slots as usize;

    for slot_idx in 0..slots {
        let lease = reader_lease(buf, chunk, slot_idx);
        loop {
            let state = lease.state.load(Ordering::SeqCst);
            let (owner, refs) = lease_parts(state);
            if owner == pid && refs > 0 {
                let recorded_start = lease.owner_start_time.load(Ordering::SeqCst);
                if recorded_start != start_time {
                    if recorded_start != 0 && start_time != 0 {
                        let _ = lease.state.compare_exchange(
                            state,
                            0,
                            Ordering::SeqCst,
                            Ordering::SeqCst,
                        );
                        continue;
                    }
                    break;
                }
                let next_refs = refs.checked_add(1)?;
                if lease
                    .state
                    .compare_exchange(
                        state,
                        pack_lease(pid, next_refs),
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    )
                    .is_ok()
                {
                    return Some(lease);
                }
                continue;
            }
            if state == 0 {
                let claiming = pack_lease(pid, 0);
                if lease
                    .state
                    .compare_exchange(0, claiming, Ordering::SeqCst, Ordering::SeqCst)
                    .is_err()
                {
                    continue;
                }
                lease.owner_start_time.store(start_time, Ordering::SeqCst);
                if lease
                    .state
                    .compare_exchange(
                        claiming,
                        pack_lease(pid, 1),
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    )
                    .is_ok()
                {
                    return Some(lease);
                }
                break;
            }
            let owner_dead = if refs == 0 {
                !process_exists(owner)
            } else {
                !process_instance_alive(owner, lease.owner_start_time.load(Ordering::SeqCst))
            };
            if owner_dead {
                let _ = lease
                    .state
                    .compare_exchange(state, 0, Ordering::SeqCst, Ordering::SeqCst);
                continue;
            }
            break;
        }
    }
    None
}

/// Returns whether a live process still owns a lease for `chunk`.
/// Dead-process slots are reclaimed atomically.
pub(crate) fn chunk_has_live_leases(buf: &[u8], chunk: usize) -> bool {
    let slots = header(buf).lease_slots as usize;
    let mut live = false;
    for slot_idx in 0..slots {
        let lease = reader_lease(buf, chunk, slot_idx);
        loop {
            let state = lease.state.load(Ordering::SeqCst);
            if state == 0 {
                break;
            }
            let (owner, refs) = lease_parts(state);
            // refs==0 is the short claim state. A live claimant blocks recycle.
            let start = lease.owner_start_time.load(Ordering::SeqCst);
            if (refs == 0 && process_exists(owner))
                || (refs > 0 && process_instance_alive(owner, start))
            {
                live = true;
                break;
            }
            if lease
                .state
                .compare_exchange(state, 0, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                break;
            }
            if refs == 0 {
                std::hint::spin_loop();
            }
        }
    }
    live
}

/// Pin a stable, readable chunk generation and capture its published extent.
pub(crate) fn pin_chunk<'a>(
    buf: &'a [u8],
    chunk_start: usize,
) -> crate::error::Result<Option<(Arc<ChunkReadPin<'a>>, u64, usize)>> {
    let ch = chunk_header(buf, chunk_start);
    let h = header(buf);
    let chunk = chunk_start.checked_sub(h.data_offset as usize).ok_or(
        crate::error::MemtableError::InvalidBuffer("chunk starts before data region"),
    )? / h.chunk_size as usize;
    let payload_capacity = header(buf)
        .chunk_size
        .checked_sub(CHUNK_HEADER_SIZE as u32)
        .ok_or(crate::error::MemtableError::InvalidBuffer(
            "chunk payload capacity underflow",
        ))? as usize;

    for _ in 0..64 {
        let state_before = ch.state.load(Ordering::SeqCst);
        if state_before == ChunkState::Empty as u32 {
            return Ok(None);
        }
        let generation_before = ch.generation.load(Ordering::SeqCst);
        let Some(lease) = acquire_process_lease(buf, chunk) else {
            return Err(crate::error::MemtableError::ReaderLeaseExhausted);
        };

        let state_after = ch.state.load(Ordering::SeqCst);
        let generation_after = ch.generation.load(Ordering::SeqCst);
        if state_before == state_after
            && state_after != ChunkState::Empty as u32
            && generation_before == generation_after
        {
            let used = ch.used.load(Ordering::Acquire) as usize;
            if used <= payload_capacity {
                return Ok(Some((
                    Arc::new(ChunkReadPin {
                        lease: AtomicPtr::new(lease as *const ReaderLease as *mut ReaderLease),
                        owner_pid: AtomicU32::new(current_process_id()),
                        buf,
                        chunk,
                        chunk_start,
                        generation: generation_after,
                    }),
                    generation_after,
                    used,
                )));
            }
        }

        release_process_lease(lease, current_process_id());
        std::hint::spin_loop();
    }
    Ok(None)
}

/// Read-only handle to a single row within a chunk.
///
/// The chunk generation is checked around each column read. Call
/// [`is_valid()`](Self::is_valid) after consuming borrowed string/byte
/// values when a writer may recycle the chunk concurrently.
pub struct Row<'a> {
    pub(crate) data: &'a [u8],
    pub(crate) buf: &'a [u8],
    pub(crate) chunk_start: usize,
    pub(crate) generation: u64,
    pub(crate) _pin: Arc<ChunkReadPin<'a>>,
}

impl<'a> Row<'a> {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Check whether the underlying chunk is still at the same generation.
    pub fn is_valid(&self) -> bool {
        self._pin.ensure_current_process()
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.data
    }

    fn try_col_offset(&self, col: usize) -> Option<usize> {
        if col >= header(self.buf).num_cols as usize {
            return None;
        }
        let mut off = 0usize;
        for i in 0..col {
            let dt = DType::from_u32(col_desc(self.buf, i).dtype)?;
            if let Some(sz) = dt.fixed_size() {
                off = off.checked_add(sz)?;
            } else {
                off = off.checked_add(try_var_field_size(self.data, off)?)?;
            }
        }
        Some(off)
    }

    fn try_resolve_var_col(&self, col: usize) -> Option<&[u8]> {
        let off = self.try_col_offset(col)?;
        try_resolve_var(self.data, off, self.buf, self.chunk_start)
    }

    pub fn col_u8(&self, col: usize) -> u8 {
        if !self.is_valid() {
            return 0;
        }
        let off = match self.try_col_offset(col) {
            Some(o) => o,
            None => return 0,
        };
        if off >= self.data.len() {
            return 0;
        }
        let value = self.data[off];
        if self.is_valid() {
            value
        } else {
            0
        }
    }
    pub fn col_u32(&self, col: usize) -> u32 {
        if !self.is_valid() {
            return 0;
        }
        let off = match self.try_col_offset(col) {
            Some(o) => o,
            None => return 0,
        };
        let value = try_read_u32(self.data, off).unwrap_or(0);
        if self.is_valid() {
            value
        } else {
            0
        }
    }
    pub fn col_i32(&self, col: usize) -> i32 {
        if !self.is_valid() {
            return 0;
        }
        let off = match self.try_col_offset(col) {
            Some(o) => o,
            None => return 0,
        };
        let value = try_read_i32(self.data, off).unwrap_or(0);
        if self.is_valid() {
            value
        } else {
            0
        }
    }
    pub fn col_i64(&self, col: usize) -> i64 {
        if !self.is_valid() {
            return 0;
        }
        let off = match self.try_col_offset(col) {
            Some(o) => o,
            None => return 0,
        };
        let value = try_read_i64(self.data, off).unwrap_or(0);
        if self.is_valid() {
            value
        } else {
            0
        }
    }
    pub fn col_f32(&self, col: usize) -> f32 {
        if !self.is_valid() {
            return 0.0;
        }
        let off = match self.try_col_offset(col) {
            Some(o) => o,
            None => return 0.0,
        };
        let value = try_read_f32(self.data, off).unwrap_or(0.0);
        if self.is_valid() {
            value
        } else {
            0.0
        }
    }
    pub fn col_f64(&self, col: usize) -> f64 {
        if !self.is_valid() {
            return 0.0;
        }
        let off = match self.try_col_offset(col) {
            Some(o) => o,
            None => return 0.0,
        };
        let value = try_read_f64(self.data, off).unwrap_or(0.0);
        if self.is_valid() {
            value
        } else {
            0.0
        }
    }
    pub fn col_u64(&self, col: usize) -> u64 {
        if !self.is_valid() {
            return 0;
        }
        let off = match self.try_col_offset(col) {
            Some(o) => o,
            None => return 0,
        };
        let value = try_read_u64(self.data, off).unwrap_or(0);
        if self.is_valid() {
            value
        } else {
            0
        }
    }
    pub fn col_str(&self, col: usize) -> &str {
        if !self.is_valid() {
            return "";
        }
        let b = match self.try_resolve_var_col(col) {
            Some(b) => b,
            None => return "",
        };
        if !self.is_valid() {
            return "";
        }
        if b.is_empty() {
            ""
        } else {
            std::str::from_utf8(b).unwrap_or("")
        }
    }
    pub fn col_bytes(&self, col: usize) -> &[u8] {
        if !self.is_valid() {
            return &[];
        }
        let value = self.try_resolve_var_col(col).unwrap_or(&[]);
        if self.is_valid() {
            value
        } else {
            &[]
        }
    }

    pub fn cursor(&self) -> RowCursor<'_> {
        RowCursor {
            data: self.data,
            pos: 0,
            buf: self.buf,
            chunk_start: self.chunk_start,
            generation: self.generation,
            pin: self._pin.as_ref(),
            stale: !self.is_valid(),
        }
    }
}

/// Sequential cursor over columns within a row — O(1) per column.
///
/// Generation is validated once per row by [`RowIter::next()`].
/// If the chunk is recycled mid-read, the cursor is marked stale and
/// subsequent reads return zero / empty values instead of panicking.
pub struct RowCursor<'a> {
    data: &'a [u8],
    pos: usize,
    buf: &'a [u8],
    chunk_start: usize,
    generation: u64,
    pin: &'a ChunkReadPin<'a>,
    stale: bool,
}

impl<'a> RowCursor<'a> {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Check whether the underlying chunk is still at the same generation.
    pub fn is_valid(&self) -> bool {
        !self.stale && self.generation_matches()
    }

    /// Returns `true` if the chunk was recycled or a torn read was detected.
    pub fn is_stale(&self) -> bool {
        self.stale || !self.generation_matches()
    }

    fn generation_matches(&self) -> bool {
        self.pin.ensure_current_process()
    }

    fn mark_stale(&mut self) {
        self.stale = true;
    }

    fn read_fixed<const N: usize>(&mut self) -> [u8; N] {
        if self.stale || !self.generation_matches() {
            self.mark_stale();
            return [0u8; N];
        }
        let Some(end) = self.pos.checked_add(N) else {
            self.mark_stale();
            return [0u8; N];
        };
        if end > self.data.len() {
            self.mark_stale();
            return [0u8; N];
        }
        let mut v = [0u8; N];
        v.copy_from_slice(&self.data[self.pos..end]);
        self.pos += N;
        if self.generation_matches() {
            v
        } else {
            self.mark_stale();
            [0u8; N]
        }
    }

    fn read_lp(&mut self) -> &'a [u8] {
        if self.stale {
            return &[];
        }
        let raw = i32::from_le_bytes(self.read_fixed::<4>());
        if self.stale {
            return &[];
        }
        if raw < 0 {
            let Some(ref_delta) = raw.checked_neg().map(|v| v as usize) else {
                self.mark_stale();
                return &[];
            };
            let Some(ref_off) = self.chunk_start.checked_add(ref_delta) else {
                self.mark_stale();
                return &[];
            };
            let chunk_end = self.chunk_start + header(self.buf).chunk_size as usize;
            if ref_off < self.chunk_start + CHUNK_HEADER_SIZE || ref_off + 4 > chunk_end {
                self.mark_stale();
                return &[];
            }
            let Some(len) = try_read_u32(self.buf, ref_off).map(|v| v as usize) else {
                self.mark_stale();
                return &[];
            };
            let Some(end) = ref_off.checked_add(4).and_then(|v| v.checked_add(len)) else {
                self.mark_stale();
                return &[];
            };
            if end > chunk_end || end > self.buf.len() {
                self.mark_stale();
                return &[];
            }
            let value = &self.buf[ref_off + 4..end];
            if self.generation_matches() {
                value
            } else {
                self.mark_stale();
                &[]
            }
        } else {
            let len = raw as usize;
            let Some(end) = self.pos.checked_add(len) else {
                self.mark_stale();
                return &[];
            };
            if end > self.data.len() {
                self.mark_stale();
                return &[];
            }
            let data = &self.data[self.pos..end];
            self.pos = end;
            if self.generation_matches() {
                data
            } else {
                self.mark_stale();
                &[]
            }
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
        let b = self.read_lp();
        if b.is_empty() {
            ""
        } else {
            std::str::from_utf8(b).unwrap_or("")
        }
    }
    pub fn next_bytes(&mut self) -> &'a [u8] {
        self.read_lp()
    }
}

/// Iterator over rows in a chunk.
///
/// Captures the chunk's `generation` at creation time.  Each call to
/// [`next()`](Iterator::next) checks generation **once**; if the chunk
/// was recycled it returns [`None`].  Column reads on the yielded [`Row`]
/// / [`RowCursor`] do **not** re-check, keeping the per-column path free
/// of atomic loads.
pub struct RowIter<'a> {
    pub(crate) buf: &'a [u8],
    pub(crate) chunk_start: usize,
    pub(crate) pos: usize,
    pub(crate) end: usize,
    pub(crate) generation: u64,
    pub(crate) pin: Option<Arc<ChunkReadPin<'a>>>,
}

impl<'a> RowIter<'a> {
    /// The chunk generation captured when this iterator was created.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns `true` if the chunk's generation still matches the snapshot.
    /// A mismatch means the chunk was recycled and data may be stale.
    pub fn is_valid(&self) -> bool {
        self.pin
            .as_ref()
            .is_some_and(|pin| pin.ensure_current_process())
    }
}

impl<'a> Iterator for RowIter<'a> {
    type Item = Row<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        while self.pos + 4 <= self.end {
            if !self.is_valid() {
                return None;
            }
            let row_len = r32(self.buf, self.pos) as usize;
            let row_total = 4usize.saturating_add(row_len);
            if row_total > self.end.saturating_sub(self.pos) {
                return None;
            }
            let row_end = self.pos + row_total;
            let data_offset = self.pos + 4;
            self.pos = row_end;
            if self.is_valid() {
                return Some(Row {
                    data: &self.buf[data_offset..row_end],
                    buf: self.buf,
                    chunk_start: self.chunk_start,
                    generation: self.generation,
                    _pin: Arc::clone(self.pin.as_ref()?),
                });
            }
            // Chunk recycled while parsing this row — skip it and continue.
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::memtable::{MemTable, MemTableWriter};
    use crate::schema::{DType, Schema, Value};

    #[test]
    fn row_raw_bytes() {
        let schema = Schema::new().col("v", DType::I32);
        let mut t = MemTable::new(&schema, 1024, 1).unwrap();
        t.push_row(&[Value::I32(0x12345678)]);
        assert_eq!(
            t.rows(0).next().unwrap().as_bytes(),
            &0x12345678_i32.to_le_bytes()
        );
    }

    #[test]
    fn row_cursor_basic() {
        let schema = Schema::new()
            .col("a", DType::I64)
            .col("b", DType::Str)
            .col("c", DType::F64)
            .col("d", DType::Bytes);
        let mut t = MemTable::new(&schema, 4096, 1).unwrap();
        t.row_writer()
            .put_i64(42)
            .put_str("test")
            .put_f64(3.14)
            .put_bytes(&[1, 2, 3])
            .finish();
        let row = t.rows(0).next().unwrap();
        let mut c = row.cursor();
        assert_eq!(c.next_i64(), 42);
        assert_eq!(c.next_str(), "test");
        assert_eq!(c.next_f64(), 3.14);
        assert_eq!(c.next_bytes(), &[1, 2, 3]);
    }

    #[test]
    fn cursor_multiple_rows() {
        let schema = Schema::new().col("id", DType::I32).col("name", DType::Str);
        let mut t = MemTable::new(&schema, 4096, 1).unwrap();
        for i in 0..5 {
            t.row_writer()
                .put_i32(i)
                .put_str(&format!("item_{i}"))
                .finish();
        }
        for (i, row) in t.rows(0).enumerate() {
            let mut c = row.cursor();
            assert_eq!(c.next_i32(), i as i32);
            assert_eq!(c.next_str(), format!("item_{i}"));
        }
    }
    #[test]
    fn row_iter_is_valid_detects_wrap() {
        let schema = Schema::new().col("v", DType::I32);
        let size = MemTable::required_size(&schema, 80, 2);
        let mut buf = vec![0u8; size];
        let mut mt = MemTableWriter::init(&mut buf, &schema, 80, 2).unwrap();

        for i in 0..3 {
            mt.push_row(&[Value::I32(i)]);
        }

        // Capture generation of chunk 0
        let gen0 = mt.chunk_generation(0);

        // Advance twice: chunk 0 gets recycled
        mt.advance_chunk();
        mt.advance_chunk();

        // Generation changed → stale
        assert_ne!(mt.chunk_generation(0), gen0);
        assert_eq!(mt.chunk_generation(0), gen0 + 1);
    }
    #[test]
    fn row_pins_chunk_until_drop() {
        use super::chunk_has_live_leases;

        let schema = Schema::new().col("v", DType::I64);
        let mut t = MemTable::new(&schema, 80, 2).unwrap();
        t.push_row(&[Value::I64(1)]);
        let row = t.rows(0).next().unwrap();
        assert_eq!(row.col_i64(0), 1);
        assert!(chunk_has_live_leases(t.as_bytes(), 0));
        drop(row);
        assert!(!chunk_has_live_leases(t.as_bytes(), 0));
    }

    #[test]
    fn row_pin_survives_transient_recycling_state() {
        use crate::layout::{chunk_header, ChunkState};
        use crate::memtable::MemTableView;
        use std::sync::atomic::Ordering;

        let schema = Schema::new().col("v", DType::I64);
        let size = MemTable::required_size(&schema, 80, 2);
        let mut buf = vec![0u8; size];
        MemTableWriter::init(&mut buf, &schema, 80, 2)
            .unwrap()
            .push_row(&[Value::I64(42)]);
        let reader = unsafe { std::slice::from_raw_parts(buf.as_ptr(), buf.len()) };
        let view = MemTableView::new(reader).unwrap();
        let row = view.rows(0).next().unwrap();

        assert_eq!(row.col_i64(99), 0);
        chunk_header(reader, row.chunk_start)
            .state
            .store(ChunkState::Empty as u32, Ordering::Release);
        assert!(row.is_valid());
        assert_eq!(row.col_i64(0), 42);
        assert!(!row.cursor().is_stale());
    }

    #[test]
    fn row_col_degrades_on_torn_bounds_without_panic() {
        use super::Row;

        let schema = Schema::new().col("v", DType::I64);
        let mut t = MemTable::new(&schema, 4096, 1).unwrap();
        t.push_row(&[Value::I64(42)]);
        let row = t.rows(0).next().unwrap();
        // Truncate row data to simulate torn read while generation is still valid.
        let torn = Row {
            data: &row.data[..4],
            buf: row.buf,
            chunk_start: row.chunk_start,
            generation: row.generation,
            _pin: std::sync::Arc::clone(&row._pin),
        };
        assert!(torn.is_valid());
        assert_eq!(torn.col_i64(0), 0);
    }

    #[cfg(unix)]
    #[test]
    fn inherited_row_reacquires_lease_after_fork() {
        use std::os::fd::RawFd;

        unsafe fn write_byte(fd: RawFd) {
            let byte = [1u8];
            assert_eq!(unsafe { libc::write(fd, byte.as_ptr().cast(), 1) }, 1);
        }
        unsafe fn read_byte(fd: RawFd) {
            let mut byte = [0u8];
            assert_eq!(unsafe { libc::read(fd, byte.as_mut_ptr().cast(), 1) }, 1);
        }

        let schema = Schema::new().col("v", DType::I64);
        let dir = std::env::temp_dir().join(format!(
            "probing-memt-fork-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("table.memt");
        let mut t = MemTable::file_at(&path, &schema, 80, 2).unwrap();
        assert!(t.push_row(&[Value::I64(42)]));
        let row = t.rows(0).next().unwrap();

        let mut ready = [0; 2];
        let mut proceed = [0; 2];
        assert_eq!(unsafe { libc::pipe(ready.as_mut_ptr()) }, 0);
        assert_eq!(unsafe { libc::pipe(proceed.as_mut_ptr()) }, 0);
        let child = unsafe { libc::fork() };
        assert!(child >= 0);
        if child == 0 {
            unsafe {
                libc::close(ready[0]);
                libc::close(proceed[1]);
            }
            // First access under the child PID must acquire an independent
            // shared lease before touching the inherited row bytes.
            let ok = row.col_i64(0) == 42;
            unsafe { write_byte(ready[1]) };
            unsafe { read_byte(proceed[0]) };
            let stable = row.col_i64(0) == 42;
            unsafe { libc::_exit(if ok && stable { 0 } else { 1 }) };
        }

        unsafe {
            libc::close(ready[1]);
            libc::close(proceed[0]);
            read_byte(ready[0]);
        }
        drop(row);
        assert!(t.advance_chunk());
        assert!(!t.advance_chunk(), "child lease must block chunk recycle");
        unsafe {
            write_byte(proceed[1]);
            let mut status = 0;
            assert_eq!(libc::waitpid(child, &mut status, 0), child);
            assert!(libc::WIFEXITED(status));
            assert_eq!(libc::WEXITSTATUS(status), 0);
            libc::close(ready[0]);
            libc::close(proceed[1]);
        }
        assert!(
            t.advance_chunk(),
            "child drop/_exit makes its lease reclaimable"
        );
        drop(t);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
