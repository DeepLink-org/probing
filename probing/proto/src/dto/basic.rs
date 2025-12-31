use std::fmt::{Display, Formatter};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Element type enumeration for DTO
#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub enum EleType {
    Nil,
    BOOL,
    I32,
    I64,
    F32,
    F64,
    Text,
    Url,
    DataTime,
}

/// Element value enumeration for DTO
#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub enum Ele {
    Nil,
    BOOL(bool),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    Text(String),
    Url(String),
    DataTime(u64),
}

impl Display for Ele {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ele::Nil => f.write_str("nil"),
            Ele::BOOL(x) => f.write_fmt(format_args!("{x}")),
            Ele::I32(x) => f.write_fmt(format_args!("{x}")),
            Ele::I64(x) => f.write_fmt(format_args!("{x}")),
            Ele::F32(x) => f.write_fmt(format_args!("{x}")),
            Ele::F64(x) => f.write_fmt(format_args!("{x}")),
            Ele::Text(x) => f.write_fmt(format_args!("{x}")),
            Ele::Url(x) => f.write_fmt(format_args!("{x}")),
            Ele::DataTime(x) => {
                let datetime: DateTime<Utc> =
                    (SystemTime::UNIX_EPOCH + Duration::from_micros(*x)).into();
                f.write_fmt(format_args!("{}", datetime.to_rfc3339()))
            }
        }
    }
}

/// Sequence of elements for DTO
#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
#[serde(tag = "type", content = "value")]
pub enum Seq {
    Nil,
    SeqBOOL(Vec<bool>),
    SeqI32(Vec<i32>),
    SeqI64(Vec<i64>),
    SeqF32(Vec<f32>),
    SeqF64(Vec<f64>),
    SeqText(Vec<String>),
    SeqDateTime(Vec<u64>),
}

impl Seq {
    pub fn len(&self) -> usize {
        match self {
            Seq::SeqBOOL(vec) => vec.len(),
            Seq::SeqI32(vec) => vec.len(),
            Seq::SeqI64(vec) => vec.len(),
            Seq::SeqF32(vec) => vec.len(),
            Seq::SeqF64(vec) => vec.len(),
            Seq::SeqText(vec) => vec.len(),
            Seq::SeqDateTime(vec) => vec.len(),
            Seq::Nil => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Seq::Nil => true,
            other => other.len() == 0,
        }
    }

    pub fn get(&self, idx: usize) -> Ele {
        match self {
            Seq::SeqBOOL(vec) => vec.get(idx).map(|x| Ele::BOOL(*x)),
            Seq::SeqI32(vec) => vec.get(idx).map(|x| Ele::I32(*x)),
            Seq::SeqI64(vec) => vec.get(idx).map(|x| Ele::I64(*x)),
            Seq::SeqF32(vec) => vec.get(idx).map(|x| Ele::F32(*x)),
            Seq::SeqF64(vec) => vec.get(idx).map(|x| Ele::F64(*x)),
            Seq::SeqText(vec) => vec.get(idx).map(|x| Ele::Text(x.clone())),
            Seq::SeqDateTime(vec) => vec.get(idx).map(|x| Ele::DataTime(*x)),
            Seq::Nil => None,
        }
        .unwrap_or(Ele::Nil)
    }
}

/// Value representation for DTO
#[derive(Debug, Default, Deserialize, Serialize, PartialEq, Eq, Clone)]
pub struct Value {
    pub id: u64,
    pub class: String,
    pub shape: Option<String>,
    pub dtype: Option<String>,
    pub device: Option<String>,
    pub value: Option<String>,
}

impl Display for Value {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "value: {:?}", self.value)
    }
}
