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

use std::collections::HashSet;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::layout::{
    get_u16, get_u32, xxh32, FLAG_SEALED, MAGIC_MEMC, SEGMENT_HEADER_SIZE, VERSION_MEMC,
};
use super::reader::SegmentReader;
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
        let next_seq = Self::max_seq_for(&dir, &wid)?.saturating_add(1);
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
    fn max_seq_for(dir: &Path, wid: &str) -> io::Result<u32> {
        let mut max = 0u32;
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some((w, seq)) = parse_segment_name(&name) {
                if w == wid {
                    max = max.max(seq);
                }
            }
        }
        Ok(max)
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
        loop {
            let path = self.next_segment_path();
            match SegmentWriter::create(&path) {
                Ok(writer) => return Ok(writer),
                // Multiple ColdStore handles in one process share writer_id
                // and may initially choose the same sequence. Atomic
                // create_new plus retry prevents either handle from
                // truncating the other's open segment.
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists && path.exists() => {
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// All segment files in the directory (any writer), sorted oldest →
    /// newest by modification time.
    pub fn segment_paths(&self) -> Vec<PathBuf> {
        match self.segment_paths_checked() {
            Ok(paths) => paths,
            Err(error) => {
                log::warn!(
                    "MEMC segment enumeration failed for {}: {error}",
                    self.dir.display()
                );
                Vec::new()
            }
        }
    }

    /// Fallible segment enumeration for correctness-sensitive workers.
    pub fn segment_paths_checked(&self) -> io::Result<Vec<PathBuf>> {
        let mut segs: Vec<(SystemTime, PathBuf)> = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some(SEGMENT_EXT) {
                continue;
            }
            let mtime = entry.metadata()?.modified()?;
            segs.push((mtime, path));
        }
        segs.sort_by_key(|a| a.0);
        Ok(segs.into_iter().map(|(_, p)| p).collect())
    }

    pub fn stats(&self) -> ColdStats {
        match self.stats_checked() {
            Ok(stats) => stats,
            Err(error) => {
                log::warn!(
                    "MEMC stats collection failed for {}: {error}",
                    self.dir.display()
                );
                ColdStats::default()
            }
        }
    }

    /// Fallible capacity snapshot used by retention accounting.
    pub fn stats_checked(&self) -> io::Result<ColdStats> {
        let paths = self.segment_paths_checked()?;
        let mut total = 0u64;
        let mut oldest = u64::MAX;
        for p in &paths {
            let meta = std::fs::metadata(p)?;
            total = total.saturating_add(meta.len());
            let ms = meta
                .modified()?
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            oldest = oldest.min(ms);
        }
        Ok(ColdStats {
            segment_count: paths.len(),
            total_bytes: total,
            oldest_unix_ms: if paths.is_empty() { 0 } else { oldest },
        })
    }

    /// Evict oldest segments until under `max_bytes` and within `ttl`.
    ///
    /// Either limit may be `None` to disable it. Unsealed or unreadable
    /// segments are never evicted because they may be open in any writer
    /// sharing this directory. The newest segment is also retained. Returns
    /// the paths removed.
    pub fn enforce_limits(&self, max_bytes: Option<u64>, ttl: Option<Duration>) -> Vec<PathBuf> {
        match self.enforce_limits_checked(max_bytes, ttl) {
            Ok(removed) => removed,
            Err(error) => {
                log::warn!("MEMC retention enforcement failed: {error}");
                Vec::new()
            }
        }
    }

    /// Fallible retention enforcement used by the background compactor so
    /// filesystem failures are observable instead of silently discarded.
    pub(crate) fn enforce_limits_checked(
        &self,
        max_bytes: Option<u64>,
        ttl: Option<Duration>,
    ) -> io::Result<Vec<PathBuf>> {
        let paths = self.segment_paths_checked()?;
        if paths.len() <= 1 {
            return Ok(Vec::new());
        }
        let mut protected = HashSet::new();
        for path in &paths {
            match SegmentReader::open(path) {
                Ok(reader) if reader.is_sealed() => {}
                // A concurrent writer may be between create/header write or
                // actively appending. Conservatively retain both cases.
                Err(_) if legacy_segment_is_sealed(path) => {}
                Ok(_) => {
                    protected.insert(path.clone());
                }
                Err(error) => {
                    return Err(io::Error::new(
                        error.kind(),
                        format!(
                            "cannot validate MEMC segment {} for retention: {error}",
                            path.display()
                        ),
                    ));
                }
            }
        }
        // Preserve the existing newest-segment retention guarantee even
        // when every segment happens to be sealed.
        if let Some(newest) = paths.last() {
            protected.insert(newest.clone());
        }

        let now = SystemTime::now();
        let mut total = 0u64;
        for path in &paths {
            total = total.saturating_add(std::fs::metadata(path)?.len());
        }

        let mut removed = Vec::new();
        for path in paths {
            if protected.contains(&path) {
                continue;
            }
            let metadata = match std::fs::metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            let too_old = match ttl {
                Some(ttl) => now
                    .duration_since(metadata.modified()?)
                    .is_ok_and(|age| age > ttl),
                None => false,
            };
            let over_budget = max_bytes.is_some_and(|max| total > max);
            if !(too_old || over_budget) {
                break; // sorted oldest-first: nothing newer qualifies either
            }
            let sz = metadata.len();
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    total = total.saturating_sub(sz);
                    removed.push(path);
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(removed)
    }
}

/// Recognise a sealed v1 header for retention cleanup while keeping v1
/// unavailable to readers. Unsealed, partial, foreign, and corrupt files are
/// conservatively treated as potentially open.
fn legacy_segment_is_sealed(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut header = [0u8; SEGMENT_HEADER_SIZE];
    if file.read_exact(&mut header).is_err()
        || get_u32(&header, 0) != MAGIC_MEMC
        || get_u16(&header, 4) != VERSION_MEMC - 1
        || get_u32(&header, 60) != xxh32(&header[..60])
    {
        return false;
    }
    get_u16(&header, 10) & FLAG_SEALED != 0
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

    #[test]
    fn concurrent_store_handles_never_reuse_or_truncate_a_segment() {
        let tmp = std::env::temp_dir().join(format!("memc-store-race-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let mut first = ColdStore::open(&tmp).unwrap();
        let mut second = ColdStore::open(&tmp).unwrap();

        let first_writer = first.create_segment().unwrap();
        let first_path = first_writer.path().to_path_buf();
        let first_len = std::fs::metadata(&first_path).unwrap().len();
        let second_writer = second.create_segment().unwrap();

        assert_ne!(first_writer.path(), second_writer.path());
        assert_eq!(
            std::fs::metadata(&first_path).unwrap().len(),
            first_len,
            "a racing store must not truncate the first open segment"
        );

        drop((first_writer, second_writer));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
