//! PyO3-facing feature surface (distinct from crate-root [`crate::python`] lifecycle).
//!
//! | Module | Role |
//! |--------|------|
//! | [`bridge`] | Thread detach, `Ele` ↔ Python, error mapping |
//! | [`bindings`] | `_core` pyfunctions: config / query / callstack / eval |
//! | [`tracing`] | `inspect.trace` spans and crash span snapshot |

pub mod bindings;
pub mod bridge;
pub mod tracing;
