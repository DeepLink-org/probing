#[macro_use]
extern crate ctor;

use anyhow::Result;
use pyo3::prelude::*;

use probing_core::{install_panic_hook, register_python_main_thread};
use probing_python::extensions::python::ExternalTable;
use probing_python::features::config;
use probing_python::features::python_api::{cli_main, query_json};
use probing_python::features::tracing;
use probing_python::features::vm_tracer::{
    _get_python_frames, _get_python_stacks, disable_tracer, enable_tracer, initialize_globals,
};
use probing_server::sync_env_settings;

use probing_python::pkg::TCPStore;

const ENV_PROBING_LOGLEVEL: &str = "PROBING_LOGLEVEL";
const ENV_PROBING_PORT: &str = "PROBING_PORT";

#[cfg(feature = "use-mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub fn get_hostname() -> Result<String> {
    // Pod environment - prioritize IP environment variables
    let ip_env_vars = ["POD_IP"];
    for env_var in &ip_env_vars {
        if let Ok(ip) = std::env::var(env_var) {
            if !ip.is_empty() && ip != "None" {
                log::debug!("Using IP from environment variable {env_var}: {ip}");
                return Ok(ip);
            }
        }
    }

    let ips = get_network_interfaces()?;

    if let Ok(pattern) = std::env::var("PROBING_SERVER_ADDRPATTERN") {
        for ip in ips.iter() {
            if ip.starts_with(pattern.as_str()) {
                log::debug!("Select IP address {ip} with pattern {pattern}");
                return Ok(ip.clone());
            }
            log::debug!("Skip IP address {ip} with pattern {pattern}");
        }
    }

    ips.first()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("No suitable IP address found"))
}

fn get_network_interfaces() -> Result<Vec<String>> {
    let ips = nix::ifaddrs::getifaddrs()?
        .filter_map(|addr| addr.address)
        .filter_map(|addr| addr.as_sockaddr_in().cloned())
        .filter_map(|addr| {
            let ip_addr = addr.ip();
            match ip_addr.is_unspecified() {
                true => None,
                false => Some(ip_addr.to_string()),
            }
        })
        .collect::<Vec<_>>();

    log::debug!("Found network interface IPs: {:?}", ips);
    Ok(ips)
}

/// Setup environment variables for server configuration (single-process only).
///
/// Multi-process torchrun jobs defer HTTP bind and master discovery to
/// ``probing.torchrun_cluster`` after ``init_process_group``.
fn setup_env_settings() {
    let world_size: i32 = std::env::var("WORLD_SIZE")
        .unwrap_or_else(|_| "1".to_string())
        .parse()
        .unwrap_or(1);
    if world_size > 1 {
        log::debug!(
            "WORLD_SIZE={world_size}: defer probing HTTP bind to torchrun_cluster (Python)"
        );
        return;
    }

    match std::env::var(ENV_PROBING_PORT) {
        Ok(port_env_val) => {
            if port_env_val.eq_ignore_ascii_case("RANDOM") {
                log::debug!(
                    "ENV_PROBING_PORT is RANDOM. PROBING_SERVER_ADDR set to 0.0.0.0:0 for random port binding."
                );
                std::env::set_var("PROBING_SERVER_ADDR", "'0.0.0.0:0'");
            } else if let Ok(port_number) = port_env_val.parse::<u16>() {
                log::debug!(
                    "ENV_PROBING_PORT specifies port: {port_number}. PROBING_SERVER_ADDR will be set."
                );
                std::env::set_var("PROBING_SERVER_ADDR", format!("'0.0.0.0:{port_number}'"));
            } else {
                log::warn!(
                    "ENV_PROBING_PORT value '{port_env_val}' is not 'RANDOM' and not a valid port number."
                );
            }
        }
        Err(_) => {
            log::debug!("ENV_PROBING_PORT not set. PROBING_SERVER_ADDR will not be set.");
        }
    }
}

const ENV_PROBING_CLI_MODE: &str = "PROBING_CLI_MODE";

#[ctor]
fn setup() {
    install_panic_hook();

    // Skip initialization if running in CLI mode (e.g., probing ls)
    // CLI commands should not inject probes into themselves
    if std::env::var(ENV_PROBING_CLI_MODE).is_ok() {
        return;
    }

    let pid = std::process::id();
    eprintln!("Initializing probing module for process {pid} ...",);

    // Initialize logging (try_init to avoid conflicts)
    let _ = env_logger::try_init_from_env(env_logger::Env::new().filter(ENV_PROBING_LOGLEVEL));

    // Initialize probing server (local Unix domain socket)
    // This needs to happen early, even if Python module is not imported
    probing_server::start_local();

    // Setup environment variables
    setup_env_settings();
    sync_env_settings();
}

#[dtor]
fn cleanup() {
    // Skip cleanup if running in CLI mode (no probes were initialized)
    if std::env::var(ENV_PROBING_CLI_MODE).is_ok() {
        return;
    }

    if let Err(e) = probing_server::cleanup() {
        log::error!("Failed to cleanup unix socket: {e}");
    }
}

/// Python module entry point - exported as probing._core
#[pymodule(gil_used = true)]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_python_main_thread();

    // Initialize logging (try_init to avoid conflicts if already initialized via #[ctor])
    let _ = env_logger::try_init_from_env(env_logger::Env::new().filter(ENV_PROBING_LOGLEVEL));

    // Initialize globals and tracer if needed
    if initialize_globals() {
        // Enable tracer if tracing feature is enabled
        // Note: This is handled by the probing-python crate's tracing feature
        let _ = enable_tracer();
    }

    // Register all classes
    m.add_class::<ExternalTable>()?;
    m.add_class::<TCPStore>()?;

    // Register all functions
    m.add_function(wrap_pyfunction!(query_json, m)?)?;
    m.add_function(wrap_pyfunction!(enable_tracer, m)?)?;
    m.add_function(wrap_pyfunction!(disable_tracer, m)?)?;
    m.add_function(wrap_pyfunction!(_get_python_stacks, m)?)?;
    m.add_function(wrap_pyfunction!(_get_python_frames, m)?)?;
    m.add_function(wrap_pyfunction!(cli_main, m)?)?;
    use probing_python::features::python_api::{api_callstack, api_eval};
    m.add_function(wrap_pyfunction!(api_callstack, m)?)?;
    m.add_function(wrap_pyfunction!(api_eval, m)?)?;

    // Add is_enabled function to help tests check state
    use probing_python::features::python_api::{is_enabled, should_enable_probing};
    m.add_function(wrap_pyfunction!(is_enabled, m)?)?;
    m.add_function(wrap_pyfunction!(should_enable_probing, m)?)?;

    // Register config functions directly to the module (flattened)
    config::register_config_functions(m)?;

    // Register tracing classes and functions directly to the module (flattened)
    tracing::register_tracing_functions(m)?;

    Ok(())
}
