use std::collections::HashMap;

use anyhow::Result;
use probing_proto::prelude::*;

use super::error::ApiResult;

const SENSITIVE_ENV_KEYS: &[&str] = &[
    "PROBING_AUTH_TOKEN",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
];

fn is_sensitive_env(key: &str) -> bool {
    if SENSITIVE_ENV_KEYS.contains(&key) {
        return true;
    }
    key.starts_with("PROBING_AUTH_") || key.ends_with("_TOKEN") || key.ends_with("_SECRET")
}

fn public_env_vars() -> HashMap<String, String> {
    std::env::vars()
        .filter(|(k, _)| !is_sensitive_env(k))
        .collect()
}

/// Get system overview information about the current process
pub fn get_overview() -> Result<Process> {
    let myself = std::process::id() as i32;

    #[cfg(target_os = "linux")]
    let threads = {
        let current = procfs::process::Process::new(myself)?;
        current
            .tasks()
            .map(|iter| iter.map(|r| r.map(|p| p.tid as u64).unwrap_or(0)).collect())
            .unwrap_or_default()
    };

    #[cfg(target_os = "macos")]
    let threads = vec![];

    let info = Process {
        pid: myself,
        exe: std::env::current_exe()
            .unwrap_or_default()
            .to_str()
            .unwrap_or_default()
            .to_string(),
        env: public_env_vars(),
        cmd: std::env::args().collect::<Vec<String>>().join(" "),
        cwd: std::env::current_dir()
            .unwrap_or_default()
            .to_str()
            .unwrap_or_default()
            .to_string(),
        main_thread: myself as u64,
        threads: threads,
    };
    Ok(info)
}

/// Get system overview information as JSON for API
pub async fn get_overview_json() -> ApiResult<axum::Json<Process>> {
    let overview = get_overview()?;
    Ok(axum::Json(overview))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_auth_token_from_overview_env() {
        std::env::set_var("PROBING_AUTH_TOKEN", "secret");
        std::env::set_var("PROBING_SAFE_DEMO", "visible");
        let env = public_env_vars();
        assert!(!env.contains_key("PROBING_AUTH_TOKEN"));
        assert_eq!(env.get("PROBING_SAFE_DEMO"), Some(&"visible".to_string()));
        std::env::remove_var("PROBING_AUTH_TOKEN");
        std::env::remove_var("PROBING_SAFE_DEMO");
    }
}
