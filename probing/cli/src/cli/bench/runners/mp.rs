//! `mp` — fully multi-process, time-driven soak.
//!
//! The orchestrator process creates a shared table, then re-execs itself to
//! spawn one (or more) writer processes and several reader processes, each
//! attaching to the same mapping by name/path. Every worker runs for a fixed
//! wall-clock window (synchronised by a shared start instant) and prints a
//! one-line JSON result; the orchestrator aggregates them.
//!
//! This exercises the cross-process read path: a single writer process feeds
//! the shared mapping while several reader processes read lock-free. MEMT is
//! single-writer, so there is exactly one writer process.
//!
//! Worker vs. orchestrator is selected by the `PROBING_BENCH_MP_ROLE`
//! environment variable, so the public surface stays a single `mp` command.

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use clap::ValueEnum;
use probing_memtable::{DType, MemTable};

use super::common::{scan_all, shm_name, temp_path, unique_token, Attach};
use crate::cli::bench::args::{Backend, MpArgs};
use crate::cli::bench::metrics::Report;
use crate::cli::bench::workload::RowGen;

const ENV_ROLE: &str = "PROBING_BENCH_MP_ROLE";
const ENV_ATTACH: &str = "PROBING_BENCH_MP_ATTACH";
const ENV_START_MS: &str = "PROBING_BENCH_MP_START_MS";

pub fn run(args: &MpArgs, json: bool, seed: u64) -> Result<()> {
    match std::env::var(ENV_ROLE) {
        Ok(role) => run_worker(args, &role, seed),
        Err(_) => orchestrate(args, json, seed),
    }
}

// ── orchestrator ───────────────────────────────────────────────────────

fn orchestrate(args: &MpArgs, json: bool, seed: u64) -> Result<()> {
    let spec = args.schema.spec();
    let row_bytes = spec.approx_row_bytes() as u64;
    if args.writers > 1 {
        bail!("mp is single-writer (MEMT); --writers must be 1");
    }
    let writers = 1usize;
    let readers = args.readers;

    // Create the shared backing and keep it alive for the whole run.
    let (attach, _creator) = match args.backend {
        Backend::Heap => bail!("mp requires a shared backend (shm/file/shared), not heap"),
        Backend::Shm => {
            let name = shm_name();
            let creator =
                MemTable::shm(&name, &spec.schema(), args.ring.chunk_size, args.ring.chunks)?;
            (Attach::Shm(name), creator)
        }
        Backend::File => {
            let path = temp_path("mp");
            let creator =
                MemTable::file_at(&path, &spec.schema(), args.ring.chunk_size, args.ring.chunks)?;
            (Attach::File(path), creator)
        }
        Backend::Shared => {
            let name = format!("mp-{}", unique_token());
            let creator =
                MemTable::shared(&name, &spec.schema(), args.ring.chunk_size, args.ring.chunks)?;
            let path = creator.path().expect("shared path").to_path_buf();
            (Attach::File(path), creator)
        }
    };

    let exe = std::env::current_exe().context("resolve current executable")?;
    let passthrough = passthrough_args(args);
    // Give every child time to launch and attach before the measured window.
    let start_ms = now_ms() + 1_000;

    let mut children: Vec<(String, Child)> = Vec::with_capacity(writers + readers);
    for i in 0..writers {
        children.push((
            "writer".into(),
            spawn_worker(&exe, &passthrough, "writer", &attach, start_ms, seed ^ (i as u64 + 1))?,
        ));
    }
    for i in 0..readers {
        children.push((
            "reader".into(),
            spawn_worker(&exe, &passthrough, "reader", &attach, start_ms, seed ^ (0x100 + i as u64))?,
        ));
    }

    // Collect results (each worker self-terminates after the window).
    let mut write_rows = 0u64;
    let mut read_rows = 0u64;
    let mut read_passes = 0u64;
    let mut worker_pids: Vec<u64> = Vec::new();
    let mut max_elapsed = 0.0f64;
    let mut failures = 0usize;

    for (role, child) in children {
        let out = child.wait_with_output().context("await worker")?;
        if !out.status.success() {
            failures += 1;
            eprintln!("worker {role} exited with {:?}", out.status.code());
            continue;
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        let line = stdout.lines().rev().find(|l| !l.trim().is_empty());
        let Some(line) = line else {
            failures += 1;
            continue;
        };
        let v: serde_json::Value =
            serde_json::from_str(line.trim()).with_context(|| format!("parse worker output: {line}"))?;
        let rows = v.get("rows").and_then(|x| x.as_u64()).unwrap_or(0);
        let passes = v.get("passes").and_then(|x| x.as_u64()).unwrap_or(0);
        let elapsed = v.get("elapsed_s").and_then(|x| x.as_f64()).unwrap_or(0.0);
        if let Some(pid) = v.get("pid").and_then(|x| x.as_u64()) {
            worker_pids.push(pid);
        }
        max_elapsed = max_elapsed.max(elapsed);
        match role.as_str() {
            "writer" => write_rows += rows,
            "reader" => {
                read_rows += rows;
                read_passes += passes;
            }
            _ => {}
        }
    }

    let window = Duration::from_secs_f64(max_elapsed.max(1e-9));
    let mut report = Report::new(format!("mp · {:?} · {:?}", args.backend, args.schema.schema));
    report
        .text("backend", format!("{:?}", args.backend))
        .text("schema", format!("{:?}", args.schema.schema))
        .count("writer procs", writers as u64)
        .count("reader procs", readers as u64)
        .duration("window", window)
        .count("rows written", write_rows)
        .rate("write rate", write_rows, window, "rows")
        .byte_rate("write bw", write_rows * row_bytes, window);
    if readers > 0 {
        report
            .count("scan passes", read_passes)
            .count("rows scanned", read_rows)
            .rate("read rate", read_rows, window, "rows")
            .byte_rate("read bw", read_rows * row_bytes, window);
    }
    if failures > 0 {
        report.count("failed workers", failures as u64);
    }
    report.emit(json);

    if let Attach::File(p) = &attach {
        if matches!(args.backend, Backend::File) {
            let _ = std::fs::remove_file(p);
        }
    }
    if failures > 0 {
        bail!("{failures} worker(s) failed");
    }
    Ok(())
}

/// Flags that reproduce the table geometry in a child (role/attach/start go
/// through the environment).
fn passthrough_args(args: &MpArgs) -> Vec<String> {
    let kind = args
        .schema
        .schema
        .to_possible_value()
        .map(|p| p.get_name().to_string())
        .unwrap_or_else(|| "metrics".into());
    let backend = args
        .backend
        .to_possible_value()
        .map(|p| p.get_name().to_string())
        .unwrap_or_else(|| "shared".into());
    vec![
        "bench".into(),
        "mp".into(),
        "--schema".into(),
        kind,
        "--wide-cols".into(),
        args.schema.wide_cols.to_string(),
        "--str-len".into(),
        args.schema.str_len.to_string(),
        "--chunk-size".into(),
        args.ring.chunk_size.to_string(),
        "--chunks".into(),
        args.ring.chunks.to_string(),
        "--backend".into(),
        backend,
        "--duration".into(),
        args.duration.to_string(),
    ]
}

fn spawn_worker(
    exe: &std::path::Path,
    passthrough: &[String],
    role: &str,
    attach: &Attach,
    start_ms: u128,
    seed: u64,
) -> Result<Child> {
    Command::new(exe)
        .args(passthrough)
        .args(["--seed", &seed.to_string()])
        .env(ENV_ROLE, role)
        .env(ENV_ATTACH, attach.encode())
        .env(ENV_START_MS, start_ms.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("spawn {role} worker"))
}

// ── worker ───────────────────────────────────────────────────────────────

fn run_worker(args: &MpArgs, role: &str, seed: u64) -> Result<()> {
    let attach = Attach::parse(&std::env::var(ENV_ATTACH).context("missing attach env")?)?;
    let start_ms: u128 = std::env::var(ENV_START_MS)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(now_ms);
    let duration = Duration::from_secs(args.duration.max(1));
    let spec = args.schema.spec();

    // Attach to the shared table (retry briefly in case of a startup race).
    let mut table = open_with_retry(&attach)?;

    spin_until(start_ms);
    let t0 = Instant::now();
    let (rows, passes) = match role {
        "writer" => {
            let mut gen = RowGen::new(spec.clone(), seed, (std::process::id() as i64) << 20);
            let mut scratch: Vec<f64> = Vec::new();
            let mut rows = 0u64;
            while t0.elapsed() < duration {
                for _ in 0..256 {
                    let values = gen.values(&mut scratch);
                    table.push_row_unchecked(&values);
                }
                rows += 256;
            }
            (rows, 0u64)
        }
        "reader" => {
            let dtypes: Vec<DType> = (0..spec.schema().cols.len())
                .map(|i| spec.schema().cols[i].dtype)
                .collect();
            let mut rows = 0u64;
            let mut passes = 0u64;
            let mut sink = 0u64;
            while t0.elapsed() < duration {
                let (s, n) = scan_all(&table, &dtypes);
                sink = sink.wrapping_add(s);
                rows += n;
                passes += 1;
            }
            std::hint::black_box(sink);
            (rows, passes)
        }
        other => bail!("unknown worker role: {other}"),
    };
    let elapsed = t0.elapsed().as_secs_f64();

    let out = serde_json::json!({
        "role": role,
        "pid": std::process::id(),
        "rows": rows,
        "passes": passes,
        "elapsed_s": elapsed,
    });
    println!("{out}");
    Ok(())
}

fn open_with_retry(attach: &Attach) -> Result<MemTable> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match attach.open() {
            Ok(t) => return Ok(t),
            Err(e) if Instant::now() < deadline => {
                let _ = e;
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(e).context("attach to shared table"),
        }
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn spin_until(start_ms: u128) {
    while now_ms() < start_ms {
        std::thread::sleep(Duration::from_millis(1));
    }
}
