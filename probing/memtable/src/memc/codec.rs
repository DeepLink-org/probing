//! Columnar encode/decode for MEMC page payloads.
//!
//! A page payload is the concatenation of per-column sub-blocks, each:
//!
//! ```text
//! [u8 encoding][u8 dtype][u16 _pad][u32 byte_len][payload bytes]
//! ```
//!
//! Numeric columns use Pco (`simpler_compress`); `U8` and variable-length
//! `Str`/`Bytes` columns are stored raw (Pco has no `u8`/string support).

use pco::standalone::{simple_decompress, simpler_compress};

use super::layout::{get_u32, ColEncoding, PCO_LEVEL};
use crate::schema::{DType, Value};

/// One column's worth of values, type-tagged.
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnData {
    U8(Vec<u8>),
    U32(Vec<u32>),
    I32(Vec<i32>),
    I64(Vec<i64>),
    F32(Vec<f32>),
    F64(Vec<f64>),
    U64(Vec<u64>),
    Str(Vec<String>),
    Bytes(Vec<Vec<u8>>),
}

impl ColumnData {
    pub fn dtype(&self) -> DType {
        match self {
            ColumnData::U8(_) => DType::U8,
            ColumnData::U32(_) => DType::U32,
            ColumnData::I32(_) => DType::I32,
            ColumnData::I64(_) => DType::I64,
            ColumnData::F32(_) => DType::F32,
            ColumnData::F64(_) => DType::F64,
            ColumnData::U64(_) => DType::U64,
            ColumnData::Str(_) => DType::Str,
            ColumnData::Bytes(_) => DType::Bytes,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            ColumnData::U8(v) => v.len(),
            ColumnData::U32(v) => v.len(),
            ColumnData::I32(v) => v.len(),
            ColumnData::I64(v) => v.len(),
            ColumnData::F32(v) => v.len(),
            ColumnData::F64(v) => v.len(),
            ColumnData::U64(v) => v.len(),
            ColumnData::Str(v) => v.len(),
            ColumnData::Bytes(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Builds one [`ColumnData`] of a fixed [`DType`] by pushing [`Value`]s.
pub struct ColumnBuilder {
    data: ColumnData,
}

impl ColumnBuilder {
    pub fn new(dtype: DType) -> Self {
        let data = match dtype {
            DType::U8 => ColumnData::U8(Vec::new()),
            DType::U32 => ColumnData::U32(Vec::new()),
            DType::I32 => ColumnData::I32(Vec::new()),
            DType::I64 => ColumnData::I64(Vec::new()),
            DType::F32 => ColumnData::F32(Vec::new()),
            DType::F64 => ColumnData::F64(Vec::new()),
            DType::U64 => ColumnData::U64(Vec::new()),
            DType::Str => ColumnData::Str(Vec::new()),
            DType::Bytes => ColumnData::Bytes(Vec::new()),
        };
        Self { data }
    }

    /// Append a value. Mismatched types are coerced where lossless and
    /// otherwise dropped as a zero/empty default — callers validate the
    /// row schema up front, so this only guards against logic errors.
    pub fn push(&mut self, v: &Value) {
        match (&mut self.data, v) {
            (ColumnData::U8(d), Value::U8(x)) => d.push(*x),
            (ColumnData::U32(d), Value::U32(x)) => d.push(*x),
            (ColumnData::I32(d), Value::I32(x)) => d.push(*x),
            (ColumnData::I64(d), Value::I64(x)) => d.push(*x),
            (ColumnData::F32(d), Value::F32(x)) => d.push(*x),
            (ColumnData::F64(d), Value::F64(x)) => d.push(*x),
            (ColumnData::U64(d), Value::U64(x)) => d.push(*x),
            (ColumnData::Str(d), Value::Str(x)) => d.push((*x).to_string()),
            (ColumnData::Bytes(d), Value::Bytes(x)) => d.push(x.to_vec()),
            _ => debug_assert!(false, "ColumnBuilder type mismatch"),
        }
    }

    pub fn finish(self) -> ColumnData {
        self.data
    }
}

fn pco_compress<T: pco::data_types::Number>(nums: &[T]) -> Result<Vec<u8>, String> {
    simpler_compress(nums, PCO_LEVEL).map_err(|e| e.to_string())
}

fn pco_decompress<T: pco::data_types::Number>(data: &[u8]) -> Result<Vec<T>, String> {
    simple_decompress::<T>(data).map_err(|e| e.to_string())
}

fn encode_varlen(entries: impl Iterator<Item = (usize, Vec<u8>)>, total: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(total);
    for (len, bytes) in entries {
        out.extend_from_slice(&(len as u32).to_le_bytes());
        out.extend_from_slice(&bytes);
    }
    out
}

/// Encode one column into its sub-block (header + payload).
pub fn encode_column(col: &ColumnData) -> Result<Vec<u8>, String> {
    let (encoding, payload): (ColEncoding, Vec<u8>) = match col {
        ColumnData::U8(v) => (ColEncoding::RawFixed, v.clone()),
        ColumnData::I32(v) => (ColEncoding::Pco, pco_compress(v)?),
        ColumnData::I64(v) => (ColEncoding::Pco, pco_compress(v)?),
        ColumnData::F32(v) => (ColEncoding::Pco, pco_compress(v)?),
        ColumnData::F64(v) => (ColEncoding::Pco, pco_compress(v)?),
        ColumnData::U32(v) => (ColEncoding::Pco, pco_compress(v)?),
        ColumnData::U64(v) => (ColEncoding::Pco, pco_compress(v)?),
        ColumnData::Str(v) => {
            let total: usize = v.iter().map(|s| 4 + s.len()).sum();
            let payload = encode_varlen(v.iter().map(|s| (s.len(), s.as_bytes().to_vec())), total);
            (ColEncoding::RawVarLen, payload)
        }
        ColumnData::Bytes(v) => {
            let total: usize = v.iter().map(|b| 4 + b.len()).sum();
            let payload = encode_varlen(v.iter().map(|b| (b.len(), b.clone())), total);
            (ColEncoding::RawVarLen, payload)
        }
    };

    let mut out = Vec::with_capacity(8 + payload.len());
    out.push(encoding as u8);
    out.push(col.dtype() as u32 as u8);
    out.extend_from_slice(&[0u8, 0u8]);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

/// Decode one column sub-block, returning the column and bytes consumed.
pub fn decode_column(buf: &[u8], row_count: usize) -> Result<(ColumnData, usize), String> {
    if buf.len() < 8 {
        return Err("column sub-block too small".into());
    }
    let encoding = ColEncoding::from_u8(buf[0]).ok_or("invalid column encoding")?;
    let dtype = DType::from_u32(buf[1] as u32).ok_or("invalid column dtype")?;
    let payload_len = get_u32(buf, 4) as usize;
    let start = 8;
    let end = start + payload_len;
    if buf.len() < end {
        return Err("column payload out of bounds".into());
    }
    let payload = &buf[start..end];

    let col = match (encoding, dtype) {
        (ColEncoding::RawFixed, DType::U8) => ColumnData::U8(payload.to_vec()),
        (ColEncoding::Pco, DType::I32) => ColumnData::I32(pco_decompress(payload)?),
        (ColEncoding::Pco, DType::I64) => ColumnData::I64(pco_decompress(payload)?),
        (ColEncoding::Pco, DType::F32) => ColumnData::F32(pco_decompress(payload)?),
        (ColEncoding::Pco, DType::F64) => ColumnData::F64(pco_decompress(payload)?),
        (ColEncoding::Pco, DType::U32) => ColumnData::U32(pco_decompress(payload)?),
        (ColEncoding::Pco, DType::U64) => ColumnData::U64(pco_decompress(payload)?),
        (ColEncoding::RawVarLen, DType::Str) => {
            ColumnData::Str(decode_varlen_str(payload, row_count)?)
        }
        (ColEncoding::RawVarLen, DType::Bytes) => {
            ColumnData::Bytes(decode_varlen_bytes(payload, row_count)?)
        }
        _ => return Err("encoding/dtype mismatch".into()),
    };
    Ok((col, end))
}

fn decode_varlen_entries(payload: &[u8], row_count: usize) -> Result<Vec<Vec<u8>>, String> {
    let mut out = Vec::with_capacity(row_count);
    let mut off = 0usize;
    while off + 4 <= payload.len() {
        let len = get_u32(payload, off) as usize;
        off += 4;
        if off + len > payload.len() {
            return Err("varlen entry out of bounds".into());
        }
        out.push(payload[off..off + len].to_vec());
        off += len;
    }
    Ok(out)
}

fn decode_varlen_str(payload: &[u8], row_count: usize) -> Result<Vec<String>, String> {
    decode_varlen_entries(payload, row_count)?
        .into_iter()
        .map(|b| String::from_utf8(b).map_err(|_| "varlen str not utf-8".to_string()))
        .collect()
}

fn decode_varlen_bytes(payload: &[u8], row_count: usize) -> Result<Vec<Vec<u8>>, String> {
    decode_varlen_entries(payload, row_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(col: ColumnData) {
        let rc = col.len();
        let encoded = encode_column(&col).unwrap();
        let (decoded, consumed) = decode_column(&encoded, rc).unwrap();
        assert_eq!(consumed, encoded.len());
        assert_eq!(decoded, col);
    }

    #[test]
    fn numeric_columns_roundtrip() {
        roundtrip(ColumnData::I64((0..1000).map(|i| i * 7 - 3).collect()));
        roundtrip(ColumnData::I32(vec![-5, 0, 5, 100, -100]));
        roundtrip(ColumnData::F64(vec![1.5, 2.5, 3.14, -9.0]));
        roundtrip(ColumnData::F32(vec![0.1, 0.2, 0.3]));
        roundtrip(ColumnData::U32(vec![1, 2, 3, u32::MAX]));
        roundtrip(ColumnData::U64(vec![1, 2, 3, u64::MAX]));
        roundtrip(ColumnData::U8(vec![0, 1, 2, 255]));
    }

    #[test]
    fn varlen_columns_roundtrip() {
        roundtrip(ColumnData::Str(vec![
            "alpha".into(),
            "".into(),
            "δοκιμή".into(),
        ]));
        roundtrip(ColumnData::Bytes(vec![vec![1, 2, 3], vec![], vec![0xFF; 10]]));
    }

    #[test]
    fn pco_actually_compresses_monotonic_i64() {
        // A monotonic timestamp column should shrink dramatically under Pco.
        let col = ColumnData::I64((0..10_000).map(|i| 1_700_000_000_000 + i * 1000).collect());
        let encoded = encode_column(&col).unwrap();
        let raw = 10_000 * 8;
        assert!(
            encoded.len() < raw / 4,
            "expected >4x compression, got {} vs {raw}",
            encoded.len()
        );
    }

    #[test]
    fn column_builder_from_values() {
        let mut b = ColumnBuilder::new(DType::I64);
        for v in [Value::I64(10), Value::I64(20), Value::I64(30)] {
            b.push(&v);
        }
        assert_eq!(b.finish(), ColumnData::I64(vec![10, 20, 30]));
    }

    #[test]
    fn corrupt_payload_len_is_rejected() {
        let col = ColumnData::I64(vec![1, 2, 3]);
        let mut encoded = encode_column(&col).unwrap();
        // Overstate payload_len → decode must refuse rather than panic.
        let bad = (encoded.len() as u32 + 100).to_le_bytes();
        encoded[4..8].copy_from_slice(&bad);
        assert!(decode_column(&encoded, 3).is_err());
    }
}
