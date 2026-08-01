//! Environment-backed server setting synchronization.

use log::error;
use probing_core::config;

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

    super::SERVER_RUNTIME.spawn(async move {
        for (key, value) in env_vars {
            let key = key.replace('_', ".").to_lowercase();
            match config::write(&key, &value).await {
                Ok(_) => log::debug!("Synced env setting: {key}"),
                Err(error) => error!("Failed to sync env setting '{key}': {error}"),
            };
        }
    });
}
