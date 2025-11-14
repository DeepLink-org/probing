use dioxus::prelude::*;
use crate::components::common::{LoadingState, ErrorState, EmptyState};
use crate::components::page::{PageContainer, PageTitle};
use crate::hooks::use_api_simple;
use crate::api::{ApiClient, ProfileResponse};
use crate::app::{PROFILING_VIEW, PROFILING_PPROF_FREQ, PROFILING_TORCH_ENABLED, 
    PROFILING_CHROME_LIMIT, PROFILING_PYTORCH_TIMELINE_RELOAD};

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
                let disabled_values = ["", "0", "false", "off", "disable", "disabled"];
                let enabled = !disabled_values.contains(&lowered.as_str());
                *PROFILING_TORCH_ENABLED.write() = enabled;
            },
            _ => {}
        }
    }
}

#[component]
pub fn Profiling() -> Element {
    let chrome_iframe_key = use_signal(|| 0);
    
    let config_state = use_api_simple::<Vec<(String, String)>>();
    let flamegraph_state = use_api_simple::<String>();
    let chrome_tracing_state = use_api_simple::<String>();
    let pytorch_profile_state = use_api_simple::<ProfileResponse>();
    
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

    use_effect(move || {
        let view = PROFILING_VIEW.read().clone();
        let pprof_on = *PROFILING_PPROF_FREQ.read() > 0;
        let torch = *PROFILING_TORCH_ENABLED.read();
        
        let active_profiler = match view.as_str() {
            "pprof" if pprof_on => "pprof",
            "torch" if torch => "torch",
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

    // 处理 trace timeline 的自动加载
    use_effect(move || {
        let view = PROFILING_VIEW.read().clone();
        let limit_val = *PROFILING_CHROME_LIMIT.read();
        
        if view != "trace-timeline" {
            return;
        }
        
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
    });
    
    // 处理 pytorch timeline 的加载（通过侧边栏按钮触发）
    use_effect(move || {
        let view = PROFILING_VIEW.read().clone();
        let reload_key = *PROFILING_PYTORCH_TIMELINE_RELOAD.read();
        
        if view != "pytorch-timeline" || reload_key == 0 {
            return;
        }
        
        let mut loading = chrome_tracing_state.loading;
        let mut data = chrome_tracing_state.data;
        let mut iframe_key = chrome_iframe_key.clone();
        spawn(async move {
            *loading.write() = true;
            let client = ApiClient::new();
            let result = client.get_pytorch_timeline().await;
            *data.write() = Some(result);
            *loading.write() = false;
            *iframe_key.write() += 1;
        });
    });

    rsx! {
        PageContainer {
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
                    "trace-timeline" => (
                        "Trace Timeline".to_string(),
                        Some("Chrome Tracing timeline view".to_string()),
                        Some(&icondata::AiThunderboltOutlined),
                    ),
                    "pytorch-timeline" => (
                        "PyTorch Timeline".to_string(),
                        Some("PyTorch profiler timeline view".to_string()),
                        Some(&icondata::SiPytorch),
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
            
            div {
                class: "bg-white rounded-lg shadow-sm border border-gray-200 relative",
                style: "min-height: calc(100vh - 12rem);",
                {
                    let current_view = PROFILING_VIEW.read().clone();
                    if current_view == "pprof" || current_view == "torch" {
                        rsx! {
                            FlamegraphView {
                                flamegraph_state: flamegraph_state.clone(),
                            }
                        }
                    } else if current_view == "trace-timeline" || current_view == "pytorch-timeline" {
                        rsx! {
                            ChromeTracingView {
                                chrome_tracing_state: chrome_tracing_state.clone(),
                                chrome_iframe_key: chrome_iframe_key.clone(),
                            }
                        }
                    } else {
                        rsx! { div {} }
                    }
                }
            }
            
        }
    }
}


/// 生成包含 Chrome tracing viewer 的 HTML 页面
fn get_tracing_viewer_html(trace_json: &str) -> String {
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
                const traceData = JSON.parse(`{escaped_json}`);
                
                const iframe = document.getElementById('perfetto-iframe');
                const loading = document.getElementById('loading');
                
                const perfettoUrl = 'https://ui.perfetto.dev/#!/';
                iframe.src = perfettoUrl;
                
                let loaded = false;
                let errorShown = false;
                
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
                    let handshakeComplete = false;
                    let retryCount = 0;
                    const maxRetries = 10;
                    
                    const handshakeHandler = function(event) {{
                        if (event.origin === 'https://ui.perfetto.dev' || 
                            (event.source === iframe.contentWindow && event.data === 'PONG')) {{
                            if (event.data && event.data === 'PONG') {{
                                handshakeComplete = true;
                                window.removeEventListener('message', handshakeHandler);
                                
                                try {{
                                    const traceJson = JSON.stringify(traceData, null, 2);
                                    const encoder = new TextEncoder();
                                    const buffer = encoder.encode(traceJson).buffer;
                                    
                                    iframe.contentWindow.postMessage({{
                                        perfetto: {{
                                            buffer: buffer,
                                            title: 'Chrome Tracing Timeline',
                                            fileName: 'trace.json',
                                        }}
                                    }}, 'https://ui.perfetto.dev');
                                    
                                    console.log('Trace data sent to Perfetto UI');
                                    
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
                    
                    setTimeout(sendPing, 1500);
                    
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

#[component]
fn FlamegraphView(
    #[props] flamegraph_state: crate::hooks::ApiState<String>
) -> Element {
    let pprof_enabled = *PROFILING_PPROF_FREQ.read() > 0;
    let torch_enabled = *PROFILING_TORCH_ENABLED.read();
    let current_view = PROFILING_VIEW.read().clone();
    let profiler_name = if current_view == "pprof" { "pprof" } else { "torch" };
    
    if !pprof_enabled && !torch_enabled {
        let message = format!(
            "No profilers are currently enabled. Enable {} using the controls in the sidebar.", 
            profiler_name
        );
        return rsx! {
            div {
                class: "absolute inset-0 flex items-center justify-center",
                div {
                    class: "text-center",
                    h2 { class: "text-2xl font-bold text-gray-900 mb-4", "No Profilers Enabled" }
                    EmptyState { message }
                }
            }
        };
    }
    
    if flamegraph_state.is_loading() {
        return rsx! {
            LoadingState { message: Some("Loading flamegraph...".to_string()) }
        };
    }
    
    if let Some(Ok(flamegraph)) = flamegraph_state.data.read().as_ref() {
        return rsx! {
            div {
                class: "absolute inset-0 w-full h-full",
                div {
                    class: "w-full h-full",
                    dangerous_inner_html: "{flamegraph}"
                }
            }
        };
    }
    
    if let Some(Err(err)) = flamegraph_state.data.read().as_ref() {
        return rsx! {
            ErrorState {
                error: format!("Failed to load flamegraph: {:?}", err),
                title: Some("Error Loading Flamegraph".to_string())
            }
        };
    }
    
    rsx! { div {} }
}

#[component]
fn ChromeTracingView(
    #[props] chrome_tracing_state: crate::hooks::ApiState<String>,
    #[props] chrome_iframe_key: Signal<i32>,
) -> Element {
    let current_view = PROFILING_VIEW.read().clone();
    let is_pytorch = current_view == "pytorch-timeline";
    
    if chrome_tracing_state.is_loading() {
        let message = if is_pytorch {
            "Loading PyTorch timeline data..."
        } else {
            "Loading trace data..."
        };
        return rsx! {
            LoadingState { message: Some(message.to_string()) }
        };
    }
    
    if let Some(Ok(ref trace_json)) = chrome_tracing_state.data.read().as_ref() {
        if trace_json.trim().is_empty() {
            return rsx! {
                ErrorState { 
                    error: "Timeline data is empty. Make sure the profiler has been executed.".to_string(), 
                    title: Some("Empty Timeline Data".to_string())
                }
            };
        }
        
        if let Err(e) = serde_json::from_str::<serde_json::Value>(trace_json) {
            return rsx! {
                ErrorState { 
                    error: format!("Invalid JSON data: {:?}", e), 
                    title: Some("Invalid Timeline Data".to_string())
                }
            };
        }
        
        return rsx! {
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
        };
    }
    
    if let Some(Err(ref err)) = chrome_tracing_state.data.read().as_ref() {
        return rsx! {
            ErrorState { 
                error: format!("Failed to load timeline: {:?}", err), 
                title: Some("Load Timeline Error".to_string())
            }
        };
    }
    
    let (title, description) = if is_pytorch {
        ("PyTorch Profiler Timeline", 
         "Click 'Start Profile' to begin profiling, then click 'Load Timeline' to view the results.")
    } else {
        ("Trace Events Timeline",
         "Select the number of events and the timeline will load automatically.")
    };
    
    rsx! {
        div {
            class: "absolute inset-0 flex items-center justify-center p-8",
            div {
                class: "text-center text-gray-500",
                p {
                    class: "mb-4 text-lg",
                    "{title}"
                }
                p {
                    class: "text-sm",
                    "{description}"
                }
            }
        }
    }
}

#[component]
fn PyTorchProfileStatus(#[props] profile_result: ProfileResponse) -> Element {
    if profile_result.success {
        let message = profile_result.message.as_deref().unwrap_or("Profile started successfully");
        rsx! {
            div {
                class: "p-3 bg-green-50 border border-green-200 rounded text-sm text-green-800",
                "{message}"
            }
        }
    } else {
        let error = profile_result.error.as_deref().unwrap_or("Failed to start profile");
        rsx! {
            div {
                class: "p-3 bg-red-50 border border-red-200 rounded text-sm text-red-800",
                "{error}"
            }
        }
    }
}