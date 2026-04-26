use std::fmt;

// ── Value ────────────────────────────────────────────────────────────

/// Typed row cell for batch writes.
pub enum Value<'a> {
    U8(u8),
    U32(u32),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    U64(u64),
    Str(&'a str),
    Bytes(&'a [u8]),
}

impl Value<'_> {
    /// Return the `DType` tag for this value.
    pub fn dtype(&self) -> DType {
        match self {
            Value::U8(_) => DType::U8,
            Value::U32(_) => DType::U32,
            Value::I32(_) => DType::I32,
            Value::I64(_) => DType::I64,
            Value::F32(_) => DType::F32,
            Value::F64(_) => DType::F64,
            Value::U64(_) => DType::U64,
            Value::Str(_) => DType::Str,
            Value::Bytes(_) => DType::Bytes,
        }
    }

    pub(crate) fn encoded_size(&self) -> usize {
        match self {
            Value::U8(_) => 1,
            Value::U32(_) | Value::I32(_) | Value::F32(_) => 4,
            Value::I64(_) | Value::F64(_) | Value::U64(_) => 8,
            Value::Str(s) => 4 + s.len(),
            Value::Bytes(b) => 4 + b.len(),
        }
    }

    pub(crate) fn encode(&self, out: &mut [u8]) -> usize {
        match self {
            Value::U8(v) => {
                out[0] = *v;
                1
            }
            Value::U32(v) => {
                out[..4].copy_from_slice(&v.to_le_bytes());
                4
            }
            Value::I32(v) => {
                out[..4].copy_from_slice(&v.to_le_bytes());
                4
            }
            Value::I64(v) => {
                out[..8].copy_from_slice(&v.to_le_bytes());
                8
            }
            Value::F32(v) => {
                out[..4].copy_from_slice(&v.to_le_bytes());
                4
            }
            Value::F64(v) => {
                out[..8].copy_from_slice(&v.to_le_bytes());
                8
            }
            Value::U64(v) => {
                out[..8].copy_from_slice(&v.to_le_bytes());
                8
            }
            Value::Str(s) => {
                let b = s.as_bytes();
                let len = 4 + b.len();
                out[..4].copy_from_slice(&(b.len() as u32).to_le_bytes());
                out[4..len].copy_from_slice(b);
                len
            }
            Value::Bytes(b) => {
                let len = 4 + b.len();
                out[..4].copy_from_slice(&(b.len() as u32).to_le_bytes());
                out[4..len].copy_from_slice(b);
                len
            }
        }
    }
}

// ── DType ────────────────────────────────────────────────────────────

/// Column data type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum DType {
    U8 = 1,
    I32 = 2,
    I64 = 3,
    F32 = 4,
    F64 = 5,
    U64 = 6,
    U32 = 7,
    /// Variable-length UTF-8 string. Row entry format: `[u32 len][bytes]`.
    Str = 8,
    /// Variable-length binary buffer. Row entry format: `[u32 len][bytes]`.
    Bytes = 9,
}

impl DType {
    pub fn fixed_size(self) -> Option<usize> {
        match self {
            Self::U8 => Some(1),
            Self::I32 | Self::F32 | Self::U32 => Some(4),
            Self::I64 | Self::F64 | Self::U64 => Some(8),
            Self::Str | Self::Bytes => None,
        }
    }

    pub fn is_fixed(self) -> bool {
        self.fixed_size().is_some()
    }

    pub(crate) fn from_u32(v: u32) -> Option<Self> {
        match v {
            1 => Some(Self::U8),
            2 => Some(Self::I32),
            3 => Some(Self::I64),
            4 => Some(Self::F32),
            5 => Some(Self::F64),
            6 => Some(Self::U64),
            7 => Some(Self::U32),
            8 => Some(Self::Str),
            9 => Some(Self::Bytes),
            _ => None,
        }
    }
}

impl fmt::Display for DType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Self::U8 => "u8",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::U64 => "u64",
            Self::U32 => "u32",
            Self::Str => "str",
            Self::Bytes => "bytes",
        })
    }
}

pub struct Col {
    pub name: String,
    pub dtype: DType,
    pub elem_size: usize,
}

pub struct Schema {
    pub cols: Vec<Col>,
}

impl Schema {
    pub fn new() -> Self {
        Self { cols: vec![] }
    }

    pub fn col(mut self, name: &str, dtype: DType) -> Self {
        let elem_size = dtype.fixed_size().unwrap_or(0);
        self.cols.push(Col {
            name: name.into(),
            dtype,
            elem_size,
        });
        self
    }
}

impl Default for Schema {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Schema {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Schema(")?;
        for (i, c) in self.cols.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}:{}", c.name, c.dtype)?;
        }
        write!(f, ")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_debug_format() {
        let schema = Schema::new().col("id", DType::I64).col("name", DType::Str);
        assert_eq!(format!("{schema:?}"), "Schema(id:i64, name:str)");
    }
}
