use std::fmt;

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

    pub(crate) fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::U8,
            2 => Self::I32,
            3 => Self::I64,
            4 => Self::F32,
            5 => Self::F64,
            6 => Self::U64,
            7 => Self::U32,
            8 => Self::Str,
            9 => Self::Bytes,
            _ => panic!("invalid DType: {v}"),
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
