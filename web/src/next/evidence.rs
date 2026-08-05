//! Shared request/receipt contract for evidence rendered by Next pages.
//!
//! A page may issue several HTTP/SQL requests, but all requests triggered by one
//! refresh must carry the same [`EvidenceRequest`]. Receipts preserve scope,
//! freshness, partial failures, and whether pinned investigation coordinates
//! actually matched the returned data.

use probing_proto::prelude::{DataFrame, Ele};
use probing_proto::protocol::training::StepMatrixResponse;

use crate::state::investigation::InvestigationContext;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvidenceScope {
    LocalProcess,
    ClusterRegistry,
    ClusterFanout,
}

impl EvidenceScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::LocalProcess => "local process",
            Self::ClusterRegistry => "cluster registry",
            Self::ClusterFanout => "cluster fan-out",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextMatch {
    /// No rank/host/device/span/step coordinate was requested.
    Unpinned,
    /// The returned evidence contains the requested coordinate.
    Matched,
    /// The coordinate is valid but cannot be observed in the selected scope.
    OutOfScope,
    /// The selected scope was searched and returned no matching evidence.
    NoMatch,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EvidenceRequest {
    pub refresh_epoch: u64,
    pub requested_at_ms: u64,
    pub scope: EvidenceScope,
    pub window_us: Option<u64>,
    pub context: InvestigationContext,
}

impl EvidenceRequest {
    pub fn new(
        refresh_epoch: u64,
        scope: EvidenceScope,
        window_us: Option<u64>,
        context: InvestigationContext,
    ) -> Self {
        Self {
            refresh_epoch,
            requested_at_ms: now_ms(),
            scope,
            window_us,
            context,
        }
    }

    pub fn context_match(&self, matched: bool) -> ContextMatch {
        if self.context.is_empty() {
            ContextMatch::Unpinned
        } else if matched {
            ContextMatch::Matched
        } else if self.scope == EvidenceScope::LocalProcess {
            ContextMatch::OutOfScope
        } else {
            ContextMatch::NoMatch
        }
    }

    pub fn for_scope(&self, scope: EvidenceScope, window_us: Option<u64>) -> Self {
        Self {
            refresh_epoch: self.refresh_epoch,
            requested_at_ms: self.requested_at_ms,
            scope,
            window_us,
            context: self.context.clone(),
        }
    }
}

/// A successful value and the receipt that defines how it was collected.
///
/// Pages render `value`; Agent context is built from the same payload instead
/// of issuing a second query at a different sampling instant.
#[derive(Clone, Debug, PartialEq)]
pub struct EvidencePayload<T> {
    pub value: T,
    pub receipt: EvidenceReceipt,
}

impl<T> EvidencePayload<T> {
    pub fn new(value: T, receipt: EvidenceReceipt) -> Self {
        Self { value, receipt }
    }
}

pub fn step_matrix_payload(
    matrix: StepMatrixResponse,
    request: &EvidenceRequest,
) -> EvidencePayload<StepMatrixResponse> {
    let rows = matrix.samples.len();
    let receipt = if matrix.cluster {
        EvidenceReceipt::cluster(
            "train.step",
            request,
            rows,
            matrix.nodes_queried,
            matrix.nodes_failed.len(),
            matrix.partial,
        )
    } else {
        EvidenceReceipt::local("train.step", request, rows)
    };
    EvidencePayload::new(matrix, receipt)
}

pub fn cluster_dataframe_payload(
    source: &'static str,
    request: &EvidenceRequest,
    dataframe: DataFrame,
    peers_queried: usize,
    failed_peers: usize,
    partial: bool,
) -> EvidencePayload<DataFrame> {
    let receipt = EvidenceReceipt::cluster(
        source,
        request,
        dataframe.row_count(),
        peers_queried,
        failed_peers,
        partial,
    );
    EvidencePayload::new(dataframe, receipt)
}

#[derive(Clone, Debug)]
struct EvidenceEntry {
    receipt: EvidenceReceipt,
    preview: String,
}

/// Page-level evidence assembled from the payloads used by the visible UI.
#[derive(Clone, Debug)]
pub struct EvidenceBundle {
    page_id: &'static str,
    refresh_epoch: u64,
    requested_at_ms: u64,
    context: InvestigationContext,
    entries: Vec<EvidenceEntry>,
    failures: Vec<String>,
}

impl EvidenceBundle {
    pub fn new(page_id: &'static str, request: &EvidenceRequest) -> Self {
        Self {
            page_id,
            refresh_epoch: request.refresh_epoch,
            requested_at_ms: request.requested_at_ms,
            context: request.context.clone(),
            entries: Vec::new(),
            failures: Vec::new(),
        }
    }

    pub fn push(&mut self, receipt: &EvidenceReceipt, preview: String) {
        self.entries.push(EvidenceEntry {
            receipt: receipt.clone(),
            preview,
        });
    }

    pub fn push_failure(&mut self, source: &str, detail: &str) {
        self.failures
            .push(format!("[{source}]\n(unavailable: {detail})"));
    }

    pub fn render(&self) -> String {
        let context = self.context.coordinates_summary();
        let mut parts = vec![format!(
            "[next evidence]\npage={} · refresh={} · requested_at_ms={} · context={context}",
            self.page_id, self.refresh_epoch, self.requested_at_ms,
        )];
        parts.extend(self.entries.iter().map(|entry| {
            let receipt = &entry.receipt;
            format!(
                "[{} · {} · rows={} · peers={} · failed={} · partial={} · collected_at_ms={}]\n{}",
                receipt.source,
                receipt.scope.label(),
                receipt.rows,
                receipt.peers_queried,
                receipt.failed_peers,
                receipt.partial,
                receipt.collected_at_ms,
                entry.preview,
            )
        }));
        parts.extend(self.failures.iter().cloned());
        parts.join("\n\n")
    }
}

pub fn dataframe_preview(dataframe: &DataFrame, max_rows: usize) -> String {
    let rows = dataframe.row_count();
    if rows == 0 || dataframe.names.is_empty() {
        return "(empty)".into();
    }
    let take = rows.min(max_rows);
    let mut lines = vec![dataframe.names.join("\t")];
    for row in 0..take {
        let cells = dataframe
            .cols
            .iter()
            .map(|column| evidence_cell(&column.get(row)))
            .collect::<Vec<_>>();
        lines.push(cells.join("\t"));
    }
    if rows > take {
        lines.push(format!("… +{} rows", rows - take));
    }
    lines.join("\n")
}

fn evidence_cell(value: &Ele) -> String {
    match value {
        Ele::Nil => "—".into(),
        Ele::BOOL(value) => value.to_string(),
        Ele::I32(value) => value.to_string(),
        Ele::I64(value) => value.to_string(),
        Ele::F32(value) => format!("{value:.4}"),
        Ele::F64(value) => format!("{value:.4}"),
        Ele::Text(value) | Ele::Url(value) => value.clone(),
        Ele::DataTime(value) => value.to_string(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidenceReceipt {
    pub source: &'static str,
    pub refresh_epoch: u64,
    pub collected_at_ms: u64,
    pub scope: EvidenceScope,
    pub rows: usize,
    pub peers_queried: usize,
    pub failed_peers: usize,
    pub partial: bool,
    pub context_match: ContextMatch,
}

impl EvidenceReceipt {
    pub fn local(source: &'static str, request: &EvidenceRequest, rows: usize) -> Self {
        Self {
            source,
            refresh_epoch: request.refresh_epoch,
            collected_at_ms: now_ms(),
            scope: request.scope,
            rows,
            peers_queried: 1,
            failed_peers: 0,
            partial: false,
            context_match: ContextMatch::Unpinned,
        }
    }

    pub fn cluster(
        source: &'static str,
        request: &EvidenceRequest,
        rows: usize,
        peers_queried: usize,
        failed_peers: usize,
        partial: bool,
    ) -> Self {
        Self {
            source,
            refresh_epoch: request.refresh_epoch,
            collected_at_ms: now_ms(),
            scope: request.scope,
            rows,
            peers_queried,
            failed_peers,
            partial,
            context_match: ContextMatch::Unpinned,
        }
    }

    pub fn with_context_match(mut self, context_match: ContextMatch) -> Self {
        self.context_match = context_match;
        self
    }
}

#[cfg(target_arch = "wasm32")]
fn now_ms() -> u64 {
    js_sys::Date::now() as u64
}

#[cfg(not(target_arch = "wasm32"))]
fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_refresh_keeps_a_stable_epoch_across_receipts() {
        let request = EvidenceRequest::new(
            42,
            EvidenceScope::ClusterFanout,
            Some(300_000_000),
            InvestigationContext::default(),
        );
        let devices = EvidenceReceipt::cluster("gpu.utilization", &request, 64, 8, 0, false);
        let allocator = EvidenceReceipt::cluster("python.torch_trace", &request, 64, 8, 1, true);

        assert_eq!(devices.refresh_epoch, allocator.refresh_epoch);
        assert_eq!(devices.scope, allocator.scope);
        assert!(!devices.partial);
        assert!(allocator.partial);
    }

    #[test]
    fn unmatched_pins_are_not_silently_relabelled_as_local_data() {
        let context = InvestigationContext {
            rank: Some(57),
            host: Some("node-07".into()),
            device_id: Some(1),
            ..Default::default()
        };
        let local = EvidenceRequest::new(1, EvidenceScope::LocalProcess, None, context.clone());
        let cluster = EvidenceRequest::new(2, EvidenceScope::ClusterFanout, None, context);

        assert_eq!(local.context_match(false), ContextMatch::OutOfScope);
        assert_eq!(cluster.context_match(false), ContextMatch::NoMatch);
        assert_eq!(cluster.context_match(true), ContextMatch::Matched);
    }

    #[test]
    fn bundle_keeps_scope_partial_state_and_sampling_time_with_preview() {
        let request = EvidenceRequest::new(
            9,
            EvidenceScope::ClusterFanout,
            None,
            InvestigationContext::default(),
        );
        let receipt = EvidenceReceipt::cluster("python.comm_collective", &request, 12, 8, 2, true);
        let mut bundle = EvidenceBundle::new("training", &request);
        bundle.push(&receipt, "rank\tmax_ms\n57\t12.4".into());

        let rendered = bundle.render();
        assert!(rendered.contains("page=training · refresh=9"));
        assert!(rendered.contains("cluster fan-out · rows=12 · peers=8 · failed=2 · partial=true"));
        assert!(rendered.contains("rank\tmax_ms\n57\t12.4"));
    }

    #[test]
    fn cluster_dataframe_payload_does_not_discard_partial_metadata() {
        let request = EvidenceRequest::new(
            3,
            EvidenceScope::ClusterFanout,
            None,
            InvestigationContext::default(),
        );
        let dataframe = DataFrame::new(
            vec!["rank".into()],
            vec![probing_proto::prelude::Seq::SeqI32(vec![0, 1])],
        );
        let payload =
            cluster_dataframe_payload("python.comm_collective", &request, dataframe, 8, 2, true);

        assert_eq!(payload.receipt.rows, 2);
        assert_eq!(payload.receipt.peers_queried, 8);
        assert_eq!(payload.receipt.failed_peers, 2);
        assert!(payload.receipt.partial);
    }
}
