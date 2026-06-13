//! Workload runners. Each `run` function executes one `bench` subcommand
//! and prints a [`Report`](super::metrics::Report).

pub mod coldscan;
pub mod common;
pub mod compact;
pub mod mixed;
pub mod mp;
pub mod scan;
pub mod write;
