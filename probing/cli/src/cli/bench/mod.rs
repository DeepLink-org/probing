//! Hidden `bench` command: a load generator and stress harness for the
//! probing data layer (hot MEMT ring + cold MEMC segments).
//!
//! This is an internal/diagnostic command (hidden from `--help`). Run
//! `probing bench <subcommand> --help` for per-workload options.

pub mod args;
pub mod metrics;
pub mod runners;
pub mod workload;

use anyhow::Result;
use clap::{Args, Subcommand};

use args::{ColdscanArgs, CompactArgs, MixedArgs, MpArgs, ScanArgs, WriteArgs};

/// Stress and benchmark the in-process data layer.
#[derive(Args, Debug)]
pub struct BenchCommand {
    /// Emit machine-readable JSON instead of a formatted table.
    #[arg(long, global = true)]
    pub json: bool,

    /// PRNG seed for reproducible synthetic data.
    #[arg(long, global = true, default_value_t = 0x00C0_FFEE)]
    pub seed: u64,

    #[command(subcommand)]
    pub command: BenchSub,
}

#[derive(Subcommand, Debug)]
pub enum BenchSub {
    /// Write throughput across storage backends and writer counts.
    Write(WriteArgs),
    /// Sequential scan throughput over a freshly populated hot ring.
    Scan(ScanArgs),
    /// Cold-tier compaction throughput and hot→cold compression ratio.
    Compact(CompactArgs),
    /// Cold-segment read + decode throughput.
    Coldscan(ColdscanArgs),
    /// End-to-end pipeline: writers + background compactor + readers.
    Mixed(MixedArgs),
    /// Multi-process, time-driven soak: writer + reader processes share a table.
    Mp(MpArgs),
}

impl BenchCommand {
    pub fn run(&self) -> Result<()> {
        let seed = self.seed;
        let json = self.json;
        match &self.command {
            BenchSub::Write(a) => runners::write::run(a, json, seed),
            BenchSub::Scan(a) => runners::scan::run(a, json, seed),
            BenchSub::Compact(a) => runners::compact::run(a, json, seed),
            BenchSub::Coldscan(a) => runners::coldscan::run(a, json, seed),
            BenchSub::Mixed(a) => runners::mixed::run(a, json, seed),
            BenchSub::Mp(a) => runners::mp::run(a, json, seed),
        }
    }
}
