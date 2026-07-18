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
        threads,
    };
    Ok(info)
}

/// Get system overview information as JSON for API
pub async fn get_overview_json() -> ApiResult<axum::Json<Process>> {
    let overview = get_overview()?;
    Ok(axum::Json(overview))
}


/// Get local processes that currently expose probing memtables.
pub fn get_local_processes() -> Result<Vec<Process>> {
    use std::collections::HashSet;
    use probing_memtable::discover;

    let mut pids = HashSet::<i32>::new();
    for table in discover::discover()? {
        if table.is_alive() {
            pids.insert(table.pid() as i32);
        }
    }

    let mut processes = pids
        .into_iter()
        .map(process_from_pid)
        .collect::<Vec<_>>();
    processes.sort_by_key(|process| process.pid);
    Ok(processes)
}

/// Get local probing processes as JSON for API.
pub async fn get_local_processes_json() -> ApiResult<axum::Json<Vec<Process>>> {
    Ok(axum::Json(get_local_processes()?))
}

fn process_from_pid(pid: i32) -> Process {
    use std::collections::HashMap;
    Process {
        pid,
        exe: read_proc_link(pid, "exe"),
        env: HashMap::new(),
        cmd: read_proc_cmdline(pid),
        cwd: read_proc_link(pid, "cwd"),
        main_thread: pid as u64,
        threads: read_proc_threads(pid),
    }
}

#[cfg(target_os = "linux")]
fn read_proc_cmdline(pid: i32) -> String {
    let path = format!("/proc/{pid}/cmdline");
    let cmd = std::fs::read_to_string(path)
        .unwrap_or_default()
        .replace('\0', " ")
        .trim()
        .to_string();
    if !cmd.is_empty() {
        return cmd;
    }
    std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .unwrap_or_default()
        .trim()
        .to_string()
}

#[cfg(not(target_os = "linux"))]
fn read_proc_cmdline(_pid: i32) -> String {
    String::new()
}

#[cfg(target_os = "linux")]
fn read_proc_link(pid: i32, name: &str) -> String {
    std::fs::read_link(format!("/proc/{pid}/{name}"))
        .ok()
        .and_then(|path| path.to_str().map(|value| value.to_string()))
        .unwrap_or_default()
}

#[cfg(not(target_os = "linux"))]
fn read_proc_link(_pid: i32, _name: &str) -> String {
    String::new()
}

#[cfg(target_os = "linux")]
fn read_proc_threads(pid: i32) -> Vec<u64> {
    std::fs::read_dir(format!("/proc/{pid}/task"))
        .ok()
        .into_iter()
        .flat_map(|entries| entries.flatten())
        .filter_map(|entry| entry.file_name().to_string_lossy().parse::<u64>().ok())
        .collect()
}

#[cfg(not(target_os = "linux"))]
fn read_proc_threads(_pid: i32) -> Vec<u64> {
    Vec::new()
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
