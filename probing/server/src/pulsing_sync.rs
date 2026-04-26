//! Low-intrusion Pulsing → probing cluster sync.
//!
//! Reads `{data_dir}/{pid}/pulsing.members` — a MEMH hash table written
//! continuously by Pulsing's gossip subsystem — and merges the membership
//! view into probing's `CLUSTER` via [`merge_pulsing_nodes`].
//!
//! # Design goals
//!
//! * **Zero changes to Pulsing** — we only read the file it already writes.
//! * **No Python, no HTTP** — purely file-based; works even before the Python
//!   interpreter is available.
//! * **Opt-out** — set `PROBING_PULSING_SYNC=0` to disable.
//! * **Additive** — the existing HTTP `/apis/nodes/sync` path still works;
//!   the two paths both call `merge_pulsing_nodes` which is idempotent.
//!
//! # File location
//!
//! `{PROBING_DATA_DIR}/{pid}/pulsing.members`
//!
//! where `PROBING_DATA_DIR` defaults to `/tmp/probing` (see
//! `probing_memtable::discover::default_dir`).
//!
//! # Value format
//!
//! ```text
//! key   = node_id  (opaque string, kept as role_name for inspection)
//! value = "{SocketAddr}|{status}|{epoch}"
//!         status ∈ online | suspect | fail | handshake | tombstone
//! ```
//!
//! The `SocketAddr` is Pulsing's transport address (e.g. `192.168.1.1:4500`).
//! Probing's HTTP-server address is derived by replacing the port with
//! `PROBING_PORT` (default `8080`).

use std::time::Duration;

use probing_core::core::cluster::merge_pulsing_nodes;
use probing_memtable::discover::default_dir;
use probing_memtable::{detect_table, MemhView, TableKind, TypedValue};
use probing_proto::prelude::Node;

// ── Path helpers ──────────────────────────────────────────────────────

/// Path of Pulsing's members MEMH for the current process.
fn pulsing_members_path() -> std::path::PathBuf {
    default_dir()
        .join(std::process::id().to_string())
        .join("pulsing.members")
}

// ── Address mapping ───────────────────────────────────────────────────

/// Derive the probing HTTP server address for a remote node from its Pulsing
/// transport address.
///
/// Extracts the hostname / IP from `pulsing_addr` and substitutes the port
/// with `PROBING_PORT` (env var, default `8080`).
fn probing_addr_for(pulsing_addr: &str) -> (String /* host */, String /* addr */) {
    // pulsing_addr is a SocketAddr: "192.168.1.1:4500" or "[::1]:4500"
    let host = match pulsing_addr.rfind(':') {
        Some(pos) => {
            let h = &pulsing_addr[..pos];
            h.trim_start_matches('[').trim_end_matches(']').to_string()
        }
        None => pulsing_addr.to_string(),
    };
    let port = std::env::var("PROBING_PORT").unwrap_or_else(|_| "8080".to_string());
    let addr = format!("{host}:{port}");
    (host, addr)
}

// ── Record parsing ────────────────────────────────────────────────────

/// Parse one MEMH entry (`key` = node_id, `value` = pipe-separated triple)
/// into a probing [`Node`].  Returns `None` for malformed or dead entries.
fn parse_member(node_id: &str, value: &str) -> Option<Node> {
    let mut parts = value.splitn(3, '|');
    let pulsing_addr = parts.next()?.trim();
    let status = parts.next()?.trim().to_string();
    // epoch (third field) unused — merge_pulsing_nodes stamps its own timestamp

    // Don't add permanently-dead entries to the live cluster view.
    if status == "tombstone" || status == "fail" {
        return None;
    }

    let (host, addr) = probing_addr_for(pulsing_addr);

    Some(Node {
        host,
        addr,
        status: Some(status),
        // Preserve node_id so operators can correlate with Pulsing's own view.
        role_name: Some(node_id.to_string()),
        ..Node::default()
    })
}

// ── Public API ────────────────────────────────────────────────────────

/// Read `pulsing.members` once and merge any live members into probing's
/// cluster view.  Returns the number of nodes merged.
///
/// Safe to call at any frequency; returns `0` if the file does not yet exist
/// (Pulsing not started) or contains an unrecognised format.
pub fn sync_once() -> usize {
    let path = pulsing_members_path();
    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(_) => return 0,
    };
    if detect_table(&data) != Some(TableKind::Hash) {
        return 0;
    }
    let view = match MemhView::new(&data) {
        Ok(v) => v,
        Err(_) => return 0,
    };

    let nodes: Vec<Node> = view
        .iter()
        .filter_map(|(k, v)| {
            if let TypedValue::Str(s) = v {
                parse_member(k, s)
            } else {
                None
            }
        })
        .collect();

    let count = nodes.len();
    if count > 0 {
        merge_pulsing_nodes(nodes);
    }
    count
}

/// Spawn an infinite loop that calls [`sync_once`] every `interval`.
///
/// Designed to be spawned onto the probing server's Tokio runtime.  The loop
/// is a no-op until Pulsing writes its first members entry.
pub async fn sync_loop(interval: Duration) {
    if std::env::var("PROBING_PULSING_SYNC")
        .unwrap_or_default()
        .trim()
        .to_lowercase()
        == "0"
    {
        return;
    }

    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    ticker.tick().await; // skip the first immediate tick

    loop {
        ticker.tick().await;
        let n = sync_once();
        if n > 0 {
            log::debug!("pulsing_sync: merged {n} members into cluster view");
        }
    }
}
