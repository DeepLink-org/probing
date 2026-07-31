use memmap2::MmapMut;
use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

/// Independent MAP_SHARED views of one temporary file for concurrency tests.
pub(crate) struct TestSharedFile {
    file: File,
    path: PathBuf,
}

impl TestSharedFile {
    pub(crate) fn new(size: usize) -> Self {
        let id = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "probing-memtable-test-{}-{id}.mmap",
            std::process::id()
        ));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.set_len(size as u64).unwrap();
        Self { file, path }
    }

    pub(crate) fn map_mut(&self) -> MmapMut {
        unsafe { MmapMut::map_mut(&self.file).unwrap() }
    }

    pub(crate) fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TestSharedFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
