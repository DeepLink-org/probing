#[macro_use]
extern crate ctor;

use anyhow::Result;
use pyo3::prelude::*;

use probing_cli::pyo3::cli_main;
use probing_core::{install_panic_hook, register_python_main_thread};
use probing_python::extensions::python::{register_table_docs, ExternalTable};
use probing_python::features::python::bindings;
use probing_python::features::python::bindings::query_json;
use probing_python::features::python::tracing;
use probing_python::features::stacktrace::vm::{
    _get_python_frames, _get_python_stacks, disable_tracer, enable_tracer, initialize_globals,
};
use probing_server::sync_env_settings;
use probing_skills::pyo3::register_skills_bindings;

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
    #[cfg(unix)]
    {
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
    #[cfg(windows)]
    {
        use std::net::UdpSocket;
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        let _ = socket.connect("8.8.8.8:80");
        let ip = socket.local_addr()?.ip().to_string();
        log::debug!("Using local IP via UDP probe: {ip}");
        Ok(vec![ip])
    }
    #[cfg(not(any(unix, windows)))]
    {
        Ok(Vec::new())
    }
}

/// True for the torchrun/elastic supervisor process (not a training worker).
fn is_elastic_supervisor() -> bool {
    if std::env::var("LOCAL_RANK").is_ok() || std::env::var("RANK").is_ok() {
        return false;
    }
    if std::env::var("TORCHELASTIC_RUN_ID").is_ok() {
        return true;
    }
    std::env::args().any(|arg| {
        let a = arg.as_str();
        a.ends_with("torchrun") || a.contains("torch/distributed/run")
    })
}

/// Multi-process torchrun jobs: bind HTTP and start Rust-side cluster heartbeat.
fn setup_torchrun_cluster_env() {
    if is_elastic_supervisor() {
        log::debug!("torchrun/elastic supervisor: skip cluster setup");
        return;
    }
    probing_server::maybe_start_torchrun_cluster();
}

/// Setup environment variables for server configuration (single-process only).
///
/// Multi-process torchrun jobs bind HTTP and run hierarchical cluster report in Rust.
fn setup_env_settings() {
    let world_size: i32 = std::env::var("WORLD_SIZE")
        .unwrap_or_else(|_| "1".to_string())
        .parse()
        .unwrap_or(1);
    if world_size > 1 {
        setup_torchrun_cluster_env();
        return;
    }

    if is_elastic_supervisor() {
        log::debug!("torchrun/elastic supervisor: defer probing HTTP bind to worker ranks");
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

    // Initialize logging (try_init to avoid conflicts)
    let _ = env_logger::try_init_from_env(env_logger::Env::new().filter(ENV_PROBING_LOGLEVEL));
    log::info!("Initializing probing module for process {pid} ...");

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

/// Start the in-process engine and local query server (same as normal `PROBING=1` startup).
///
/// Used when `PROBING_CLI_MODE=1` skipped the `#[ctor]` hook so docs can be registered first.
#[pyfunction]
fn start_local() {
    probing_server::start_local();
}

/// Python module entry point - exported as probing._core
#[pymodule(gil_used = true)]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_python_main_thread();
    probing_python::features::stacktrace::capture::register_main_os_tid();

    // Initialize logging (try_init to avoid conflicts if already initialized via #[ctor])
    let _ = env_logger::try_init_from_env(env_logger::Env::new().filter(ENV_PROBING_LOGLEVEL));

    // Initialize globals and tracer if needed
    if initialize_globals() {
        let disable = std::env::var("PROBING_VM_TRACER").as_deref() == Ok("0");
        if !disable {
            // Enable tracer if tracing feature is enabled
            // Note: This is handled by the probing-python crate's tracing feature
            let _ = enable_tracer();
        }
    }

    // Register all classes
    m.add_class::<ExternalTable>()?;
    m.add_function(wrap_pyfunction!(register_table_docs, m)?)?;
    m.add_function(wrap_pyfunction!(start_local, m)?)?;
    m.add_class::<TCPStore>()?;

    // Register all functions
    m.add_function(wrap_pyfunction!(query_json, m)?)?;
    m.add_function(wrap_pyfunction!(enable_tracer, m)?)?;
    m.add_function(wrap_pyfunction!(disable_tracer, m)?)?;
    m.add_function(wrap_pyfunction!(_get_python_stacks, m)?)?;
    m.add_function(wrap_pyfunction!(_get_python_frames, m)?)?;
    m.add_function(wrap_pyfunction!(cli_main, m)?)?;
    m.add_function(wrap_pyfunction!(
        probing_python::features::python::bindings::api_callstack,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        probing_python::features::python::bindings::api_eval,
        m
    )?)?;
    register_skills_bindings(m)?;

    // Add is_enabled function to help tests check state
    use probing_python::features::python::bindings::{is_enabled, should_enable_probing};
    m.add_function(wrap_pyfunction!(is_enabled, m)?)?;
    m.add_function(wrap_pyfunction!(should_enable_probing, m)?)?;
    use probing_python::features::crash::{
        crash_enabled, note_last_comm, record_crash, request_crash_hold, request_crash_release,
    };
    m.add_function(wrap_pyfunction!(record_crash, m)?)?;
    m.add_function(wrap_pyfunction!(crash_enabled, m)?)?;
    m.add_function(wrap_pyfunction!(note_last_comm, m)?)?;
    m.add_function(wrap_pyfunction!(request_crash_hold, m)?)?;
    m.add_function(wrap_pyfunction!(request_crash_release, m)?)?;

    #[pyfunction]
    fn start_torchrun_cluster() -> PyResult<Option<String>> {
        probing_server::maybe_start_torchrun_cluster();
        Ok(probing_server::master_http_base())
    }

    #[pyfunction]
    fn refresh_torchrun_cluster_role() -> PyResult<bool> {
        Ok(probing_server::refresh_torchrun_role())
    }

    m.add_function(wrap_pyfunction!(start_torchrun_cluster, m)?)?;
    m.add_function(wrap_pyfunction!(refresh_torchrun_cluster_role, m)?)?;

    // Register config functions directly to the module (flattened)
    bindings::register_config_functions(m)?;

    // Register tracing classes and functions directly to the module (flattened)
    tracing::register_tracing_functions(m)?;

    Ok(())
}
