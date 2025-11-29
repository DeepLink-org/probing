#[macro_use]
extern crate ctor;

use anyhow::Result;
use pyo3::prelude::*;

use probing_python::features::python_api::{cli_main, query_json};
use probing_python::features::vm_tracer::{
    enable_tracer, disable_tracer, _get_python_stacks, _get_python_frames, initialize_globals,
};
use probing_python::features::config;
use probing_python::features::tracing;
use probing_python::extensions::python::ExternalTable;
use probing_server::sync_env_settings;

use probing_python::pkg::TCPStore;

const ENV_PROBING_LOGLEVEL: &str = "PROBING_LOGLEVEL";
const ENV_PROBING_PORT: &str = "PROBING_PORT";

#[cfg(feature = "use-mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub fn get_hostname() -> Result<String> {
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

    // Check for address pattern match from environment variable
    if let Ok(pattern) = std::env::var("PROBING_SERVER_ADDRPATTERN") {
        for ip in ips.iter() {
            if ip.starts_with(pattern.as_str()) {
                log::debug!("Select IP address {ip} with pattern {pattern}");
                return Ok(ip.clone());
            }
            log::debug!("Skip IP address {ip} with pattern {pattern}");
        }
    }

    // Return first IP if no pattern match found
    ips.first()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("No suitable IP address found"))
}

/// Setup environment variables for server configuration
fn setup_env_settings() {
    let mut report_port_basis: Option<u16> = None;

    match std::env::var(ENV_PROBING_PORT) {
        Ok(port_env_val) => {
            if port_env_val.eq_ignore_ascii_case("RANDOM") {
                log::debug!(
                    "ENV_PROBING_PORT is RANDOM. PROBING_SERVER_ADDR set to 0.0.0.0:0 for random port binding."
                );
                std::env::set_var("PROBING_SERVER_ADDR", "'0.0.0.0:0'");
                // report_port_basis remains None for RANDOM
            } else {
                // Not "RANDOM", try to parse as a specific port number
                match port_env_val.parse::<u16>() {
                    Ok(port_number) => {
                        log::debug!(
                            "ENV_PROBING_PORT specifies port: {port_number}. PROBING_SERVER_ADDR will be set."
                        );
                        report_port_basis = Some(port_number);

                        let local_rank: u16 = std::env::var("LOCAL_RANK")
                            .unwrap_or_else(|_| "0".to_string())
                            .parse()
                            .unwrap_or(0);
                        let serving_port = port_number.saturating_add(local_rank);

                        let hostname =
                            if std::env::var("RANK").unwrap_or_else(|_| "0".to_string()) == "0" {
                                "0.0.0.0".to_string()
                            } else {
                                get_hostname().unwrap_or_else(|err| {
                                    log::warn!(
                                        "Failed to get hostname: {err}, defaulting to localhost"
                                    );
                                    "localhost".to_string()
                                })
                            };
                        std::env::set_var(
                            "PROBING_SERVER_ADDR",
                            format!("'{hostname}:{serving_port}'"),
                        );
                        log::debug!(
                            "PROBING_SERVER_ADDR set to {hostname}:{serving_port} (base: {port_number}, local_rank: {local_rank})."
                        );
                    }
                    Err(_) => {
                        log::warn!(
                            "ENV_PROBING_PORT value '{port_env_val}' is not 'RANDOM' and not a valid port number. PROBING_SERVER_ADDR will not be set. Remote server not started by sync_env_settings."
                        );
                        // PROBING_SERVER_ADDR is not set
                    }
                }
            }
        }
        Err(_) => {
            log::debug!("ENV_PROBING_PORT not set. PROBING_SERVER_ADDR will not be set. Remote server not started by sync_env_settings.");
            // PROBING_SERVER_ADDR is not set
        }
    }

    // Setup reporting address only if a base port was determined (specific port, not RANDOM)
    if let Some(base_port_for_reporting) = report_port_basis {
        if let Ok(master_addr) = std::env::var("MASTER_ADDR") {
            if !master_addr.is_empty() {
                // Ensure MASTER_ADDR is not empty
                log::debug!("Configuring PROBING_SERVER_REPORT_ADDR to {master_addr}:{base_port_for_reporting} based on MASTER_ADDR and base port");
                std::env::set_var(
                    "PROBING_SERVER_REPORT_ADDR",
                    format!("'{master_addr}:{base_port_for_reporting}'"),
                );
            }
        }
    }
}

#[ctor]
fn setup() {
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
    if let Err(e) = probing_server::cleanup() {
        log::error!("Failed to cleanup unix socket: {e}");
    }
}

/// Python module entry point - exported as probing._core
#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
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
