pub mod config;
pub mod convert;
pub mod pprof;
pub mod python_api;
pub mod spy;
/// Native stack capture, Python/native merge, and signal handling (`SIGUSR2`).
pub mod stack_tracer;
pub mod torch;
pub mod tracing;
/// Python eval-frame hook; source of Python call frames for stack tracing.
pub mod vm_tracer;
