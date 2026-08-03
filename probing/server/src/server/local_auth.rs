//! OS-backed authentication for the local Unix control socket.

use std::io;
use std::time::Duration;

use axum::serve::Listener;
use tokio::net::{unix::SocketAddr, UnixListener, UnixStream};

fn same_uid(peer_uid: u32, server_uid: u32) -> bool {
    peer_uid == server_uid
}

/// Unix listener that admits only clients running as the server's effective UID.
///
/// The check happens immediately after `accept`, before Hyper parses an HTTP
/// request, so every route on the local control socket shares the same boundary.
#[derive(Debug)]
pub(crate) struct SameUidUnixListener {
    inner: UnixListener,
    server_uid: u32,
}

impl SameUidUnixListener {
    pub(crate) fn new(inner: UnixListener) -> Self {
        Self {
            inner,
            server_uid: nix::unistd::geteuid().as_raw(),
        }
    }

    async fn accept_authorized(&mut self) -> (UnixStream, SocketAddr) {
        loop {
            let (stream, addr) = match self.inner.accept().await {
                Ok(connection) => connection,
                Err(error) => {
                    log::error!("local control socket accept failed: {error}");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };

            match stream.peer_cred() {
                Ok(credentials) if same_uid(credentials.uid(), self.server_uid) => {
                    return (stream, addr);
                }
                Ok(credentials) => {
                    log::warn!(
                        "rejected local control connection: peer uid {} does not match server uid {} (peer pid {:?})",
                        credentials.uid(),
                        self.server_uid,
                        credentials.pid(),
                    );
                }
                Err(error) => {
                    // Fail closed: an unverifiable local client must not reach
                    // query, extension, REPL, or code-execution routes.
                    log::warn!(
                        "rejected local control connection: cannot read peer credentials: {error}"
                    );
                }
            }
        }
    }
}

impl Listener for SameUidUnixListener {
    type Io = UnixStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        self.accept_authorized().await
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_uid_mismatch() {
        assert!(!same_uid(1001, 1000));
    }

    #[tokio::test]
    async fn recognizes_current_process_peer_credentials() {
        let (server, _client) = UnixStream::pair().expect("create Unix socket pair");
        let credentials = server.peer_cred().expect("peer credentials");
        assert!(same_uid(credentials.uid(), nix::unistd::geteuid().as_raw()));
    }

    #[tokio::test]
    async fn accepts_connection_from_same_uid() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("probing.sock");
        let listener = match UnixListener::bind(&path) {
            Ok(listener) => listener,
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                eprintln!("skipping Unix peer-credential test: socket bind denied ({error})");
                return;
            }
            Err(error) => panic!("bind unix socket: {error}"),
        };
        let mut listener = SameUidUnixListener::new(listener);

        let (accepted, connected) =
            tokio::join!(listener.accept_authorized(), UnixStream::connect(&path));
        let client = connected.expect("connect unix socket");

        let credentials = accepted.0.peer_cred().expect("peer credentials");
        assert_eq!(credentials.uid(), nix::unistd::geteuid().as_raw());
        drop(client);
    }
}
