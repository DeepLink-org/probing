//! Local and remote HTTP listener transports.

use anyhow::Result;

use super::app::build_app;

fn local_socket_path() -> String {
    #[cfg(target_os = "linux")]
    {
        format!("\0probing-{}", std::process::id())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let pid = std::process::id();
        let temp_dir = std::env::temp_dir();
        temp_dir
            .join(format!("probing-{}.sock", pid))
            .to_string_lossy()
            .to_string()
    }
}

async fn serve_local(socket_path: String) -> Result<()> {
    #[cfg(not(target_os = "linux"))]
    if std::path::Path::new(&socket_path).exists() {
        std::fs::remove_file(&socket_path)?;
    }

    log::info!(
        "Starting local server at {}",
        socket_path.replace('\0', "@")
    );

    let app = build_app(false);
    let listener = tokio::net::UnixListener::bind(socket_path)?;
    let listener = super::local_auth::SameUidUnixListener::new(listener);
    axum::serve(listener, app).await?;
    Ok(())
}

pub async fn local_server() -> Result<()> {
    serve_local(local_socket_path()).await
}

/// Initialize the in-process engine and start the local control listener.
pub fn start_local() {
    crate::bootstrap::start_local();
}

pub(crate) fn start_local_listener() {
    let socket_path = local_socket_path();
    let key = socket_path.replace('\0', "@");
    crate::runtime_state::supervisor().start_local_listener(key, move |_| async move {
        serve_local(socket_path)
            .await
            .map_err(|error| crate::failure::component_failed("local HTTP server", error))
    });
}

pub async fn remote_server(addr: Option<String>) -> Result<()> {
    let addr = addr.unwrap_or_else(|| "0.0.0.0:0".to_string());
    log::info!("Starting probe server at {addr}");
    let (listener, app, bound_addr) = bind_remote_server(&addr).await?;
    probing_core::config::write("server.address", &bound_addr.to_string()).await?;
    publish_remote_address(bound_addr);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn bind_remote_server(
    addr: &str,
) -> Result<(tokio::net::TcpListener, axum::Router, std::net::SocketAddr)> {
    crate::auth::bootstrap_auth_from_env().await;
    let app = build_app(true);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let bound_addr = listener.local_addr()?;
    Ok((listener, app, bound_addr))
}

fn publish_remote_address(addr: std::net::SocketAddr) {
    {
        let mut probing_address = crate::vars::write_probing_address();
        *probing_address = addr.to_string();
    }
    probing_core::core::cluster::set_local_listen_addrs(vec![addr.to_string()]);
    log::info!("probing server is available on: {addr}");
}

async fn run_supervised_remote_server(generation: u64, addr: String) -> Result<()> {
    log::info!("Starting probe server candidate at {addr}");
    let (listener, app, bound_addr) = bind_remote_server(&addr).await?;
    probing_core::config::write("server.address", &bound_addr.to_string()).await?;
    if !crate::runtime_state::supervisor().promote_remote_listener(generation) {
        log::debug!("discarding stale remote listener candidate generation {generation}");
        return Ok(());
    }
    publish_remote_address(bound_addr);
    axum::serve(listener, app).await?;
    Ok(())
}

pub fn start_remote(addr: Option<String>) {
    let requested_addr = addr.unwrap_or_else(|| "0.0.0.0:0".to_string());
    crate::runtime_state::supervisor().start_remote_listener(
        requested_addr.clone(),
        move |generation| async move {
            run_supervised_remote_server(generation, requested_addr)
                .await
                .map_err(|error| crate::failure::component_failed("remote HTTP server", error))
        },
    );
}
