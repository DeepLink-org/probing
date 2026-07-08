pub mod config;
pub mod convert;
/// Backtrace-on-crash handler for fatal signals (`SIGSEGV`/`SIGBUS`/...).
pub mod crash;
pub mod flamegraph;
pub mod native_bridge;
pub mod pprof;
pub mod py_result;
pub mod python_api;
pub mod skills_api;
pub mod spy;
/// Native stack capture, Python/native merge, and signal handling (`SIGUSR2`).
pub mod stack_tracer;
pub mod torch;
pub mod tracing;
/// Python eval-frame hook; source of Python call frames for stack tracing.
pub mod vm_tracer;
