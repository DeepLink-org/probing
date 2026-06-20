use axum::extract::ws::Message;
use futures_util::{SinkExt, StreamExt};

pub async fn ws_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
) -> impl axum::response::IntoResponse {
    ws.on_upgrade(|ws| async move {
        log::info!("WebSocket connection established");
        let (mut write, mut read) = ws.split();
        let mut session = probing_python::repl::ReplSession::new();

        while let Some(Ok(msg)) = read.next().await {
            if let Message::Text(msg) = msg {
                let rsp = session.handle_text(msg.to_string());
                if write.send(Message::Text(rsp.into())).await.is_err() {
                    break;
                }
            }
        }
    })
}
