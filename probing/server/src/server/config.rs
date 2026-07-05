/// Maximum request body size allowed (5MB)
pub const MAX_REQUEST_BODY_SIZE: usize = 5 * 1024 * 1024;

/// Maximum file size allowed for file API reading (10MB)
pub const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Static allowed base directories (relative paths resolved at runtime).
pub const ALLOWED_FILE_DIRS: &[&str] = &["./logs", "./data", "./config", "/tmp"];

/// Get maximum request body size from environment or use default
pub fn get_max_request_body_size() -> usize {
    std::env::var("PROBING_MAX_REQUEST_SIZE")
        .unwrap_or(MAX_REQUEST_BODY_SIZE.to_string())
        .parse::<usize>()
        .unwrap_or(MAX_REQUEST_BODY_SIZE)
}

/// HTTP connection limit — defaults to max(fan-out concurrency, 128).
pub fn effective_max_connections() -> usize {
    if let Ok(raw) = std::env::var("PROBING_MAX_CONNECTIONS") {
        if let Ok(n) = raw.trim().parse::<usize>() {
            if n > 0 {
                return n;
            }
        }
    }
    probing_core::core::federation::remote_fanout_concurrency().max(128)
}

/// Get maximum file size from environment or use default
pub fn get_max_file_size() -> u64 {
    std::env::var("PROBING_MAX_FILE_SIZE")
        .unwrap_or(MAX_FILE_SIZE.to_string())
        .parse::<u64>()
        .unwrap_or(MAX_FILE_SIZE)
}

/// Runtime base directories for the file read API (stack traces, workspace sources).
pub fn allowed_file_base_dirs() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;

    let mut bases: Vec<PathBuf> = ALLOWED_FILE_DIRS.iter().map(PathBuf::from).collect();

    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            bases.push(PathBuf::from(home));
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        bases.push(cwd);
    }

    if let Ok(extra) = std::env::var("PROBING_ALLOWED_FILE_DIRS") {
        for part in extra.split(':') {
            let part = part.trim();
            if !part.is_empty() {
                bases.push(PathBuf::from(part));
            }
        }
    }

    bases
}
