//! HTTP control-plane modules and stable server entry points.

pub mod api;
mod app;
mod listener;
mod query_dto;
mod repl;
mod runtime;
mod settings;
mod spa;
pub mod sql_guard;

#[cfg(unix)]
mod local_auth;

pub use app::TOP_LEVEL_ROUTES;
pub(crate) use listener::start_local_listener;
pub use listener::{local_server, remote_server, start_local, start_remote};
pub use runtime::SERVER_RUNTIME;
pub use settings::{bind_address_from_port, sync_env_settings};

pub mod cluster;
pub mod cluster_fanout;
pub mod cluster_query;
pub mod config;
pub mod error;
pub mod file_api;
pub mod health;
pub mod local_query;
pub mod middleware;
pub mod system;
pub mod training;
