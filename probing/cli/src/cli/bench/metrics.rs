//! Measurement primitives: a bounded latency reservoir and a small
//! report builder that renders either as an aligned table or as JSON.

use std::time::Duration;

/// Reservoir-sampled latency recorder (nanoseconds).
///
/// Per-operation timing on a hot write path is itself measurable overhead,
/// so latency capture is opt-in. When enabled we keep an unbiased uniform
/// sample of at most `cap` observations (reservoir sampling) plus exact
/// `min`/`max`/`sum`/`count`, which is enough for stable tail-quantile
/// estimates without unbounded memory.
pub struct Latency {
    samples: Vec<u64>,
    cap: usize,
    seen: u64,
    min: u64,
    max: u64,
    sum: u128,
    rng: u64,
}

impl Latency {
    pub fn new(cap: usize) -> Self {
        Self {
            samples: Vec::with_capacity(cap.min(1 << 16)),
            cap: cap.max(1),
            seen: 0,
            min: u64::MAX,
            max: 0,
            sum: 0,
            rng: 0x9E37_79B9_7F4A_7C15,
        }
    }

    #[inline]
    fn next_rng(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    #[inline]
    pub fn record(&mut self, ns: u64) {
        self.seen += 1;
        self.min = self.min.min(ns);
        self.max = self.max.max(ns);
        self.sum += ns as u128;
        if self.samples.len() < self.cap {
            self.samples.push(ns);
        } else {
            let j = (self.next_rng() % self.seen) as usize;
            if j < self.cap {
                self.samples[j] = ns;
            }
        }
    }

    pub fn merge(&mut self, other: &Latency) {
        for &s in &other.samples {
            self.record(s);
        }
        // record() above already folded the sampled values; fix the exact
        // aggregates from the source's exact tallies instead of the sample.
        if other.seen > 0 {
            self.min = self.min.min(other.min);
            self.max = self.max.max(other.max);
        }
    }

    pub fn count(&self) -> u64 {
        self.seen
    }

    pub fn mean_ns(&self) -> f64 {
        if self.seen == 0 {
            0.0
        } else {
            self.sum as f64 / self.seen as f64
        }
    }

    pub fn min_ns(&self) -> u64 {
        if self.seen == 0 {
            0
        } else {
            self.min
        }
    }

    pub fn max_ns(&self) -> u64 {
        self.max
    }

    /// Estimated quantile (`q` in `[0,1]`) from the reservoir sample.
    pub fn quantile_ns(&self, q: f64) -> u64 {
        if self.samples.is_empty() {
            return 0;
        }
        let mut s = self.samples.clone();
        s.sort_unstable();
        let q = q.clamp(0.0, 1.0);
        let idx = ((s.len() as f64 - 1.0) * q).round() as usize;
        s[idx]
    }
}

// ── Report ───────────────────────────────────────────────────────────

/// One labelled measurement: a human string plus a machine-readable value.
struct Entry {
    key: String,
    display: String,
    json: serde_json::Value,
}

/// Accumulates labelled results and renders them as an aligned table or
/// a JSON object. Construction order is preserved.
pub struct Report {
    title: String,
    entries: Vec<Entry>,
}

impl Report {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            entries: Vec::new(),
        }
    }

    fn push(&mut self, key: &str, display: String, json: serde_json::Value) -> &mut Self {
        self.entries.push(Entry {
            key: key.to_string(),
            display,
            json,
        });
        self
    }

    pub fn text(&mut self, key: &str, value: impl Into<String>) -> &mut Self {
        let v = value.into();
        let json = serde_json::Value::String(v.clone());
        self.push(key, v, json)
    }

    pub fn count(&mut self, key: &str, n: u64) -> &mut Self {
        self.push(key, group_thousands(n), serde_json::Value::from(n))
    }

    pub fn float(&mut self, key: &str, v: f64, suffix: &str) -> &mut Self {
        let disp = if suffix.is_empty() {
            format!("{v:.3}")
        } else {
            format!("{v:.3} {suffix}")
        };
        self.push(key, disp, json_f64(v))
    }

    pub fn ratio(&mut self, key: &str, v: f64) -> &mut Self {
        self.push(key, format!("{v:.2}x"), json_f64(v))
    }

    pub fn bytes(&mut self, key: &str, n: u64) -> &mut Self {
        self.push(key, human_bytes(n), serde_json::Value::from(n))
    }

    pub fn duration(&mut self, key: &str, d: Duration) -> &mut Self {
        self.push(
            key,
            format!("{:.3} s", d.as_secs_f64()),
            json_f64(d.as_secs_f64()),
        )
    }

    /// Throughput in ops/second, displayed with an SI suffix.
    pub fn rate(&mut self, key: &str, ops: u64, elapsed: Duration, unit: &str) -> &mut Self {
        let per_sec = rate_per_sec(ops, elapsed);
        self.push(key, format!("{} {unit}/s", si(per_sec)), json_f64(per_sec))
    }

    /// Throughput in bytes/second, displayed as MiB/s.
    pub fn byte_rate(&mut self, key: &str, bytes: u64, elapsed: Duration) -> &mut Self {
        let per_sec = rate_per_sec(bytes, elapsed);
        self.push(
            key,
            format!("{:.2} MiB/s", per_sec / (1024.0 * 1024.0)),
            json_f64(per_sec),
        )
    }

    /// Append the standard quantile rows for a latency recorder.
    pub fn latency(&mut self, prefix: &str, lat: &Latency) -> &mut Self {
        if lat.count() == 0 {
            return self;
        }
        self.float(&format!("{prefix} min"), lat.min_ns() as f64, "ns");
        self.float(&format!("{prefix} mean"), lat.mean_ns(), "ns");
        self.float(&format!("{prefix} p50"), lat.quantile_ns(0.50) as f64, "ns");
        self.float(&format!("{prefix} p99"), lat.quantile_ns(0.99) as f64, "ns");
        self.float(
            &format!("{prefix} p999"),
            lat.quantile_ns(0.999) as f64,
            "ns",
        );
        self.float(&format!("{prefix} max"), lat.max_ns() as f64, "ns");
        self
    }

    pub fn print_table(&self) {
        let width = self.entries.iter().map(|e| e.key.len()).max().unwrap_or(0);
        println!("\n  {}", self.title);
        println!("  {}", "─".repeat(self.title.len().max(20)));
        for e in &self.entries {
            println!("  {:<width$}  {}", e.key, e.display, width = width);
        }
        println!();
    }

    pub fn to_json(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for e in &self.entries {
            map.insert(e.key.clone(), e.json.clone());
        }
        serde_json::json!({ "benchmark": self.title, "metrics": map })
    }

    pub fn emit(&self, json: bool) {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&self.to_json()).unwrap_or_else(|_| "{}".to_string())
            );
        } else {
            self.print_table();
        }
    }
}

// ── formatting helpers ─────────────────────────────────────────────────

fn json_f64(v: f64) -> serde_json::Value {
    serde_json::Number::from_f64(v)
        .map(serde_json::Value::Number)
        .unwrap_or(serde_json::Value::Null)
}

pub fn rate_per_sec(n: u64, elapsed: Duration) -> f64 {
    let s = elapsed.as_secs_f64();
    if s <= 0.0 {
        0.0
    } else {
        n as f64 / s
    }
}

/// SI-suffixed magnitude (K/M/G) for human-readable rates.
pub fn si(v: f64) -> String {
    let a = v.abs();
    if a >= 1e9 {
        format!("{:.2}G", v / 1e9)
    } else if a >= 1e6 {
        format!("{:.2}M", v / 1e6)
    } else if a >= 1e3 {
        format!("{:.2}K", v / 1e3)
    } else {
        format!("{v:.0}")
    }
}

pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.2} {}", UNITS[i])
    }
}

fn group_thousands(n: u64) -> String {
    let s = n.to_string();
    let len = s.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
