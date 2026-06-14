//! Synthetic schemas and deterministic row generation.
//!
//! Generators are driven by a seedable xorshift PRNG so a run is fully
//! reproducible given `--seed`. The timestamp column is named `timestamp`
//! (recognised by the memtable as the designated time column) and is kept
//! monotonically increasing, which is both realistic for observability data
//! and the case Pco compresses best.

use std::str::FromStr;

use probing_memtable::{DType, RowWriter, Schema, Value};

/// Built-in column layouts covering the main compression / width regimes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum SchemaKind {
    /// `timestamp:i64, value:f64, tag:u32` — narrow numeric, the common
    /// metrics shape; compresses very well in the cold tier.
    Metrics,
    /// `timestamp:i64` + N `f64` columns — wide numeric rows.
    Wide,
    /// `timestamp:i64, level:u32, msg:str` — variable-length string payload
    /// (no Pco, exercises the raw var-len path).
    Logs,
}

impl FromStr for SchemaKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "metrics" => Ok(Self::Metrics),
            "wide" => Ok(Self::Wide),
            "logs" => Ok(Self::Logs),
            other => Err(format!(
                "unknown schema '{other}' (expected metrics|wide|logs)"
            )),
        }
    }
}

/// Parameters that shape a generated workload.
#[derive(Debug, Clone)]
pub struct WorkloadSpec {
    pub kind: SchemaKind,
    /// Number of `f64` columns for [`SchemaKind::Wide`].
    pub wide_cols: usize,
    /// Length in bytes of the `msg` payload for [`SchemaKind::Logs`].
    pub str_len: usize,
}

impl WorkloadSpec {
    pub fn schema(&self) -> Schema {
        match self.kind {
            SchemaKind::Metrics => Schema::new()
                .col("timestamp", DType::I64)
                .col("value", DType::F64)
                .col("tag", DType::U32),
            SchemaKind::Wide => {
                let mut s = Schema::new().col("timestamp", DType::I64);
                for i in 0..self.wide_cols {
                    s = s.col(&format!("f{i}"), DType::F64);
                }
                s
            }
            SchemaKind::Logs => Schema::new()
                .col("timestamp", DType::I64)
                .col("level", DType::U32)
                .col("msg", DType::Str),
        }
    }

    /// Approximate encoded bytes of one row (excludes the 4-byte row-length
    /// prefix); used to translate row counts into a logical byte rate.
    pub fn approx_row_bytes(&self) -> usize {
        match self.kind {
            SchemaKind::Metrics => 8 + 8 + 4,
            SchemaKind::Wide => 8 + self.wide_cols * 8,
            SchemaKind::Logs => 8 + 4 + 4 + self.str_len,
        }
    }
}

/// Deterministic per-thread row generator.
pub struct RowGen {
    spec: WorkloadSpec,
    rng: u64,
    ts: i64,
    msg: String,
}

impl RowGen {
    /// `seed` should differ per thread for independent streams; `start_ts`
    /// offsets the monotonic timestamp so concurrent streams don't fully
    /// overlap in time.
    pub fn new(spec: WorkloadSpec, seed: u64, start_ts: i64) -> Self {
        let str_len = spec.str_len;
        Self {
            spec,
            rng: seed | 1,
            ts: start_ts,
            msg: String::with_capacity(str_len),
        }
    }

    #[inline]
    fn next(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Write one row through the streaming [`RowWriter`] fast path.
    ///
    /// Returns the value of [`RowWriter::finish`] — `false` means the row
    /// did not fit the current chunk (the caller should advance and retry).
    #[inline]
    pub fn write_into(&mut self, w: &mut RowWriter) -> bool {
        // Timestamp advances by a small positive jitter (1..=4): monotone,
        // realistic, Pco-friendly.
        self.ts += 1 + (self.next() & 0x3) as i64;
        let ts = self.ts;
        match self.spec.kind {
            SchemaKind::Metrics => {
                let v = (self.next() % 1_000_000) as f64 * 0.001;
                w.put_i64(ts)
                    .put_f64(v)
                    .put_u32((self.next() % 1024) as u32)
                    .finish()
            }
            SchemaKind::Wide => {
                let mut wr = w.put_i64(ts);
                for _ in 0..self.spec.wide_cols {
                    let v = (self.next() % 1_000_000) as f64 * 0.001;
                    wr = wr.put_f64(v);
                }
                wr.finish()
            }
            SchemaKind::Logs => {
                self.fill_msg();
                w.put_i64(ts)
                    .put_u32((self.next() % 5) as u32)
                    .put_str(&self.msg)
                    .finish()
            }
        }
    }

    fn fill_msg(&mut self) {
        self.msg.clear();
        const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789 ";
        for _ in 0..self.spec.str_len {
            let c = ALPHABET[(self.next() as usize) % ALPHABET.len()];
            self.msg.push(c as char);
        }
    }

    /// Build a borrowed [`Value`] row for the `push_row` path. The returned
    /// vector borrows `self.msg` for the logs schema, so it must be consumed
    /// before the next call.
    pub fn values<'a>(&'a mut self, scratch: &'a mut Vec<f64>) -> Vec<Value<'a>> {
        self.ts += 1 + (self.next() & 0x3) as i64;
        let ts = self.ts;
        match self.spec.kind {
            SchemaKind::Metrics => {
                let v = (self.next() % 1_000_000) as f64 * 0.001;
                vec![
                    Value::I64(ts),
                    Value::F64(v),
                    Value::U32((self.next() % 1024) as u32),
                ]
            }
            SchemaKind::Wide => {
                scratch.clear();
                for _ in 0..self.spec.wide_cols {
                    scratch.push((self.next() % 1_000_000) as f64 * 0.001);
                }
                let mut row = Vec::with_capacity(1 + scratch.len());
                row.push(Value::I64(ts));
                for v in scratch.iter() {
                    row.push(Value::F64(*v));
                }
                row
            }
            SchemaKind::Logs => {
                let level = (self.next() % 5) as u32;
                self.fill_msg();
                vec![Value::I64(ts), Value::U32(level), Value::Str(&self.msg)]
            }
        }
    }
}
