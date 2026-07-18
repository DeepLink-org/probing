//! Feature modules for the Python extension (`probing-python`).
//!
//! Grouped by concern:
//!
//! | Module | Role |
//! |--------|------|
//! | [`python`] | PyO3 glue — bridge, bindings, tracing |
//! | [`stacktrace`] | Call-stack capture, merge, tracers (vm / pprof / dynamic) |
//! | [`torch`] | Module profiling from `python.torch_trace` |
//! | [`flamegraph`] | Shared flamegraph render + distributed stack fold merge |
//! | [`crash`] | Fatal-signal backtrace / grace |

/// PyO3-facing surface (`bridge` / `bindings` / `tracing`).
pub mod python;

/// Unified stack expression, merge, CPython ABI (spy), and tracers.
pub mod stacktrace;

/// Torch module profiling (`python.torch_trace` → flamegraph / JSON).
pub mod torch;

/// Shared HTML/JSON flamegraph renderer (CPU stacks + torch modules).
pub mod flamegraph;

/// Backtrace-on-crash for fatal signals (`SIGSEGV` / `SIGBUS` / …).
pub mod crash;
