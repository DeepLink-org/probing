//! MEMC v1 binary layout: segment header, block headers, footer.
//!
//! All multi-byte fields are little-endian. See [`super`] (module docs)
//! for the full format walkthrough.

use crate::schema::DType;
use xxhash_rust::xxh3::xxh3_64;

/// Segment file magic: ASCII bytes `M E M C` in little-endian order.
pub const MAGIC_MEMC: u32 = u32::from_le_bytes(*b"MEMC");
/// Table-definition block magic.
pub const MAGIC_TABLE_BLOCK: u32 = u32::from_le_bytes(*b"MCTB");
/// Page (data) block magic.
pub const MAGIC_PAGE_BLOCK: u32 = u32::from_le_bytes(*b"MCPG");
/// Footer magic.
pub const MAGIC_FOOTER: u32 = u32::from_le_bytes(*b"MCFT");

/// MEMC format version.
pub const VERSION_MEMC: u16 = 1;

/// Segment header size (one cache line, mirrors MEMT/MEMH).
pub const SEGMENT_HEADER_SIZE: usize = 64;
/// Block header size; blocks start 64-aligned.
pub const BLOCK_HEADER_SIZE: usize = 64;
/// Fixed size of one page-directory entry in the footer.
pub const PAGE_DIR_ENTRY_SIZE: usize = 56;

/// `flags` bit: segment is sealed (footer present, file immutable).
pub const FLAG_SEALED: u16 = 1 << 0;

/// Sentinels for "no timestamp column / no rows yet" (match the hot ring).
pub const TS_MIN_INIT: i64 = i64::MAX;
pub const TS_MAX_INIT: i64 = i64::MIN;

/// Pco compression level for numeric columns (pco default).
pub const PCO_LEVEL: usize = 8;

/// Column encoding inside a page payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ColEncoding {
    /// Plain little-endian array of the fixed-size type.
    RawFixed = 0,
    /// Pco-compressed numeric column.
    Pco = 1,
    /// Concatenated `[u32 len][bytes]` entries (Str / Bytes).
    RawVarLen = 2,
}

impl ColEncoding {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::RawFixed),
            1 => Some(Self::Pco),
            2 => Some(Self::RawVarLen),
            _ => None,
        }
    }
}

/// Low 32 bits of xxh3-64 — the integrity check used throughout MEMC.
#[inline]
pub fn xxh32(bytes: &[u8]) -> u32 {
    xxh3_64(bytes) as u32
}

#[inline]
pub fn align64(n: usize) -> usize {
    (n + 63) & !63
}

// ── byte helpers (encode into Vec / decode from slice) ───────────────

#[inline]
pub fn get_u16(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(buf[off..off + 2].try_into().unwrap())
}
#[inline]
pub fn get_u32(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(buf[off..off + 4].try_into().unwrap())
}
#[inline]
pub fn get_u64(buf: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(buf[off..off + 8].try_into().unwrap())
}
#[inline]
pub fn get_i64(buf: &[u8], off: usize) -> i64 {
    i64::from_le_bytes(buf[off..off + 8].try_into().unwrap())
}
#[inline]
pub fn put_u16(buf: &mut [u8], off: usize, v: u16) {
    buf[off..off + 2].copy_from_slice(&v.to_le_bytes());
}
#[inline]
pub fn put_u32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}
#[inline]
pub fn put_u64(buf: &mut [u8], off: usize, v: u64) {
    buf[off..off + 8].copy_from_slice(&v.to_le_bytes());
}
#[inline]
pub fn put_i64(buf: &mut [u8], off: usize, v: i64) {
    buf[off..off + 8].copy_from_slice(&v.to_le_bytes());
}

// ── segment header ────────────────────────────────────────────────────

/// Parsed segment header.
///
/// ```text
/// offset size field
///  0      4   magic            "MEMC"
///  4      2   version          1
///  6      2   header_size      64
///  8      2   byte_order       BOM [0x01, 0x02]
/// 10      2   flags            bit0 = SEALED
/// 12      4   writer_pid
/// 16      8   writer_start     creator process start time
/// 24      8   created_unix_ms
/// 32      8   footer_off       0 until sealed
/// 40      8   ts_min           segment-wide (valid when sealed)
/// 48      8   ts_max
/// 56      4   page_count       valid when sealed
/// 60      4   header_xxh       xxh32 of bytes 0..60
/// ```
#[derive(Debug, Clone)]
pub struct SegmentHeader {
    pub flags: u16,
    pub writer_pid: u32,
    pub writer_start: u64,
    pub created_unix_ms: u64,
    pub footer_off: u64,
    pub ts_min: i64,
    pub ts_max: i64,
    pub page_count: u32,
}

impl SegmentHeader {
    pub fn is_sealed(&self) -> bool {
        self.flags & FLAG_SEALED != 0
    }

    pub fn encode(&self) -> [u8; SEGMENT_HEADER_SIZE] {
        let mut b = [0u8; SEGMENT_HEADER_SIZE];
        put_u32(&mut b, 0, MAGIC_MEMC);
        put_u16(&mut b, 4, VERSION_MEMC);
        put_u16(&mut b, 6, SEGMENT_HEADER_SIZE as u16);
        b[8..10].copy_from_slice(&[0x01, 0x02]);
        put_u16(&mut b, 10, self.flags);
        put_u32(&mut b, 12, self.writer_pid);
        put_u64(&mut b, 16, self.writer_start);
        put_u64(&mut b, 24, self.created_unix_ms);
        put_u64(&mut b, 32, self.footer_off);
        put_i64(&mut b, 40, self.ts_min);
        put_i64(&mut b, 48, self.ts_max);
        put_u32(&mut b, 56, self.page_count);
        let h = xxh32(&b[..60]);
        put_u32(&mut b, 60, h);
        b
    }

    pub fn decode(buf: &[u8]) -> Result<Self, &'static str> {
        if buf.len() < SEGMENT_HEADER_SIZE {
            return Err("buffer too small for MEMC header");
        }
        if get_u32(buf, 0) != MAGIC_MEMC {
            return Err("invalid MEMC magic");
        }
        if get_u16(buf, 4) != VERSION_MEMC {
            return Err("unsupported MEMC version");
        }
        if get_u16(buf, 6) as usize != SEGMENT_HEADER_SIZE {
            return Err("invalid MEMC header size");
        }
        if buf[8..10] != [0x01, 0x02] {
            return Err("byte order mismatch");
        }
        if get_u32(buf, 60) != xxh32(&buf[..60]) {
            return Err("MEMC header checksum mismatch");
        }
        Ok(Self {
            flags: get_u16(buf, 10),
            writer_pid: get_u32(buf, 12),
            writer_start: get_u64(buf, 16),
            created_unix_ms: get_u64(buf, 24),
            footer_off: get_u64(buf, 32),
            ts_min: get_i64(buf, 40),
            ts_max: get_i64(buf, 48),
            page_count: get_u32(buf, 56),
        })
    }
}

// ── block header (table-definition and page blocks) ──────────────────

/// Header shared by `MCTB` (table definition) and `MCPG` (page) blocks.
///
/// ```text
/// offset size field
///  0      4   block magic      "MCTB" / "MCPG"
///  4      4   table_id
///  8      4   row_count        (MCTB: 0)
/// 12      4   col_count
/// 16      8   ts_min           (MCTB: TS_MIN_INIT)
/// 24      8   ts_max           (MCTB: TS_MAX_INIT)
/// 32      8   source_gen       hot-ring chunk generation this page drained (0 = n/a)
/// 40      4   payload_len
/// 44      4   payload_xxh      xxh32 of payload bytes
/// 48      4   source_chunk     hot-ring chunk index this page drained (u32::MAX = n/a)
/// 52      4   header_xxh       xxh32 of bytes 0..52
/// 56      8   reserved (zero)
/// ```
///
/// `source_gen` + `source_chunk` together identify the hot-ring chunk a page
/// was compacted from, letting a restarting compactor rebuild its per-chunk
/// drain watermark from existing cold pages (exactly-once across restarts).
///
/// The payload follows the header and is padded to the next 64-byte
/// boundary; the padding is excluded from `payload_xxh`.
#[derive(Debug, Clone)]
pub struct BlockHeader {
    pub magic: u32,
    pub table_id: u32,
    pub row_count: u32,
    pub col_count: u32,
    pub ts_min: i64,
    pub ts_max: i64,
    pub source_gen: u64,
    pub payload_len: u32,
    pub payload_xxh: u32,
    pub source_chunk: u32,
}

/// Sentinel for "this page did not originate from a specific hot-ring chunk".
pub const SOURCE_CHUNK_NONE: u32 = u32::MAX;

impl BlockHeader {
    pub fn encode(&self) -> [u8; BLOCK_HEADER_SIZE] {
        let mut b = [0u8; BLOCK_HEADER_SIZE];
        put_u32(&mut b, 0, self.magic);
        put_u32(&mut b, 4, self.table_id);
        put_u32(&mut b, 8, self.row_count);
        put_u32(&mut b, 12, self.col_count);
        put_i64(&mut b, 16, self.ts_min);
        put_i64(&mut b, 24, self.ts_max);
        put_u64(&mut b, 32, self.source_gen);
        put_u32(&mut b, 40, self.payload_len);
        put_u32(&mut b, 44, self.payload_xxh);
        put_u32(&mut b, 48, self.source_chunk);
        let h = xxh32(&b[..52]);
        put_u32(&mut b, 52, h);
        b
    }

    /// Decode and verify the header checksum. The payload checksum is
    /// verified separately, against the actual payload bytes.
    pub fn decode(buf: &[u8]) -> Option<Self> {
        if buf.len() < BLOCK_HEADER_SIZE {
            return None;
        }
        let magic = get_u32(buf, 0);
        if magic != MAGIC_TABLE_BLOCK && magic != MAGIC_PAGE_BLOCK {
            return None;
        }
        if get_u32(buf, 52) != xxh32(&buf[..52]) {
            return None;
        }
        Some(Self {
            magic,
            table_id: get_u32(buf, 4),
            row_count: get_u32(buf, 8),
            col_count: get_u32(buf, 12),
            ts_min: get_i64(buf, 16),
            ts_max: get_i64(buf, 24),
            source_gen: get_u64(buf, 32),
            payload_len: get_u32(buf, 40),
            payload_xxh: get_u32(buf, 44),
            source_chunk: get_u32(buf, 48),
        })
    }
}

// ── table-definition payload ──────────────────────────────────────────

/// In-memory table definition (parsed from an `MCTB` payload).
#[derive(Debug, Clone)]
pub struct TableDef {
    pub id: u32,
    pub name: String,
    pub cols: Vec<(String, DType)>,
    /// Index of the designated timestamp column, per the hot-ring
    /// convention (`I64` column named `timestamp` / `ts`).
    pub ts_col: Option<usize>,
}

/// Encode a table definition payload:
/// `[u16 name_len][u16 col_count][name]` then per column
/// `[u8 dtype][u8 0][u16 name_len][name]`.
pub fn encode_table_payload(name: &str, cols: &[(String, DType)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + name.len() + cols.len() * 16);
    out.extend_from_slice(&(name.len() as u16).to_le_bytes());
    out.extend_from_slice(&(cols.len() as u16).to_le_bytes());
    out.extend_from_slice(name.as_bytes());
    for (cname, dtype) in cols {
        out.push(*dtype as u32 as u8);
        out.push(0);
        out.extend_from_slice(&(cname.len() as u16).to_le_bytes());
        out.extend_from_slice(cname.as_bytes());
    }
    out
}

pub fn decode_table_payload(id: u32, payload: &[u8]) -> Result<TableDef, &'static str> {
    if payload.len() < 4 {
        return Err("table payload too small");
    }
    let name_len = get_u16(payload, 0) as usize;
    let col_count = get_u16(payload, 2) as usize;
    let mut off = 4;
    if payload.len() < off + name_len {
        return Err("table name out of bounds");
    }
    let name = std::str::from_utf8(&payload[off..off + name_len])
        .map_err(|_| "table name not utf-8")?
        .to_string();
    off += name_len;

    let mut cols = Vec::with_capacity(col_count);
    for _ in 0..col_count {
        if payload.len() < off + 4 {
            return Err("column entry out of bounds");
        }
        let dtype = DType::from_u32(payload[off] as u32).ok_or("invalid column dtype")?;
        let cname_len = get_u16(payload, off + 2) as usize;
        off += 4;
        if payload.len() < off + cname_len {
            return Err("column name out of bounds");
        }
        let cname = std::str::from_utf8(&payload[off..off + cname_len])
            .map_err(|_| "column name not utf-8")?
            .to_string();
        off += cname_len;
        cols.push((cname, dtype));
    }

    let ts_col = cols
        .iter()
        .position(|(n, dt)| *dt == DType::I64 && crate::raw::TS_COL_NAMES.contains(&n.as_str()));
    Ok(TableDef {
        id,
        name,
        cols,
        ts_col,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magics_are_distinct_from_hot_formats() {
        assert_ne!(MAGIC_MEMC, crate::MAGIC_MEMT);
        assert_ne!(MAGIC_MEMC, crate::MAGIC_MEMH);
        assert_ne!(MAGIC_TABLE_BLOCK, MAGIC_PAGE_BLOCK);
    }

    #[test]
    fn segment_header_roundtrip() {
        let h = SegmentHeader {
            flags: FLAG_SEALED,
            writer_pid: 1234,
            writer_start: 99,
            created_unix_ms: 1_700_000_000_000,
            footer_off: 4096,
            ts_min: -5,
            ts_max: 500,
            page_count: 7,
        };
        let bytes = h.encode();
        let d = SegmentHeader::decode(&bytes).unwrap();
        assert!(d.is_sealed());
        assert_eq!(d.writer_pid, 1234);
        assert_eq!(d.footer_off, 4096);
        assert_eq!((d.ts_min, d.ts_max), (-5, 500));
        assert_eq!(d.page_count, 7);

        // Corruption is detected
        let mut bad = bytes;
        bad[12] ^= 0xFF;
        assert!(SegmentHeader::decode(&bad).is_err());
    }

    #[test]
    fn block_header_roundtrip_and_corruption() {
        let h = BlockHeader {
            magic: MAGIC_PAGE_BLOCK,
            table_id: 3,
            row_count: 100,
            col_count: 2,
            ts_min: 10,
            ts_max: 20,
            source_gen: 42,
            payload_len: 512,
            payload_xxh: 0xDEAD,
            source_chunk: 6,
        };
        let bytes = h.encode();
        let d = BlockHeader::decode(&bytes).unwrap();
        assert_eq!(d.table_id, 3);
        assert_eq!(d.source_gen, 42);
        assert_eq!(d.source_chunk, 6);

        let mut bad = bytes;
        bad[8] ^= 1;
        assert!(BlockHeader::decode(&bad).is_none());
    }

    #[test]
    fn table_payload_roundtrip() {
        let cols = vec![
            ("timestamp".to_string(), DType::I64),
            ("value".to_string(), DType::F64),
            ("tag".to_string(), DType::Str),
        ];
        let payload = encode_table_payload("metrics", &cols);
        let def = decode_table_payload(5, &payload).unwrap();
        assert_eq!(def.name, "metrics");
        assert_eq!(def.cols.len(), 3);
        assert_eq!(def.cols[2].1, DType::Str);
        assert_eq!(def.ts_col, Some(0), "timestamp I64 column detected");
    }
}
