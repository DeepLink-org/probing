//! [`ColdStore`]: directory of MEMC segment files with capacity management.
//!
//! Layout (one directory per host, segments shared across all of a
//! writer's tables):
//!
//! ```text
//! <base>/
//!     a3f2c1-000001.memc   ← writer "a3f2c1", sequence 1 (sealed)
//!     a3f2c1-000002.memc   ← sequence 2 (current, may be unsealed)
//!     9c81b0-000001.memc   ← another writer/process on the same host
//! ```
//!
//! The store is a **second-level ring**: the hot MEMT buffer wraps by
//! bytes, the cold store wraps by whole segment files. Eviction deletes
//! the oldest segments once a byte budget or TTL is exceeded; because
//! segments are immutable whole files, eviction is atomic and O(1) per
//! file, and `unlink`ing a segment that a query still has mmap'd is safe
//! under POSIX (the inode survives until the last mapping drops).

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::layout::xxh32;
use super::writer::SegmentWriter;
use crate::raw::process_start_time;

const SEGMENT_EXT: &str = "memc";

/// Stable per-writer id: hash of (pid, process start time). Restarting the
/// process yields a fresh id, so sequence numbers never collide across the
/// lifetime of a host directory.
pub fn writer_id(pid: u32, start_time: u64) -> String {
    let mut buf = [0u8; 12];
    buf[0..4].copy_from_slice(&pid.to_le_bytes());
    buf[4..12].copy_from_slice(&start_time.to_le_bytes());
    format!("{:06x}", xxh32(&buf) & 0x00FF_FFFF)
}

/// Capacity snapshot of a cold store.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ColdStats {
    pub segment_count: usize,
    pub total_bytes: u64,
    /// Modification time of the oldest segment, ms since epoch (0 if none).
    pub oldest_unix_ms: u64,
}

/// A directory of MEMC segments owned by one writer process.
pub struct ColdStore {
    dir: PathBuf,
    writer_id: String,
    next_seq: u32,
}

/// Default cold-store base directory: `$PROBING_COLD_DIR`, else
/// `<temp>/probing-cold`.
pub fn default_cold_dir() -> PathBuf {
    std::env::var_os("PROBING_COLD_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("probing-cold"))
}

impl ColdStore {
    /// Open (creating if needed) a cold store rooted at `dir`.
    pub fn open(dir: impl AsRef<Path>) -> io::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        let pid = std::process::id();
        let wid = writer_id(pid, process_start_time(pid));
        let next_seq = Self::max_seq_for(&dir, &wid) + 1;
        Ok(Self {
            dir,
            writer_id: wid,
            next_seq,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn writer_id(&self) -> &str {
        &self.writer_id
    }

    /// Highest existing sequence number for `wid` in `dir` (0 if none).
    fn max_seq_for(dir: &Path, wid: &str) -> u32 {
        let mut max = 0u32;
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if let Some((w, seq)) = parse_segment_name(&name) {
                    if w == wid {
                        max = max.max(seq);
                    }
                }
            }
        }
        max
    }

    /// Path for the next segment (does not create the file).
    pub fn next_segment_path(&mut self) -> PathBuf {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.dir
            .join(format!("{}-{:06}.{}", self.writer_id, seq, SEGMENT_EXT))
    }

    /// Create a new [`SegmentWriter`] for the next sequence number.
    pub fn create_segment(&mut self) -> io::Result<SegmentWriter> {
        let path = self.next_segment_path();
        SegmentWriter::create(path)
    }

    /// All segment files in the directory (any writer), sorted oldest →
    /// newest by modification time.
    pub fn segment_paths(&self) -> Vec<PathBuf> {
        let mut segs: Vec<(SystemTime, PathBuf)> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            for e in entries.flatten() {
                let path = e.path();
                if path.extension().and_then(|s| s.to_str()) != Some(SEGMENT_EXT) {
                    continue;
                }
                let mtime = e
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                segs.push((mtime, path));
            }
        }
        segs.sort_by_key(|a| a.0);
        segs.into_iter().map(|(_, p)| p).collect()
    }

    pub fn stats(&self) -> ColdStats {
        let paths = self.segment_paths();
        let mut total = 0u64;
        let mut oldest = u64::MAX;
        for p in &paths {
            if let Ok(meta) = std::fs::metadata(p) {
                total += meta.len();
                if let Ok(mtime) = meta.modified() {
                    let ms = mtime
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);
                    oldest = oldest.min(ms);
                }
            }
        }
        ColdStats {
            segment_count: paths.len(),
            total_bytes: total,
            oldest_unix_ms: if paths.is_empty() { 0 } else { oldest },
        }
    }

    /// Evict oldest segments until under `max_bytes` and within `ttl`.
    ///
    /// Either limit may be `None` to disable it. The newest segment is
    /// never evicted (it may be the one currently being appended). Returns
    /// the paths removed.
    pub fn enforce_limits(&self, max_bytes: Option<u64>, ttl: Option<Duration>) -> Vec<PathBuf> {
        let mut paths = self.segment_paths();
        if paths.len() <= 1 {
            return Vec::new();
        }
        // Protect the newest segment (oldest-first order ⇒ it is last);
        // it may be the one currently being appended.
        paths.pop();

        let file_len = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        let now = SystemTime::now();
        let mut total: u64 = self.stats().total_bytes;

        let mut removed = Vec::new();
        for path in paths {
            let too_old = ttl
                .and_then(|ttl| {
                    let mtime = std::fs::metadata(&path).ok()?.modified().ok()?;
                    now.duration_since(mtime).ok().map(|age| age > ttl)
                })
                .unwrap_or(false);
            let over_budget = max_bytes.is_some_and(|max| total > max);
            if !(too_old || over_budget) {
                break; // sorted oldest-first: nothing newer qualifies either
            }
            let sz = file_len(&path);
            if std::fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(sz);
                removed.push(path);
            }
        }
        removed
    }
}

/// Parse `"<writer_id>-<seq>.memc"` → `(writer_id, seq)`.
fn parse_segment_name(name: &str) -> Option<(String, u32)> {
    let stem = name.strip_suffix(".memc")?;
    let (wid, seq) = stem.rsplit_once('-')?;
    let seq: u32 = seq.parse().ok()?;
    Some((wid.to_string(), seq))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_id_is_stable_and_pid_sensitive() {
        assert_eq!(writer_id(100, 5), writer_id(100, 5));
        assert_ne!(writer_id(100, 5), writer_id(101, 5));
        assert_ne!(writer_id(100, 5), writer_id(100, 6));
        assert_eq!(writer_id(100, 5).len(), 6);
    }

    #[test]
    fn parse_segment_name_roundtrip() {
        assert_eq!(
            parse_segment_name("a3f2c1-000007.memc"),
            Some(("a3f2c1".to_string(), 7))
        );
        assert_eq!(parse_segment_name("notasegment.txt"), None);
        assert_eq!(parse_segment_name("missingseq.memc"), None);
    }

    #[test]
    fn sequence_numbers_increment_and_persist() {
        let tmp = std::env::temp_dir().join(format!("memc-store-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let mut store = ColdStore::open(&tmp).unwrap();
        let p1 = store.next_segment_path();
        let p2 = store.next_segment_path();
        assert_ne!(p1, p2);
        assert!(p1.to_string_lossy().contains("-000001."));
        assert!(p2.to_string_lossy().contains("-000002."));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
