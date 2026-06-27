//! Crash capture — stderr + spill + grace.
//!
//! Python ``excepthook`` only. Post-mortem: logs and ``default_dir()/crash/<pid>/latest.json``.

mod api;
mod grace;
mod handler;
mod report;
mod signal;

pub use api::{
    crash_enabled, note_last_comm, record_crash, request_crash_hold, request_crash_release,
};
pub use signal::install_crash_handler;

pub(crate) mod config {
    pub fn enabled() -> bool {
        match std::env::var("PROBING_CRASH") {
            Ok(val) => {
                let lower = val.trim().to_ascii_lowercase();
                !matches!(lower.as_str(), "0" | "false" | "no" | "off")
            }
            Err(_) => true,
        }
    }

    pub fn grace_sec() -> u64 {
        if env_truthy("PROBING_CRASH_NO_GRACE", false) {
            return 0;
        }
        std::env::var("PROBING_CRASH_GRACE_SEC")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(20)
    }

    pub fn force_hold() -> bool {
        env_truthy("PROBING_CRASH_HOLD", false)
    }

    pub fn grace_all_ranks() -> bool {
        env_truthy("PROBING_CRASH_GRACE_ALL_RANKS", false)
    }

    pub fn spill_enabled() -> bool {
        match std::env::var("PROBING_CRASH_SPILL") {
            Ok(val) => {
                let lower = val.trim().to_ascii_lowercase();
                !matches!(lower.as_str(), "0" | "false" | "no" | "off")
            }
            Err(_) => true,
        }
    }

    fn env_truthy(name: &str, default: bool) -> bool {
        match std::env::var(name) {
            Ok(val) => matches!(
                val.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            ),
            Err(_) => default,
        }
    }
}

pub(crate) mod context {
    pub struct Snapshot {
        pub rank: i32,
        pub local_rank: i32,
        pub world_size: i32,
        pub host: String,
        pub pid: i32,
    }

    pub fn snapshot() -> Snapshot {
        Snapshot {
            rank: env_i32("RANK"),
            local_rank: env_i32("LOCAL_RANK"),
            world_size: env_i32("WORLD_SIZE"),
            host: std::env::var("POD_IP")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    std::env::var("HOSTNAME").unwrap_or_else(|_| "localhost".into())
                }),
            pid: std::process::id() as i32,
        }
    }

    pub fn hold_file_path(pid: i32) -> String {
        probing_memtable::discover::default_dir()
            .join("crash")
            .join(format!("hold-{pid}"))
            .to_string_lossy()
            .into()
    }

    fn env_i32(key: &str) -> i32 {
        std::env::var(key)
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(-1)
    }
}

pub fn handle_http(
    path: &str,
    _params: &std::collections::HashMap<String, String>,
    _body: &[u8],
) -> Result<Vec<u8>, String> {
    match path {
        "crash/hold" => {
            grace::request_hold();
            Ok(r#"{"ok":true,"held":true}"#.into())
        }
        "crash/release" => {
            grace::request_release(true);
            Ok(r#"{"ok":true,"released":true}"#.into())
        }
        _ => Err(format!("unknown crash path: {path}")),
    }
}
