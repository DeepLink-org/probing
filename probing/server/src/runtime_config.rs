//! Mutable server runtime configuration initialized from process environment.

use std::fmt;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 30;
const MIN_DEFAULT_MAX_CONNECTIONS: usize = 128;

pub(crate) struct ServerRuntimeConfig {
    max_connections: AtomicUsize,
    request_timeout_secs: AtomicU64,
}

pub(crate) struct RuntimeConfigSnapshot {
    max_connections: usize,
    request_timeout_secs: u64,
}

impl fmt::Display for RuntimeConfigSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "max_connections={}, request_timeout_secs={}",
            self.max_connections, self.request_timeout_secs
        )
    }
}

impl ServerRuntimeConfig {
    pub(crate) fn from_env() -> Self {
        let default_connections = probing_core::core::federation::remote_fanout_concurrency()
            .max(MIN_DEFAULT_MAX_CONNECTIONS);
        Self {
            max_connections: AtomicUsize::new(
                parse_env("PROBING_MAX_CONNECTIONS")
                    .filter(|&value| value > 0)
                    .unwrap_or(default_connections),
            ),
            request_timeout_secs: AtomicU64::new(
                parse_env("PROBING_SERVER_TIMEOUT_SECS")
                    .filter(|&value| value > 0)
                    .unwrap_or(DEFAULT_REQUEST_TIMEOUT_SECS),
            ),
        }
    }

    pub(crate) fn max_connections(&self) -> usize {
        self.max_connections.load(Ordering::Acquire)
    }

    pub(crate) fn set_max_connections(&self, value: usize) {
        self.max_connections.store(value, Ordering::Release);
    }

    pub(crate) fn request_timeout_secs(&self) -> u64 {
        self.request_timeout_secs.load(Ordering::Acquire)
    }

    pub(crate) fn set_request_timeout_secs(&self, value: u64) {
        self.request_timeout_secs.store(value, Ordering::Release);
    }

    pub(crate) fn snapshot(&self) -> RuntimeConfigSnapshot {
        RuntimeConfigSnapshot {
            max_connections: self.max_connections(),
            request_timeout_secs: self.request_timeout_secs(),
        }
    }
}

fn parse_env<T>(key: &str) -> Option<T>
where
    T: FromStr,
{
    std::env::var(key)
        .ok()
        .and_then(|raw| raw.trim().parse().ok())
}
