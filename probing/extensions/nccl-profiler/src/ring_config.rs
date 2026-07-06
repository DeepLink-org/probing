//! Mmap ring sizing for NCCL memtables (`PROBING_NCCL_CHUNK_BYTES`, `PROBING_NCCL_NUM_CHUNKS`).

const DEFAULT_CHUNK_BYTES: u32 = 64 * 1024;
const DEFAULT_NUM_CHUNKS: u32 = 64;
const MIN_CHUNK_BYTES: u32 = 4 * 1024;
const MAX_CHUNK_BYTES: u32 = 16 * 1024 * 1024;
const MIN_NUM_CHUNKS: u32 = 4;
const MAX_NUM_CHUNKS: u32 = 4096;

/// `(chunk_size_bytes, num_chunks)` for NCCL mmap tables.
pub fn nccl_mmap_ring_config() -> (u32, u32) {
    let chunk = std::env::var("PROBING_NCCL_CHUNK_BYTES")
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .map(|v| v.clamp(MIN_CHUNK_BYTES, MAX_CHUNK_BYTES))
        .unwrap_or(DEFAULT_CHUNK_BYTES);
    let num = std::env::var("PROBING_NCCL_NUM_CHUNKS")
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .map(|v| v.clamp(MIN_NUM_CHUNKS, MAX_NUM_CHUNKS))
        .unwrap_or(DEFAULT_NUM_CHUNKS);
    (chunk, num)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_production_sized() {
        let (chunk, num) = nccl_mmap_ring_config();
        assert_eq!(chunk, DEFAULT_CHUNK_BYTES);
        assert_eq!(num, DEFAULT_NUM_CHUNKS);
    }
}
