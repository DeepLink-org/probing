use dioxus::prelude::*;
use crate::components::common::{LoadingState, ErrorState};
use crate::components::page::{PageContainer, PageTitle};
use crate::hooks::use_api_simple;
use crate::api::{ApiClient, ProfileResponse};
use crate::app::{PROFILING_VIEW, PROFILING_PPROF_FREQ, PROFILING_TORCH_ENABLED, 
    PROFILING_CHROME_DATA_SOURCE, PROFILING_CHROME_LIMIT, PROFILING_PYTORCH_STEPS};

/// 从配置中更新全局状态
fn apply_config(config: &[(String, String)]) {
    *PROFILING_PPROF_FREQ.write() = 0;
    *PROFILING_TORCH_ENABLED.write() = false;
    
    for (name, value) in config {
        match name.as_str() {
            "probing.pprof.sample_freq" => {
                if let Ok(v) = value.parse::<i32>() {
                    *PROFILING_PPROF_FREQ.write() = v.max(0);
                }
            },
            "probing.torch.profiling" => {
                let lowered = value.trim().to_lowercase();
                let enabled = !lowered.is_empty()
                    && lowered != "0"
                    && lowered != "false"
                    && lowered != "off"
                    && lowered != "disable"
                    && lowered != "disabled";
                *PROFILING_TORCH_ENABLED.write() = enabled;
            },
            _ => {}
        }
    }
}

#[component]
pub fn Profiling() -> Element {
    // 使用全局状态
    let chrome_iframe_key = use_signal(|| 0);
    
    let config_state = use_api_simple::<Vec<(String, String)>>();
    let flamegraph_state = use_api_simple::<String>();
    let chrome_tracing_state = use_api_simple::<String>();
    let pytorch_profile_state = use_api_simple::<ProfileResponse>();
    
    // 加载配置
    use_effect(move || {
        let mut loading = config_state.loading;
        let mut data = config_state.data;
        spawn(async move {
            *loading.write() = true;
            let client = ApiClient::new();
            let result = client.get_profiler_config().await;
            match result {
                Ok(ref config) => {
                    apply_config(config);
                }
                Err(_) => {}
            }
            *data.write() = Some(result);
            *loading.write() = false;
        });
    });

    // 当视图切换时，重新加载配置
    use_effect(move || {
        let view = PROFILING_VIEW.read().clone();
        drop(view);
        spawn(async move {
            let client = ApiClient::new();
            if let Ok(config) = client.get_profiler_config().await {
                apply_config(&config);
            }
        });
    });

    // 加载 flamegraph（pprof 或 torch）
    use_effect(move || {
        let view = PROFILING_VIEW.read().clone();
        let pprof_on = *PROFILING_PPROF_FREQ.read() > 0;
        let torch = *PROFILING_TORCH_ENABLED.read();
        
        // 只在选择 pprof 或 torch 视图时加载
        if view != "pprof" && view != "torch" {
            return;
        }
        
        let active_profiler = match (view.as_str(), pprof_on, torch) {
            ("pprof", true, _) => "pprof",
            ("torch", _, true) => "torch",
            _ => return,
        };
        
        let mut loading = flamegraph_state.loading;
        let mut data = flamegraph_state.data;
        spawn(async move {
            *loading.write() = true;
            let client = ApiClient::new();
            let result = client.get_flamegraph(active_profiler).await;
            *data.write() = Some(result);
            *loading.write() = false;
        });
    });

    // 加载 Chrome Tracing 数据
    use_effect(move || {
        let view = PROFILING_VIEW.read().clone();
        let source = PROFILING_CHROME_DATA_SOURCE.read().clone();
        let limit_val = *PROFILING_CHROME_LIMIT.read();
        
        // 只在选择 chrome-tracing 视图时加载
        if view != "chrome-tracing" {
            return;
        }
        
        if source == "trace" {
            let mut loading = chrome_tracing_state.loading;
            let mut data = chrome_tracing_state.data;
            let mut iframe_key = chrome_iframe_key.clone();
            spawn(async move {
                *loading.write() = true;
                let client = ApiClient::new();
                let result = client.get_chrome_tracing_json(Some(limit_val)).await;
                *data.write() = Some(result);
                *loading.write() = false;
                *iframe_key.write() += 1;
            });
        }
    });

    rsx! {
        PageContainer {
            // 动态页面标题 - 根据当前视图显示
            {
                let current_view = PROFILING_VIEW.read();
                let (title, subtitle, icon) = match current_view.as_str() {
                    "pprof" => (
                        "pprof Flamegraph".to_string(),
                        Some("CPU profiling with pprof".to_string()),
                        Some(&icondata::CgPerformance),
                    ),
                    "torch" => (
                        "torch Flamegraph".to_string(),
                        Some("PyTorch profiling visualization".to_string()),
                        Some(&icondata::SiPytorch),
                    ),
                    "chrome-tracing" => (
                        "Timeline".to_string(),
                        Some("Chrome Tracing timeline view".to_string()),
                        Some(&icondata::AiThunderboltOutlined),
                    ),
                    _ => (
                        "Profiling".to_string(),
                        Some("Performance profiling and analysis".to_string()),
                        Some(&icondata::AiSearchOutlined),
                    ),
                };
                rsx! {
                    PageTitle {
                        title,
                        subtitle,
                        icon,
                    }
                }
            }
            
            // Profiling 内容区域 - 使用 Card 样式统一布局
            div {
                class: "bg-white rounded-lg shadow-sm border border-gray-200 relative",
                style: "min-height: calc(100vh - 12rem);",
                // pprof 或 torch flamegraph
                if *PROFILING_VIEW.read() == "pprof" || *PROFILING_VIEW.read() == "torch" {
                    if !(*PROFILING_PPROF_FREQ.read() > 0) && !*PROFILING_TORCH_ENABLED.read() {
                        EmptyState {
                            message: format!("No profilers are currently enabled. Enable {} using the controls in the sidebar.", 
                                if *PROFILING_VIEW.read() == "pprof" { "pprof" } else { "torch" })
                        }
                    } else if flamegraph_state.is_loading() {
                        LoadingState { message: Some("Loading flamegraph...".to_string()) }
                    } else if let Some(Ok(flamegraph)) = flamegraph_state.data.read().as_ref() {
                        div {
                            class: "absolute inset-0 w-full h-full",
                            div {
                                class: "w-full h-full",
                                dangerous_inner_html: "{flamegraph}"
                            }
                        }
                    } else if let Some(Err(err)) = flamegraph_state.data.read().as_ref() {
                        ErrorState {
                            error: format!("Failed to load flamegraph: {:?}", err),
                            title: Some("Error Loading Flamegraph".to_string())
                        }
                    }
                }
                
                // Chrome Tracing 视图
                if *PROFILING_VIEW.read() == "chrome-tracing" {
                    if chrome_tracing_state.is_loading() {
                        LoadingState { 
                            message: Some(if *PROFILING_CHROME_DATA_SOURCE.read() == "pytorch" {
                                "Loading PyTorch timeline data...".to_string()
                            } else {
                                "Loading trace data...".to_string()
                            })
                        }
                    } else if let Some(Ok(ref trace_json)) = chrome_tracing_state.data.read().as_ref() {
                        if trace_json.trim().is_empty() {
                            ErrorState { 
                                error: "Timeline data is empty. Make sure the profiler has been executed.".to_string(), 
                                title: Some("Empty Timeline Data".to_string())
                            }
                        } else if let Err(e) = serde_json::from_str::<serde_json::Value>(trace_json) {
                            ErrorState { 
                                error: format!("Invalid JSON data: {:?}", e), 
                                title: Some("Invalid Timeline Data".to_string())
                            }
                        } else {
                            div {
                                class: "absolute inset-0 overflow-hidden",
                                style: "min-height: 600px;",
                                iframe {
                                    key: "{*chrome_iframe_key.read()}",
                                    srcdoc: get_tracing_viewer_html(trace_json),
                                    style: "width: 100%; height: 100%; border: none;",
                                    title: "Chrome Tracing Viewer"
                                }
                            }
                        }
                    } else if let Some(Err(ref err)) = chrome_tracing_state.data.read().as_ref() {
                        ErrorState { 
                            error: format!("Failed to load timeline: {:?}", err), 
                            title: Some("Load Timeline Error".to_string())
                        }
                    } else {
                        div {
                            class: "absolute inset-0 flex items-center justify-center p-8",
                            div {
                                class: "text-center text-gray-500",
                                if *PROFILING_CHROME_DATA_SOURCE.read() == "pytorch" {
                                    p {
                                        class: "mb-4 text-lg",
                                        "PyTorch Profiler Timeline"
                                    }
                                    p {
                                        class: "text-sm",
                                        "Click 'Start Profile' to begin profiling, then click 'Load Timeline' to view the results."
                                    }
                                } else {
                                    p {
                                        class: "mb-4 text-lg",
                                        "Trace Events Timeline"
                                    }
                                    p {
                                        class: "text-sm",
                                        "Select the number of events and the timeline will load automatically."
                                    }
                                }
                            }
                        }
                    }
                }
            }
            
            // PyTorch Profile 状态显示
            if *PROFILING_VIEW.read() == "chrome-tracing" && *PROFILING_CHROME_DATA_SOURCE.read() == "pytorch" {
                if let Some(Ok(ref profile_result)) = pytorch_profile_state.data.read().as_ref() {
                    if profile_result.success {
                        div {
                            class: "p-3 bg-green-50 border border-green-200 rounded text-sm text-green-800",
                            if let Some(ref msg) = profile_result.message {
                                "{msg}"
                            } else {
                                "Profile started successfully"
                            }
                        }
                    } else {
                        div {
                            class: "p-3 bg-red-50 border border-red-200 rounded text-sm text-red-800",
                            if let Some(ref err) = profile_result.error {
                                "{err}"
                            } else {
                                "Failed to start profile"
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn EmptyState(message: String) -> Element {
    rsx! {
        div {
            class: "absolute inset-0 flex items-center justify-center",
            div {
                class: "text-center",
                h2 { class: "text-2xl font-bold text-gray-900 mb-4", "No Profilers Enabled" }
                p { class: "text-gray-600 mb-6", "{message}" }
            }
        }
    }
}

/// 生成包含 Chrome tracing viewer 的 HTML 页面
/// 直接使用已加载的 trace JSON 数据，通过 postMessage API 传递给 Perfetto UI
fn get_tracing_viewer_html(trace_json: &str) -> String {
    // 转义 JSON 数据以便嵌入到 JavaScript 中
    let escaped_json = trace_json
        .replace('\\', "\\\\")
        .replace('`', "\\`")
        .replace('$', "\\$");
    
    format!(r#"
<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>Chrome Tracing Viewer</title>
    <style>
        body {{
            margin: 0;
            padding: 0;
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            overflow: hidden;
        }}
        #perfetto-iframe {{
            width: 100%;
            height: 100vh;
            border: none;
        }}
        .loading {{
            display: flex;
            align-items: center;
            justify-content: center;
            height: 100vh;
            font-size: 18px;
            color: #666;
        }}
    </style>
</head>
<body>
    <div id="loading" class="loading">Loading Chrome Tracing Viewer...</div>
    <iframe id="perfetto-iframe" style="display: none;"></iframe>
    <script>
        (function() {{
            try {{
                // 解析已加载的 trace 数据
                const traceData = JSON.parse(`{escaped_json}`);
                
                const iframe = document.getElementById('perfetto-iframe');
                const loading = document.getElementById('loading');
                
                // 使用 Perfetto UI 的 postMessage API 传递 trace 数据
                const perfettoUrl = 'https://ui.perfetto.dev/#!/';
                iframe.src = perfettoUrl;
                
                let loaded = false;
                let errorShown = false;
                
                // 监听来自 Perfetto UI 的消息
                const messageHandler = function(event) {{
                    if (event.origin === 'https://ui.perfetto.dev') {{
                        if (event.data) {{
                            const dataStr = typeof event.data === 'string' ? event.data : JSON.stringify(event.data);
                            if (dataStr.includes('error') || dataStr.includes('Failed')) {{
                                console.error('Perfetto UI error:', event.data);
                                if (!loaded && !errorShown) {{
                                    errorShown = true;
                                    showError('Perfetto UI reported an error. Please check the trace data format.');
                                    window.removeEventListener('message', messageHandler);
                                }}
                            }} else if (dataStr.includes('loaded') || dataStr.includes('ready')) {{
                                if (!loaded) {{
                                    loaded = true;
                                    loading.style.display = 'none';
                                    iframe.style.display = 'block';
                                    window.removeEventListener('message', messageHandler);
                                }}
                            }}
                        }}
                    }}
                }};
                window.addEventListener('message', messageHandler);
                
                iframe.onload = function() {{
                    // Perfetto UI 页面加载完成，等待 PING/PONG handshake
                    let handshakeComplete = false;
                    let retryCount = 0;
                    const maxRetries = 10;
                    
                    // 监听来自 Perfetto UI 的 PONG 消息
                    const handshakeHandler = function(event) {{
                        if (event.origin === 'https://ui.perfetto.dev' || 
                            (event.source === iframe.contentWindow && event.data === 'PONG')) {{
                            if (event.data && event.data === 'PONG') {{
                                handshakeComplete = true;
                                window.removeEventListener('message', handshakeHandler);
                                
                                // Handshake 完成，发送 trace 数据
                                try {{
                                    // 将 trace 数据转换为 ArrayBuffer
                                    const traceJson = JSON.stringify(traceData, null, 2);
                                    const encoder = new TextEncoder();
                                    const buffer = encoder.encode(traceJson).buffer;
                                    
                                    // 发送 trace 数据到 Perfetto UI
                                    iframe.contentWindow.postMessage({{
                                        perfetto: {{
                                            buffer: buffer,
                                            title: 'Chrome Tracing Timeline',
                                            fileName: 'trace.json',
                                        }}
                                    }}, 'https://ui.perfetto.dev');
                                    
                                    console.log('Trace data sent to Perfetto UI');
                                    
                                    // 等待一下，然后隐藏 loading
                                    setTimeout(() => {{
                                        if (!loaded && !errorShown) {{
                                            loaded = true;
                                            loading.style.display = 'none';
                                            iframe.style.display = 'block';
                                            window.removeEventListener('message', messageHandler);
                                        }}
                                    }}, 2000);
                                }} catch (e) {{
                                    console.error('Error sending trace data:', e);
                                    if (!errorShown) {{
                                        errorShown = true;
                                        showError('Failed to send trace data to Perfetto UI: ' + e.message);
                                        window.removeEventListener('message', messageHandler);
                                    }}
                                }}
                            }}
                        }}
                    }};
                    window.addEventListener('message', handshakeHandler);
                    
                    // 发送 PING 消息启动 handshake
                    const sendPing = function() {{
                        if (!handshakeComplete && retryCount < maxRetries) {{
                            try {{
                                if (iframe.contentWindow) {{
                                    iframe.contentWindow.postMessage('PING', 'https://ui.perfetto.dev');
                                    retryCount++;
                                    if (retryCount < maxRetries) {{
                                        setTimeout(sendPing, 500);
                                    }} else {{
                                        console.warn('PING/PONG handshake failed, trying data URL fallback');
                                        // 回退到 data URL 方式
                                        const traceJson = JSON.stringify(traceData, null, 2);
                                        const base64Data = btoa(unescape(encodeURIComponent(traceJson)));
                                        const dataUrl = 'data:application/json;base64,' + base64Data;
                                        iframe.src = 'https://ui.perfetto.dev/#!/?url=' + encodeURIComponent(dataUrl);
                                        window.removeEventListener('message', handshakeHandler);
                                    }}
                                }} else {{
                                    if (retryCount < maxRetries) {{
                                        retryCount++;
                                        setTimeout(sendPing, 500);
                                    }}
                                }}
                            }} catch (e) {{
                                console.error('Error sending PING:', e);
                                if (retryCount < maxRetries) {{
                                    retryCount++;
                                    setTimeout(sendPing, 500);
                                }}
                            }}
                        }}
                    }};
                    
                    // 等待 iframe 完全加载后发送 PING
                    setTimeout(sendPing, 1500);
                    
                    // 超时处理
                    setTimeout(() => {{
                        if (!loaded && !errorShown) {{
                            loaded = true;
                            loading.style.display = 'none';
                            iframe.style.display = 'block';
                            window.removeEventListener('message', messageHandler);
                            window.removeEventListener('message', handshakeHandler);
                        }}
                    }}, 10000);
                }};
                
                iframe.onerror = function() {{
                    if (!loaded && !errorShown) {{
                        errorShown = true;
                        showError('Failed to load Perfetto UI');
                    }}
                }};
                
                function showError(message) {{
                    loading.innerHTML = `
                        <div style="padding: 20px; text-align: center;">
                            <h2>${{message}}</h2>
                            <p>You can view this trace in Chrome DevTools:</p>
                            <ol style="text-align: left; display: inline-block;">
                                <li>Open Chrome and navigate to <code>chrome://tracing</code></li>
                                <li>Click "Load" and select the trace file</li>
                            </ol>
                            <br>
                            <button onclick="window.location.reload()" style="padding: 10px 20px; background: #4285f4; color: white; border: none; border-radius: 4px; cursor: pointer; margin: 10px 0;">
                                Retry
                            </button>
                        </div>
                    `;
                }}
            }} catch (e) {{
                document.getElementById('loading').innerHTML = `
                    <div style="padding: 20px; color: red; text-align: center;">
                        <h2>Error loading trace viewer</h2>
                        <p>${{e.message}}</p>
                    </div>
                `;
            }}
        }})();
    </script>
</body>
</html>
    "#)
}