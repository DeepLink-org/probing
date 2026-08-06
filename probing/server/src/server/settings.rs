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

pub fn sync_env_settings() {
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

#[cfg(test)]
mod tests {
    use super::bind_address_from_port;

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
}
