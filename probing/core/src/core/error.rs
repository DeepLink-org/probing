//! Error handling for the Probing engine
//!
//! This module defines a single, structured error type ([`EngineError`]) for the
//! whole `probing-core` crate, together with the conversions that wire every
//! sub-system (storage, addressing, runtime, tracing, DataFusion, Arrow, …) into
//! one coherent propagation chain.
//!
//! Design principles
//! -----------------
//! * **One error type per crate.** Everything funnels into [`EngineError`]; the
//!   `?` operator works across layers without hand-written `map_err`.
//! * **Preserve the source chain.** Wrapping variants carry `#[source]`/`#[from]`
//!   so `std::error::Error::source()` walks the real cause, instead of flattening
//!   everything into an opaque string.
//! * **No stringly-typed coercion.** There is intentionally *no*
//!   `From<String>`/`From<&str>`; build errors through the explicit constructors
//!   or the typed variants so categorisation is never lost by accident.
//! * **One boundary conversion.** [`EngineError`] converts to
//!   [`datafusion::error::DataFusionError`] in a single place, so DataFusion trait
//!   implementations can just use `?` instead of re-inventing the mapping.

use thiserror::Error;

use datafusion::error::DataFusionError;

/// Core result type for all Probing engine operations.
pub type Result<T> = std::result::Result<T, EngineError>;

/// Comprehensive error type for the Probing engine.
///
/// Variants are grouped by sub-system. Variants that wrap a foreign error keep
/// that error as their [`source`](std::error::Error::source) so the full causal
/// chain survives propagation.
#[derive(Error, Debug)]
pub enum EngineError {
    // ===== Plugin System Errors =====
    /// Generic plugin error.
    #[error("Plugin error: {0}")]
    PluginError(String),

    /// Plugin not found error.
    #[error("Plugin not found: {0}")]
    PluginNotFound(String),

    // ===== Query Processing Errors =====
    /// General query execution error.
    #[error("Query execution error: {0}")]
    QueryError(String),

    /// Internal engine error.
    #[error("Internal engine error: {0}")]
    InternalError(String),

    /// Error during external API call.
    #[error("API call error: {0}")]
    CallError(String),

    /// Unsupported API call.
    #[error("Unsupported API call")]
    UnsupportedCall,

    // ===== Data Processing Errors =====
    /// Apache Arrow data processing error.
    #[error(transparent)]
    ArrowError(#[from] arrow::error::ArrowError),

    /// DataFusion query processing error.
    #[error(transparent)]
    DataFusionError(#[from] DataFusionError),

    /// (De)serialization failure (bincode payloads in the storage layer).
    #[error("Serialization error: {0}")]
    Serialization(#[from] bincode::Error),

    // ===== Business Logic Errors =====
    /// Cluster management error.
    #[error("Cluster error: {0}")]
    ClusterError(String),

    /// Distributed/local storage error.
    #[error("Storage error: {0}")]
    Storage(String),

    /// Object addressing failure (URI parsing, allocation, …).
    #[error(transparent)]
    Address(#[from] crate::storage::addressing::AddressError),

    // ===== System Errors =====
    /// I/O error (filesystem, sockets, etc.).
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Async runtime bridge failure.
    #[error(transparent)]
    Runtime(#[from] crate::runtime::RuntimeError),

    /// Tracing/span operation failure.
    #[error(transparent)]
    Trace(#[from] crate::trace::TraceError),

    /// Thread/mutex concurrency error.
    #[error("Concurrency error: {0}")]
    ConcurrencyError(String),

    // ===== Configuration Errors =====
    /// General configuration error.
    #[error("Configuration error: {0}")]
    ConfigError(String),

    /// Unsupported configuration option.
    #[error("Unsupported option: {0}")]
    UnsupportedOption(String),

    /// Invalid configuration option value.
    #[error("Invalid option value: {0}={1}")]
    InvalidOptionValue(String, String),

    /// Attempt to modify a read-only option.
    #[error("Read-only option: {0}")]
    ReadOnlyOption(String),

    /// Memtable mmap / validation failure (from `probing-memtable`).
    #[error(transparent)]
    Memtable(#[from] probing_memtable::MemtableError),
}

impl EngineError {
    /// Build a [`EngineError::PluginError`] without boilerplate.
    pub fn plugin(msg: impl Into<String>) -> Self {
        Self::PluginError(msg.into())
    }

    /// Build a [`EngineError::QueryError`].
    pub fn query(msg: impl Into<String>) -> Self {
        Self::QueryError(msg.into())
    }

    /// Build a [`EngineError::InternalError`].
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::InternalError(msg.into())
    }

    /// Build a [`EngineError::ClusterError`].
    pub fn cluster(msg: impl Into<String>) -> Self {
        Self::ClusterError(msg.into())
    }

    /// Build a [`EngineError::ConfigError`].
    pub fn config(msg: impl Into<String>) -> Self {
        Self::ConfigError(msg.into())
    }

    /// Build a [`EngineError::Storage`].
    pub fn storage(msg: impl Into<String>) -> Self {
        Self::Storage(msg.into())
    }

    /// Build a [`EngineError::InvalidOptionValue`] from an option name and detail.
    pub fn invalid_option(option: impl Into<String>, detail: impl std::fmt::Display) -> Self {
        Self::InvalidOptionValue(option.into(), detail.to_string())
    }
}

// Generic lock poison error conversion.
impl<T> From<std::sync::PoisonError<T>> for EngineError {
    fn from(err: std::sync::PoisonError<T>) -> Self {
        EngineError::ConcurrencyError(format!("Lock poisoned: {err}"))
    }
}

/// Single boundary conversion into DataFusion's error type.
///
/// DataFusion trait implementations (`TableProvider`, `ExecutionPlan`, …) must
/// return [`DataFusionError`]. Centralising the mapping here means call sites can
/// simply use `?` / `.map_err(DataFusionError::from)` instead of hand-rolling a
/// `DataFusionError::Execution(format!(...))` everywhere.
impl From<EngineError> for DataFusionError {
    fn from(err: EngineError) -> Self {
        match err {
            EngineError::DataFusionError(e) => e,
            EngineError::ArrowError(e) => DataFusionError::ArrowError(Box::new(e), None),
            other => DataFusionError::External(Box::new(other)),
        }
    }
}
