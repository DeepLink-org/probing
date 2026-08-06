use std::fs;
use std::io;
use std::path::{Path, PathBuf};

fn copy_dir(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&source_path, &destination_path)?;
        } else {
            fs::copy(source_path, destination_path)?;
        }
    }
    Ok(())
}

fn main() -> io::Result<()> {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "CARGO_MANIFEST_DIR is unavailable")
    })?);
    let generated = manifest_dir.join("web-assets/public");
    let fallback = manifest_dir.join("web-fallback");
    let source = if generated.join("embedded.manifest").is_file() {
        generated.as_path()
    } else {
        fallback.as_path()
    };

    println!("cargo:rerun-if-changed={}", generated.display());
    println!("cargo:rerun-if-changed={}", fallback.display());

    let out_dir = PathBuf::from(
        std::env::var_os("OUT_DIR")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "OUT_DIR is unavailable"))?,
    );
    let destination = out_dir.join("probing-web-assets");
    if destination.exists() {
        fs::remove_dir_all(&destination)?;
    }
    copy_dir(source, &destination)
}
