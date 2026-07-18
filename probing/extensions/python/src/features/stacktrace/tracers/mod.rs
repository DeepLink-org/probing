//! Stack acquisition tracers.
//!
//! | Tracer | Trigger | Consumers |
//! |--------|---------|-----------|
//! | [`vm`] | Eval-frame hook | Always on when tracing; records Python frames |
//! | [`pprof`] | `SIGPROF` | SQL / continuous CPU flamegraph |
//! | [`dynamic`] | `SIGUSR2` + sync walk | HTTP/command on-demand backtrace |
//!
//! Python frame payloads for [`pprof`] and [`dynamic`] always come from [`vm`].

pub mod dynamic;
pub mod pprof;
pub mod vm;
