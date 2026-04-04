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
    pub(crate) fn encoded_size(&self) -> usize {
        match self {
            Value::U8(_) => 1,
            Value::U32(_) | Value::I32(_) | Value::F32(_) => 4,
            Value::I64(_) | Value::F64(_) | Value::U64(_) => 8,
            Value::Str(s) => 4 + s.len(),
            Value::Bytes(b) => 4 + b.len(),
        }
    }

    pub(crate) fn encode(&self, out: &mut [u8]) {
        match self {
            Value::U8(v) => out[0] = *v,
            Value::U32(v) => out[..4].copy_from_slice(&v.to_le_bytes()),
            Value::I32(v) => out[..4].copy_from_slice(&v.to_le_bytes()),
            Value::I64(v) => out[..8].copy_from_slice(&v.to_le_bytes()),
            Value::F32(v) => out[..4].copy_from_slice(&v.to_le_bytes()),
            Value::F64(v) => out[..8].copy_from_slice(&v.to_le_bytes()),
            Value::U64(v) => out[..8].copy_from_slice(&v.to_le_bytes()),
            Value::Str(s) => {
                let b = s.as_bytes();
                out[..4].copy_from_slice(&(b.len() as u32).to_le_bytes());
                out[4..4 + b.len()].copy_from_slice(b);
            }
            Value::Bytes(b) => {
                out[..4].copy_from_slice(&(b.len() as u32).to_le_bytes());
                out[4..4 + b.len()].copy_from_slice(b);
            }
        }
    }
}
