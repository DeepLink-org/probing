pub mod config;
pub mod convert;
/// Backtrace-on-crash handler for fatal signals (`SIGSEGV`/`SIGBUS`/...).
pub mod crash;
pub mod flamegraph;
pub mod native_bridge;
pub mod pprof;
pub mod py_result;
pub mod python_api;
pub mod spy;
/// Async-signal-safe raw stack snapshotting (SIGPROF / SIGUSR2).
pub mod stack_capture;
/// Unified Python / native stack merge.
pub mod stack_merge;
/// On-demand stack capture (`SIGUSR2`) and synchronous walks.
pub mod stack_tracer;
pub mod torch;
pub mod tracing;
/// Python eval-frame hook; source of Python call frames for stack tracing.
pub mod vm_tracer;
