//! Environment-backed server setting synchronization.

use log::error;
use probing_core::config;

/// Convert `PROBING_PORT` into the numeric wildcard address used for binding.
pub fn bind_address_from_port(value: &str) -> Result<String, std::num::ParseIntError> {
    if value.eq_ignore_ascii_case("RANDOM") {
        Ok("0.0.0.0:0".to_string())
    } else {
        value.parse::<u16>().map(|port| format!("0.0.0.0:{port}"))
    }
}

/// Port env is only meaningful for the single-process HTTP server.
fn should_apply_port_env() -> bool {
    let world_size = std::env::var("WORLD_SIZE")
        .ok()
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(1);
    if world_size > 1 {
        return false;
    }
    let elastic_supervisor = std::env::var("TORCHELASTIC_RUN_ID").is_ok()
        && std::env::var("LOCAL_RANK").is_err()
        && std::env::var("RANK").is_err();
    let torchrun_args = std::env::args()
        .any(|arg| arg.ends_with("torchrun") || arg.contains("torch/distributed/run"));
    !(elastic_supervisor || torchrun_args)
}

fn apply_port_env_if_single_process() {
    if should_apply_port_env() {
        apply_probing_port_env();
    }
}

pub fn sync_env_settings() {
    apply_port_env_if_single_process();

    if let Some(message) = crate::engine_lifecycle::engine_not_ready_message() {
        error!("Cannot sync env settings: {message}");
        return;
    }

    let env_vars: Vec<(String, String)> = std::env::vars()
        .filter(|(key, _)| {
            key.starts_with("PROBING_")
                && ![
                    "PROBING_PORT",
                    "PROBING_LOGLEVEL",
                    "PROBING_ASSETS_ROOT",
                    "PROBING_SERVER_ADDRPATTERN",
                    "PROBING_AUTH_TOKEN",
                    "PROBING_BASE_PATH",
                    "PROBING_ORIGINAL",
                ]
                .contains(&key.as_str())
        })
        .collect();

    if let Err(error) = super::SERVER_RUNTIME.spawn(async move {
        for (key, value) in env_vars {
            let key = key.replace('_', ".").to_lowercase();
            match config::write(&key, &value).await {
                Ok(_) => log::debug!("Synced env setting: {key}"),
                Err(error) => error!("Failed to sync env setting '{key}': {error}"),
            };
        }
    }) {
        log::error!("failed to schedule server settings update: {error}");
    }
}

/// Best-effort check that the requested HTTP address is free.
///
/// Ray propagates `PROBING_PORT` to every raylet/worker, so only one process
/// can own the HTTP port; the rest fall back to their Unix socket.
pub fn tcp_port_available(address: &str) -> bool {
    match std::net::TcpListener::bind(address) {
        Ok(listener) => {
            drop(listener);
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => false,
        // Let the real server surface other errors (permissions, bad address).
        Err(_) => true,
    }
}

/// Resolve `PROBING_PORT` to a usable HTTP bind address.
///
/// Returns `None` when the port is busy or malformed; in both cases the local
/// Unix socket remains available.
pub fn resolve_http_bind(port_env_val: &str) -> Option<String> {
    match bind_address_from_port(port_env_val) {
        Ok(address) => {
            if tcp_port_available(&address) {
                Some(address)
            } else {
                log::warn!("PROBING_PORT {port_env_val} busy; HTTP disabled");
                None
            }
        }
        Err(_) => {
            log::warn!("invalid PROBING_PORT '{port_env_val}'; HTTP disabled");
            None
        }
    }
}

/// Apply `PROBING_PORT` to `PROBING_SERVER_ADDR` unless the port is unusable.
pub fn apply_probing_port_env() {
    match std::env::var("PROBING_PORT") {
        Ok(port_env_val) => {
            if let Some(address) = resolve_http_bind(&port_env_val) {
                log::debug!("PROBING_SERVER_ADDR set to {address}");
                std::env::set_var("PROBING_SERVER_ADDR", address);
            }
        }
        Err(_) => {
            log::debug!("PROBING_PORT not set. PROBING_SERVER_ADDR will not be set.");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::bind_address_from_port;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn fixed_port_produces_an_unquoted_wildcard_bind_address() {
        let address = bind_address_from_port("9922").unwrap();
        assert_eq!(address, "0.0.0.0:9922");
        assert!(address.parse::<std::net::SocketAddr>().is_ok());
    }

    #[test]
    fn random_port_uses_port_zero_and_invalid_values_fail() {
        assert_eq!(bind_address_from_port("RANDOM").unwrap(), "0.0.0.0:0");
        assert!(bind_address_from_port("not-a-port").is_err());
    }

    #[test]
    fn port_env_guard_skips_multi_process_and_supervisors() {
        let _env_guard = ENV_LOCK.lock().unwrap();
        use super::should_apply_port_env;

        let old_world_size = std::env::var("WORLD_SIZE").ok();
        let old_torchrun = std::env::var("TORCHELASTIC_RUN_ID").ok();
        let old_rank = std::env::var("RANK").ok();
        let old_local_rank = std::env::var("LOCAL_RANK").ok();

        std::env::remove_var("WORLD_SIZE");
        std::env::remove_var("TORCHELASTIC_RUN_ID");
        std::env::remove_var("RANK");
        std::env::remove_var("LOCAL_RANK");
        assert!(should_apply_port_env());

        std::env::set_var("WORLD_SIZE", "2");
        assert!(!should_apply_port_env());

        std::env::set_var("WORLD_SIZE", "1");
        std::env::set_var("TORCHELASTIC_RUN_ID", "test");
        assert!(!should_apply_port_env());

        std::env::set_var("LOCAL_RANK", "0");
        assert!(should_apply_port_env());

        match old_world_size {
            Some(value) => std::env::set_var("WORLD_SIZE", value),
            None => std::env::remove_var("WORLD_SIZE"),
        }
        match old_torchrun {
            Some(value) => std::env::set_var("TORCHELASTIC_RUN_ID", value),
            None => std::env::remove_var("TORCHELASTIC_RUN_ID"),
        }
        match old_rank {
            Some(value) => std::env::set_var("RANK", value),
            None => std::env::remove_var("RANK"),
        }
        match old_local_rank {
            Some(value) => std::env::set_var("LOCAL_RANK", value),
            None => std::env::remove_var("LOCAL_RANK"),
        }
    }

    #[test]
    fn single_process_applies_port_env() {
        let _env_guard = ENV_LOCK.lock().unwrap();
        use super::apply_port_env_if_single_process;
        use std::net::TcpListener;

        let old_world_size = std::env::var("WORLD_SIZE").ok();
        let old_port = std::env::var("PROBING_PORT").ok();
        let old_address = std::env::var("PROBING_SERVER_ADDR").ok();
        std::env::remove_var("WORLD_SIZE");
        std::env::remove_var("TORCHELASTIC_RUN_ID");
        std::env::remove_var("RANK");
        std::env::remove_var("LOCAL_RANK");

        let listener = TcpListener::bind("0.0.0.0:0").expect("bind ephemeral port");
        let port = listener.local_addr().expect("local addr").port();
        drop(listener);
        std::env::set_var("PROBING_PORT", port.to_string());
        std::env::remove_var("PROBING_SERVER_ADDR");
        apply_port_env_if_single_process();
        assert_eq!(
            std::env::var("PROBING_SERVER_ADDR").unwrap(),
            format!("0.0.0.0:{port}")
        );
        super::sync_env_settings();

        match old_world_size {
            Some(value) => std::env::set_var("WORLD_SIZE", value),
            None => std::env::remove_var("WORLD_SIZE"),
        }
        match old_port {
            Some(value) => std::env::set_var("PROBING_PORT", value),
            None => std::env::remove_var("PROBING_PORT"),
        }
        match old_address {
            Some(value) => std::env::set_var("PROBING_SERVER_ADDR", value),
            None => std::env::remove_var("PROBING_SERVER_ADDR"),
        }
    }

    #[test]
    fn port_conflict_and_bad_port_do_not_set_http_address() {
        let _env_guard = ENV_LOCK.lock().unwrap();
        use super::{apply_probing_port_env, resolve_http_bind, tcp_port_available};
        use std::net::TcpListener;

        let old_port = std::env::var("PROBING_PORT").ok();
        let old_address = std::env::var("PROBING_SERVER_ADDR").ok();
        let listener = TcpListener::bind("0.0.0.0:0").expect("bind ephemeral port");
        let address = listener.local_addr().expect("local addr").to_string();
        let port = listener.local_addr().expect("local addr").port();

        assert!(!tcp_port_available(&address));
        assert!(tcp_port_available("not a valid address"));
        assert!(resolve_http_bind(&port.to_string()).is_none());
        assert!(resolve_http_bind("not-a-port").is_none());

        std::env::remove_var("PROBING_PORT");
        std::env::remove_var("PROBING_SERVER_ADDR");
        apply_probing_port_env();
        assert!(std::env::var("PROBING_SERVER_ADDR").is_err());

        std::env::set_var("PROBING_PORT", port.to_string());
        std::env::remove_var("PROBING_SERVER_ADDR");
        apply_probing_port_env();
        assert!(std::env::var("PROBING_SERVER_ADDR").is_err());

        std::env::set_var("PROBING_PORT", "not-a-port");
        apply_probing_port_env();
        assert!(std::env::var("PROBING_SERVER_ADDR").is_err());

        drop(listener);
        assert!(tcp_port_available(&address));
        std::env::set_var("PROBING_PORT", port.to_string());
        apply_probing_port_env();
        assert_eq!(
            std::env::var("PROBING_SERVER_ADDR").unwrap(),
            format!("0.0.0.0:{port}")
        );

        match old_port {
            Some(value) => std::env::set_var("PROBING_PORT", value),
            None => std::env::remove_var("PROBING_PORT"),
        }
        match old_address {
            Some(value) => std::env::set_var("PROBING_SERVER_ADDR", value),
            None => std::env::remove_var("PROBING_SERVER_ADDR"),
        }
    }
}
