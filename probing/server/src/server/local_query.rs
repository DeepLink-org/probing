use axum::extract::Query as AxumQuery;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::client::conn;
use hyper::Request;
use hyper_util::rt::TokioIo;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct LocalPidQuery {
    pid: i32,
}

pub async fn query_local_pid(
    AxumQuery(params): AxumQuery<LocalPidQuery>,
    body: String,
) -> impl IntoResponse {
    match forward_query_to_pid(params.pid, body).await {
        Ok(response) => (StatusCode::OK, response).into_response(),
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            format!("Failed to query local pid {}: {}", params.pid, err),
        )
            .into_response(),
    }
}

async fn forward_query_to_pid(pid: i32, body: String) -> anyhow::Result<String> {
    #[cfg(target_os = "linux")]
    let path = format!("\0probing-{pid}");
    #[cfg(not(target_os = "linux"))]
    let path = {
        let file_path = std::env::temp_dir().join(format!("probing-{pid}.sock"));
        file_path.to_string_lossy().to_string()
    };

    let stream = tokio::net::UnixStream::connect(path).await?;
    let io = TokioIo::new(stream);
    let (mut sender, connection) = conn::http1::handshake(io).await?;
    tokio::spawn(async move {
        if let Err(err) = connection.await {
            log::debug!("Local pid query connection error: {err}");
        }
    });

    let request = Request::builder()
        .method("POST")
        .uri("/query")
        .header("Content-Type", "application/json")
        .body(Full::<Bytes>::from(body))?;
    let response = sender.send_request(request).await?;
    let status = response.status();
    let bytes = response.into_body().collect().await?.to_bytes();
    let text = String::from_utf8_lossy(&bytes).to_string();
    if !status.is_success() {
        anyhow::bail!("target returned HTTP {status}: {text}");
    }
    Ok(text)
}
