mod asset;
// Make auth module public for integration tests
pub mod auth;
pub mod cluster_http;
mod cluster_report_backoff;
mod engine;
mod extensions;
pub mod memtable_ext;
mod report;
// Make server module public for integration tests in tests/ directory
pub mod server;
mod torchrun_cluster;
mod vars;

pub use self::report::start_report_worker;
pub use self::server::start_local;
pub use self::server::start_remote;
pub use self::server::sync_env_settings;
pub use self::torchrun_cluster::{
    is_torchrun_cluster_active, master_http_base, maybe_start_torchrun_cluster,
    refresh_torchrun_role,
};

pub fn cleanup() -> anyhow::Result<()> {
    let prefix = std::env::var("PROBING_CTRL_ROOT").unwrap_or("/tmp/probing/".to_string());

    let pid = std::process::id();
    let path = format!("{prefix}/{pid}");
    let path = std::path::Path::new(&path);
    if path.exists() {
        std::fs::remove_file(path)?;
    }

    Ok(())
}
