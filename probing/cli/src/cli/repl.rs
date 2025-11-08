use anyhow::Result;
use futures_util::sink::Sink;
use futures_util::stream::Stream;
use futures_util::{SinkExt, StreamExt};
use reedline::{DefaultPrompt, Reedline, Signal};
use serde_json::Value;
use std::io::Write;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::UnixStream;
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::{client_async, connect_async};
use tokio_tungstenite::{tungstenite::Message as WsMessage, WebSocketStream as WsStream};

use super::ctrl::ProbeEndpoint;

pub async fn start_repl(ctrl: ProbeEndpoint) -> Result<()> {
    println!("Connecting to REPL server...");
    println!("Type 'exit' or press Ctrl+D to exit");
    println!();

    // 连接 WebSocket
    let mut ws = connect_websocket(&ctrl).await?;

    // 创建 Reedline 实例（使用 Arc<Mutex> 以便在异步环境中共享）
    let line_editor = Arc::new(Mutex::new(Reedline::create()));
    // 使用自定义提示符 ">>> "
    use reedline::DefaultPromptSegment;
    let prompt = DefaultPrompt {
        left_prompt: DefaultPromptSegment::Basic(">>> ".to_string()),
        right_prompt: DefaultPromptSegment::Empty,
    };

    // REPL 循环 - 串行处理：输入 -> 发送 -> 等待响应 -> 输出 -> 下一轮
    loop {
        // 在阻塞任务中读取用户输入（这会阻塞直到用户按下回车）
        let editor = line_editor.clone();
        let prompt = prompt.clone();
        let sig = tokio::task::spawn_blocking(move || {
            let mut editor = editor.lock()
                .unwrap_or_else(|e| {
                    eprintln!("Failed to acquire lock on line editor (lock poisoned): {e}");
                    panic!("Lock poisoned: {e}")
                });
            editor.read_line(&prompt)
        })
        .await
        .map_err(|e| anyhow::anyhow!("Error reading input task: {}", e))?;

        match sig {
            Ok(Signal::Success(line)) => {
                let trimmed = line.trim();
                // 空输入时跳过，继续循环
                if trimmed.is_empty() {
                    continue;
                }
                // 只处理 "exit" 命令时退出
                if trimmed == "exit" {
                    break;
                }

                // 发送代码到服务器
                let msg = format!("{}\n", line);
                if let Err(e) = ws.write.as_mut().send(WsMessage::Text(msg.into())).await {
                    eprintln!("\nSend error: {}", e);
                    break;
                }

                // 等待并接收服务器响应（这会阻塞直到响应到达）
                match ws.read.as_mut().next().await {
                    Some(Ok(WsMessage::Text(response))) => {
                        // 解析 JSON 响应
                        match serde_json::from_str::<Value>(&response) {
                            Ok(json) => {
                                // 显示输出
                                if let Some(output) = json.get("output").and_then(|v| v.as_str()) {
                                    if !output.is_empty() {
                                        print!("{}", output);
                                        // 如果输出不以换行结尾，添加换行
                                        if !output.ends_with('\n') {
                                            print!("\n");
                                        }
                                    }
                                }

                                // 显示错误堆栈
                                if let Some(traceback) =
                                    json.get("traceback").and_then(|v| v.as_array())
                                {
                                    if !traceback.is_empty() {
                                        for line in traceback {
                                            if let Some(line_str) = line.as_str() {
                                                eprintln!("{}", line_str);
                                            }
                                        }
                                    }
                                }

                                // 刷新输出
                                std::io::stdout().flush().unwrap();
                                std::io::stderr().flush().unwrap();
                            }
                            Err(_) => {
                                // 如果不是 JSON，直接显示原始响应
                                print!("{}", response);
                                if !response.ends_with('\n') {
                                    print!("\n");
                                }
                                std::io::stdout().flush().unwrap();
                            }
                        }
                    }
                    Some(Ok(WsMessage::Close(_))) => {
                        println!("\nConnection closed");
                        break;
                    }
                    Some(Err(e)) => {
                        eprintln!("\nReceive error: {}", e);
                        break;
                    }
                    None => {
                        println!("\nConnection disconnected");
                        break;
                    }
                    _ => {}
                }
            }
            Ok(Signal::CtrlC) => {
                eprintln!("\nPress Ctrl+C to cancel");
                // 继续循环，不退出
            }
            Ok(Signal::CtrlD) => {
                break;
            }
            Err(err) => {
                eprintln!("\nRead error: {:?}", err);
                break;
            }
        }
    }

    println!("\nExiting...");
    Ok(())
}

async fn connect_websocket(ctrl: &ProbeEndpoint) -> Result<WsConnection> {
    match ctrl {
        ProbeEndpoint::Local { pid } => connect_unix_websocket(*pid).await,
        ProbeEndpoint::Remote { addr } => connect_tcp_websocket(addr).await,
        _ => anyhow::bail!("Unsupported endpoint type for REPL"),
    }
}

async fn connect_tcp_websocket(addr: &str) -> Result<WsConnection> {
    let url = format!("ws://{}/ws", addr);
    let (ws_stream, _) = connect_async(&url)
        .await
        .map_err(|e| anyhow::anyhow!("WebSocket connection failed: {}", e))?;

    Ok(boxed_connection(ws_stream))
}

async fn connect_unix_websocket(pid: i32) -> Result<WsConnection> {
    #[cfg(target_os = "linux")]
    let path = format!("\0probing-{}", pid);
    #[cfg(not(target_os = "linux"))]
    let path = {
        let temp_dir = std::env::temp_dir();
        temp_dir.join(format!("probing-{}.sock", pid))
    };

    let stream: UnixStream = {
        #[cfg(target_os = "linux")]
        {
            UnixStream::connect(path.as_str()).await?
        }
        #[cfg(not(target_os = "linux"))]
        {
            UnixStream::connect(&path).await?
        }
    };

    let (ws_stream, _) = client_async("ws://localhost/ws", stream)
        .await
        .map_err(|e| anyhow::anyhow!("WebSocket connection failed: {}", e))?;

    Ok(boxed_connection(ws_stream))
}

type DynWsSink = Pin<Box<dyn Sink<WsMessage, Error = WsError> + Send>>;
type DynWsStream = Pin<Box<dyn Stream<Item = Result<WsMessage, WsError>> + Send>>;

struct WsConnection {
    write: DynWsSink,
    read: DynWsStream,
}

fn boxed_connection<S>(ws_stream: WsStream<S>) -> WsConnection
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (write, read) = ws_stream.split();
    WsConnection {
        write: Box::pin(write),
        read: Box::pin(read),
    }
}
