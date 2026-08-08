use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use datafusion::error::{DataFusionError, Result as DataFusionResult};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::client::conn;
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use probing_core::core::federation::{
    fanout_strict_enabled, remote_fanout_concurrency, remote_query_timeout, FanoutHttpMethod,
    FanoutHttpRequest, FanoutHttpResponse, FanoutScope, FanoutService, FanoutStats,
    PeerQueryOutcome,
};
use probing_proto::prelude::*;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::Semaphore;

use super::types::FanoutOutcome;
use crate::auth::peer_auth_header_value;

const FANOUT_WORKER_THREADS_ENV: &str = "PROBING_FANOUT_WORKER_THREADS";
const DEFAULT_FANOUT_WORKER_THREADS: usize = 4;

#[async_trait]
pub(super) trait PeerQueryClient: Send + Sync {
    async fn query_leaf(&self, addr: &str, sql: &str) -> anyhow::Result<DataFrame>;
    async fn query_node(&self, addr: &str, sql: &str) -> anyhow::Result<FanoutOutcome>;
}

#[derive(Clone)]
pub(super) struct HttpPeerQueryClient {
    service: Arc<dyn FanoutService>,
}

impl HttpPeerQueryClient {
    pub(super) fn new(service: Arc<dyn FanoutService>) -> Self {
        Self { service }
    }
}

/// Shared HTTP fan-out implementation with a runtime isolated from Axum and
/// DataFusion's caller runtime. All distributed SQL and extension requests are
/// dispatched here, while their public endpoints remain unchanged.
struct HttpFanoutService {
    runtime: Runtime,
    permits: Arc<Semaphore>,
}

impl std::fmt::Debug for HttpFanoutService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HttpFanoutService")
            .field("available_permits", &self.permits.available_permits())
            .finish_non_exhaustive()
    }
}

impl HttpFanoutService {
    fn create() -> anyhow::Result<Self> {
        let worker_threads = fanout_worker_threads();
        let runtime = Builder::new_multi_thread()
            .worker_threads(worker_threads)
            .thread_name("probing-fanout")
            .enable_all()
            .build()?;
        log::info!(
            "initialized isolated fan-out runtime: workers={worker_threads}, concurrency={}",
            remote_fanout_concurrency()
        );
        Ok(Self {
            runtime,
            permits: Arc::new(Semaphore::new(remote_fanout_concurrency())),
        })
    }

    async fn dispatch(
        &self,
        addr: &str,
        request: FanoutHttpRequest,
    ) -> DataFusionResult<FanoutHttpResponse> {
        let addr = normalize_peer_addr(addr);
        let timeout = request.timeout;
        let permits = self.permits.clone();
        self.runtime
            .spawn(async move {
                let request_addr = addr.clone();
                tokio::time::timeout(timeout, async move {
                    let _permit = permits
                        .acquire_owned()
                        .await
                        .map_err(|_| anyhow::anyhow!("fan-out concurrency controller is closed"))?;
                    send_http(&request_addr, request).await
                })
                .await
                .map_err(|_| {
                    anyhow::anyhow!("fan-out request to {addr} timed out after {timeout:?}")
                })?
            })
            .await
            .map_err(|error| transport_error(anyhow::Error::new(error)))?
            .map_err(transport_error)
    }
}

#[derive(Debug)]
struct FanoutTransportError(anyhow::Error);

impl std::fmt::Display for FanoutTransportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:#}", self.0)
    }
}

impl std::error::Error for FanoutTransportError {}

fn transport_error(error: anyhow::Error) -> DataFusionError {
    DataFusionError::External(Box::new(FanoutTransportError(error)))
}

fn fanout_worker_threads() -> usize {
    std::env::var(FANOUT_WORKER_THREADS_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_FANOUT_WORKER_THREADS)
}

fn normalize_peer_addr(addr: &str) -> String {
    addr.trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/')
        .to_string()
}

async fn send_http(addr: &str, request: FanoutHttpRequest) -> anyhow::Result<FanoutHttpResponse> {
    let stream = tokio::net::TcpStream::connect(addr).await?;
    let io = TokioIo::new(stream);
    let (mut sender, connection) = conn::http1::handshake(io).await?;
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            log::debug!("fan-out HTTP connection error: {error}");
        }
    });

    let method = match request.method {
        FanoutHttpMethod::Get => Method::GET,
        FanoutHttpMethod::Post => Method::POST,
        FanoutHttpMethod::Put => Method::PUT,
    };
    let mut builder = Request::builder()
        .method(method)
        .uri(&request.path)
        .header("Host", addr);
    if let Some(content_type) = request.content_type {
        builder = builder.header("Content-Type", content_type);
    }
    if let Some(value) = peer_auth_header_value() {
        builder = builder.header("Authorization", value);
    }
    let response = sender
        .send_request(builder.body(Full::<Bytes>::from(request.body))?)
        .await?;
    let status = response.status().as_u16();
    let body = response.into_body().collect().await?.to_bytes().to_vec();
    Ok(FanoutHttpResponse { status, body })
}

#[async_trait]
impl FanoutService for HttpFanoutService {
    async fn query_peer(
        &self,
        addr: &str,
        sql: &str,
        scope: FanoutScope,
    ) -> DataFusionResult<PeerQueryOutcome> {
        if scope == FanoutScope::Coordinator {
            let body = serde_json::to_vec(&serde_json::json!({
                "expr": sql,
                "cluster": true,
                "hierarchical": true,
                "scope": "node",
            }))
            .map_err(|error| transport_error(anyhow::Error::new(error)))?;
            let response = self
                .request_peer(
                    addr,
                    FanoutHttpRequest {
                        method: FanoutHttpMethod::Post,
                        path: "/apis/cluster/query".into(),
                        content_type: Some("application/json".into()),
                        body,
                        timeout: remote_query_timeout(),
                    },
                )
                .await?;
            let text = String::from_utf8(response.body)
                .map_err(|error| transport_error(anyhow::Error::new(error)))?;
            return parse_node_response(response.status, &text, addr)
                .map(peer_query_outcome)
                .map_err(transport_error);
        }

        let body = serde_json::to_vec(&Message::new(Query {
            expr: sql.to_string(),
            ..Default::default()
        }))
        .map_err(|error| transport_error(anyhow::Error::new(error)))?;
        let response = self
            .request_peer(
                addr,
                FanoutHttpRequest {
                    method: FanoutHttpMethod::Post,
                    path: "/query".into(),
                    content_type: Some("application/json".into()),
                    body,
                    timeout: remote_query_timeout(),
                },
            )
            .await?;
        let text = String::from_utf8(response.body)
            .map_err(|error| transport_error(anyhow::Error::new(error)))?;
        parse_leaf_response(response.status, &text, addr)
            .map(PeerQueryOutcome::complete)
            .map_err(transport_error)
    }

    async fn request_peer(
        &self,
        addr: &str,
        request: FanoutHttpRequest,
    ) -> DataFusionResult<FanoutHttpResponse> {
        self.dispatch(addr, request).await
    }
}

static HTTP_FANOUT_SERVICE: LazyLock<Result<Arc<HttpFanoutService>, String>> =
    LazyLock::new(|| {
        HttpFanoutService::create()
            .map(Arc::new)
            .map_err(|error| format!("{error:#}"))
    });

fn http_service() -> anyhow::Result<Arc<HttpFanoutService>> {
    HTTP_FANOUT_SERVICE
        .as_ref()
        .cloned()
        .map_err(|error| anyhow::anyhow!("failed to initialize fan-out service: {error}"))
}

pub(crate) fn core_service() -> anyhow::Result<Arc<dyn FanoutService>> {
    Ok(http_service()? as Arc<dyn FanoutService>)
}

pub(crate) fn request_peer_blocking(
    addr: &str,
    request: FanoutHttpRequest,
) -> anyhow::Result<FanoutHttpResponse> {
    let service = http_service()?;
    let runtime_handle = service.runtime.handle().clone();
    let addr = addr.to_string();
    let wait_timeout = request
        .timeout
        .saturating_add(std::time::Duration::from_secs(1));
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    runtime_handle.spawn(async move {
        let result = service
            .request_peer(&addr, request)
            .await
            .map_err(anyhow::Error::new);
        let _ = sender.send(result);
    });
    receiver
        .recv_timeout(wait_timeout)
        .map_err(|error| anyhow::anyhow!("fan-out service response wait failed: {error}"))?
}

fn peer_query_outcome(outcome: FanoutOutcome) -> PeerQueryOutcome {
    let meta = outcome.meta;
    PeerQueryOutcome::with_stats(
        outcome.dataframe,
        FanoutStats {
            nodes_succeeded: meta.nodes_queried.saturating_sub(meta.nodes_failed.len()),
            nodes_failed: meta.nodes_failed,
            peer_batches_dropped: meta.peer_batches_dropped,
            partial: meta.partial,
        },
    )
}

#[async_trait]
impl PeerQueryClient for HttpPeerQueryClient {
    async fn query_leaf(&self, addr: &str, sql: &str) -> anyhow::Result<DataFrame> {
        let outcome = self
            .service
            .query_peer(addr, sql, FanoutScope::Flat)
            .await
            .map_err(anyhow::Error::new)?;
        Ok(outcome.dataframe)
    }

    async fn query_node(&self, addr: &str, sql: &str) -> anyhow::Result<FanoutOutcome> {
        let body = serde_json::to_vec(&serde_json::json!({
            "expr": sql,
            "cluster": true,
            "hierarchical": true,
            "scope": "node",
        }))?;
        let response = self
            .service
            .request_peer(
                addr,
                FanoutHttpRequest {
                    method: FanoutHttpMethod::Post,
                    path: "/apis/cluster/query".into(),
                    content_type: Some("application/json".into()),
                    body,
                    timeout: remote_query_timeout(),
                },
            )
            .await
            .map_err(anyhow::Error::new)?;
        let text = String::from_utf8(response.body)?;
        parse_node_response(response.status, &text, addr)
    }
}

pub async fn remote_query_df(addr: &str, sql: &str) -> anyhow::Result<DataFrame> {
    HttpPeerQueryClient::new(core_service()?)
        .query_leaf(addr, sql)
        .await
}

fn parse_leaf_response(status: u16, text: &str, addr: &str) -> anyhow::Result<DataFrame> {
    if status >= 400 {
        if status == 503 && !fanout_strict_enabled() {
            if let Ok(dataframe) = decode_query_message_dataframe(text) {
                log::warn!("accepted partial 503 dataframe from {addr}");
                return Ok(dataframe);
            }
            if let Ok(response) = decode_cluster_query_response(text) {
                log::warn!("accepted partial 503 cluster response from {addr}");
                return Ok(response.dataframe);
            }
        }
        anyhow::bail!("HTTP {status}: {text}");
    }
    decode_query_message_dataframe(text)
}

fn parse_node_response(status: u16, text: &str, addr: &str) -> anyhow::Result<FanoutOutcome> {
    if status >= 400 {
        if status == 503 && !fanout_strict_enabled() {
            if let Ok(response) = decode_cluster_query_response(text) {
                log::warn!("accepted partial 503 node aggregate from {addr}");
                return Ok(response);
            }
        }
        anyhow::bail!("HTTP {status}: {text}");
    }
    decode_cluster_query_response(text)
}

fn decode_query_message_dataframe(text: &str) -> anyhow::Result<DataFrame> {
    let message: Message<QueryDataFormat> = serde_json::from_str(text)?;
    match message.payload {
        QueryDataFormat::DataFrame(dataframe) => Ok(dataframe),
        QueryDataFormat::Nil => Ok(DataFrame::default()),
        QueryDataFormat::Error(error) => anyhow::bail!("remote query: {}", error.message),
        QueryDataFormat::TimeSeries(_) => anyhow::bail!("unexpected timeseries"),
    }
}

fn decode_cluster_query_response(text: &str) -> anyhow::Result<FanoutOutcome> {
    let value: serde_json::Value = serde_json::from_str(text)?;
    if let Some(error) = value.get("error").and_then(|value| value.as_str()) {
        anyhow::bail!("remote cluster query: {error}");
    }
    Ok(serde_json::from_value(value)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::cluster_fanout::FanoutMeta;

    #[test]
    fn core_outcome_preserves_child_partial_metadata() {
        let outcome = peer_query_outcome(FanoutOutcome {
            dataframe: DataFrame::default(),
            meta: FanoutMeta {
                cluster: true,
                hierarchical: true,
                scope: "node".into(),
                nodes_queried: 3,
                nodes_failed: vec!["rank-3: timeout".into()],
                peer_batches_dropped: 1,
                node_aggregators_queried: 0,
                local_ranks_queried: 2,
                partial: true,
            },
        });

        assert_eq!(outcome.stats.nodes_succeeded, 2);
        assert_eq!(outcome.stats.nodes_failed, vec!["rank-3: timeout"]);
        assert_eq!(outcome.stats.peer_batches_dropped, 1);
        assert!(outcome.stats.partial);
    }

    #[test]
    fn worker_count_rejects_zero() {
        std::env::set_var(FANOUT_WORKER_THREADS_ENV, "0");
        assert_eq!(fanout_worker_threads(), DEFAULT_FANOUT_WORKER_THREADS);
        std::env::remove_var(FANOUT_WORKER_THREADS_ENV);
    }

    #[test]
    fn fanout_tasks_run_on_named_isolated_workers() {
        let service = HttpFanoutService::create().expect("fan-out runtime");
        let task = service.runtime.spawn(async {
            std::thread::current()
                .name()
                .unwrap_or_default()
                .to_string()
        });
        let thread_name = service.runtime.block_on(task).expect("fan-out task");
        assert_eq!(thread_name, "probing-fanout");
    }
}
