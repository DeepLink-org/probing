use dioxus::prelude::*;
use crate::components::page::{PageContainer, PageTitle};
use crate::components::common::{LoadingState, ErrorState};
use crate::hooks::use_api_simple;
use crate::api::{ApiClient, ProfileResponse};

#[component]
pub fn ChromeTracing() -> Element {
    let mut data_source = use_signal(|| "trace".to_string()); // "trace" or "pytorch"
    let limit = use_signal(|| 1000usize);
    let pytorch_steps = use_signal(|| 5i32);
    let state = use_api_simple::<String>();
    let profile_state = use_api_simple::<ProfileResponse>();
    let mut iframe_key = use_signal(|| 0);
    
    // 创建依赖项，当limit改变时重新计算
    let limit_value = use_memo({
        let limit = limit.clone();
        move || *limit.read()
    });
    
    // 创建数据源依赖项
    let data_source_value = use_memo({
        let data_source = data_source.clone();
        move || data_source.read().clone()
    });
    
    // 当数据源或limit改变时重新获取数据（仅对 trace 数据源）
    use_effect({
        let data_source_value = data_source_value.clone();
        let limit_value = limit_value.clone();
        let mut loading = state.loading;
        let mut data = state.data;
        let mut iframe_key = iframe_key.clone();
        move || {
            let source = data_source_value.read().clone();
            let limit_val = *limit_value.read();
            if source == "trace" {
                spawn(async move {
                    *loading.write() = true;
                    let client = ApiClient::new();
                    let result = client.get_chrome_tracing_json(Some(limit_val)).await;
                    *data.write() = Some(result);
                    *loading.write() = false;
                    // 更新 iframe key 以强制重新加载
                    *iframe_key.write() += 1;
                });
            }
        }
    });

    rsx! {
        PageContainer {
            PageTitle {
                title: "Chrome Tracing".to_string(),
                subtitle: Some("View timeline in Chrome DevTools tracing format".to_string()),
                icon: Some(&icondata::AiThunderboltOutlined),
            }
            
            // Data source selector
            div {
                class: "mb-4 p-4 bg-white rounded-lg shadow",
                div {
                    class: "flex items-center space-x-4 mb-4",
                    span {
                        class: "text-sm font-medium text-gray-700",
                        "Data Source:"
                    }
                    button {
                        class: if *data_source.read() == "trace" {
                            "px-4 py-2 text-sm font-medium rounded-md bg-blue-600 text-white"
                        } else {
                            "px-4 py-2 text-sm font-medium rounded-md bg-gray-200 text-gray-700 hover:bg-gray-300"
                        },
                        onclick: move |_| *data_source.write() = "trace".to_string(),
                        "Trace Events"
                    }
                    button {
                        class: if *data_source.read() == "pytorch" {
                            "px-4 py-2 text-sm font-medium rounded-md bg-blue-600 text-white"
                        } else {
                            "px-4 py-2 text-sm font-medium rounded-md bg-gray-200 text-gray-700 hover:bg-gray-300"
                        },
                        onclick: move |_| *data_source.write() = "pytorch".to_string(),
                        "PyTorch Profiler"
                    }
                }
                
                // Trace Events controls
                if *data_source.read() == "trace" {
                    div {
                        class: "space-y-2",
                        div {
                            class: "flex items-center justify-between",
                            span {
                                class: "text-sm text-gray-600",
                                "Number of Events"
                            }
                            span {
                                class: "text-sm text-gray-800 font-mono",
                                "{*limit.read()} events"
                            }
                        }
                        input {
                            r#type: "range",
                            min: "100",
                            max: "5000",
                            step: "100",
                            value: "{*limit.read()}",
                            class: "w-full",
                            oninput: {
                                let mut limit = limit.clone();
                                move |ev| {
                                    if let Ok(val) = ev.value().parse::<usize>() {
                                        *limit.write() = val;
                                    }
                                }
                            }
                        }
                        div {
                            class: "flex justify-between text-xs text-gray-500",
                            span { "100" }
                            span { "5000" }
                        }
                    }
                }
                
                // PyTorch Profiler controls
                if *data_source.read() == "pytorch" {
                    div {
                        class: "space-y-4",
                        div {
                            class: "space-y-2",
                            div {
                                class: "flex items-center justify-between",
                                span {
                                    class: "text-sm text-gray-600",
                                    "Number of Steps"
                                }
                                input {
                                    r#type: "number",
                                    min: "1",
                                    max: "100",
                                    value: "{*pytorch_steps.read()}",
                                    class: "w-20 px-2 py-1 border border-gray-300 rounded text-sm",
                                    oninput: {
                                        let mut steps = pytorch_steps.clone();
                                        move |ev| {
                                            if let Ok(val) = ev.value().parse::<i32>() {
                                                *steps.write() = val.max(1).min(100);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        div {
                            class: "flex items-center space-x-4",
                            button {
                                class: "px-4 py-2 text-sm font-medium rounded-md bg-green-600 text-white hover:bg-green-700 disabled:bg-gray-400 disabled:cursor-not-allowed",
                                disabled: profile_state.is_loading(),
                                onclick: {
                                    let mut profile_state = profile_state.clone();
                                    let steps = pytorch_steps.clone();
                                    move |_| {
                                        spawn(async move {
                                            *profile_state.loading.write() = true;
                                            let client = ApiClient::new();
                                            let result = client.start_pytorch_profile(*steps.read()).await;
                                            *profile_state.data.write() = Some(result);
                                            *profile_state.loading.write() = false;
                                        });
                                    }
                                },
                                if profile_state.is_loading() {
                                    "Starting Profile..."
                                } else {
                                    "Start Profile"
                                }
                            }
                            button {
                                class: "px-4 py-2 text-sm font-medium rounded-md bg-blue-600 text-white hover:bg-blue-700 disabled:bg-gray-400 disabled:cursor-not-allowed",
                                disabled: state.is_loading(),
                                onclick: {
                                    let mut state = state.clone();
                                    let mut iframe_key = iframe_key.clone();
                                    move |_| {
                                        spawn(async move {
                                            *state.loading.write() = true;
                                            *state.data.write() = None; // Clear previous data
                                            let client = ApiClient::new();
                                            let result = client.get_pytorch_timeline().await;
                                            match &result {
                                                Ok(ref data) => {
                                                    log::info!("PyTorch timeline loaded successfully, length: {}", data.len());
                                                }
                                                Err(ref err) => {
                                                    log::error!("Failed to load PyTorch timeline: {:?}", err);
                                                }
                                            }
                                            *state.data.write() = Some(result);
                                            *state.loading.write() = false;
                                            *iframe_key.write() += 1;
                                        });
                                    }
                                },
                                if state.is_loading() {
                                    "Loading Timeline..."
                                } else {
                                    "Load Timeline"
                                }
                            }
                        }
                        if let Some(Ok(ref profile_result)) = profile_state.data.read().as_ref() {
                            if profile_result.success {
                                div {
                                    class: "mt-2 p-2 bg-green-50 border border-green-200 rounded text-sm text-green-800",
                                    if let Some(ref msg) = profile_result.message {
                                        "{msg}"
                                    } else {
                                        "Profile started successfully"
                                    }
                                }
                            } else {
                                div {
                                    class: "mt-2 p-2 bg-red-50 border border-red-200 rounded text-sm text-red-800",
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
            
            // Chrome Tracing Viewer
            if state.is_loading() {
                LoadingState { 
                    message: Some(if *data_source.read() == "pytorch" {
                        "Loading PyTorch timeline data...".to_string()
                    } else {
                        "Loading trace data...".to_string()
                    })
                }
            } else if let Some(Ok(ref trace_json)) = state.data.read().as_ref() {
                // 使用已加载的数据直接显示
                // 验证数据是否是有效的 JSON
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
                        class: "bg-white rounded-lg shadow overflow-hidden",
                        style: "height: calc(100vh - 300px); min-height: 600px;",
                        iframe {
                            key: "{*iframe_key.read()}",
                            srcdoc: get_tracing_viewer_html(trace_json),
                            style: "width: 100%; height: 100%; border: none;",
                            title: "Chrome Tracing Viewer"
                        }
                    }
                }
            } else if let Some(Err(ref err)) = state.data.read().as_ref() {
                // 显示错误信息
                ErrorState { 
                    error: format!("Failed to load timeline: {:?}", err), 
                    title: Some("Load Timeline Error".to_string())
                }
            } else {
                // 没有数据，显示提示信息
                div {
                    class: "bg-white rounded-lg shadow p-8 text-center",
                    div {
                        class: "text-gray-500",
                        if *data_source.read() == "pytorch" {
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
}

/// 生成 tracing viewer 的 URL，通过 URL 参数指定要加载的 JSON 数据
/// 创建一个 HTML 页面，通过 URL 参数获取 JSON URL，然后加载到 Perfetto UI
fn get_tracing_viewer_url(data_source: String, limit: usize) -> String {
    // 构建 API URL 来获取 JSON 数据
    let api_path = if data_source == "pytorch" {
        "/apis/pythonext/pytorch/timeline".to_string()
    } else {
        format!("/apis/pythonext/trace/chrome-tracing?limit={}", limit)
    };
    
    // 获取当前页面的 origin
    let origin = web_sys::window()
        .and_then(|w| w.location().origin().ok())
        .unwrap_or_else(|| "http://localhost:8080".to_string());
    
    let json_url = format!("{}{}", origin, api_path);
    
    // 创建一个 HTML 页面，通过 URL 参数传递 JSON URL
    // 这样 iframe 可以自动加载远程 JSON 数据
    get_tracing_viewer_html_with_url(&json_url)
}

/// 生成包含 Chrome tracing viewer 的 HTML 页面
/// 通过 URL 参数获取 JSON 数据的 URL，然后自动加载到 Perfetto UI
/// 先获取 JSON 数据，然后通过 blob URL 传递给 Perfetto UI 以避免 CORS 问题
fn get_tracing_viewer_html_with_url(json_url: &str) -> String {
    // 转义 URL 以便嵌入到 JavaScript 中
    let escaped_url = json_url
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
                // 从 URL 参数获取 JSON 数据的 URL，或者使用默认值
                const urlParams = new URLSearchParams(window.location.search);
                const jsonUrl = urlParams.get('url') || `{escaped_url}`;
                
                const iframe = document.getElementById('perfetto-iframe');
                const loading = document.getElementById('loading');
                
                // 先获取 JSON 数据，然后使用 Catapult trace viewer 直接显示
                // 这样可以避免 iframe 跨域问题
                loading.textContent = 'Loading trace data...';
                
                // Fetch with CORS support
                // Note: The server must have CORS headers configured
                fetch(jsonUrl, {{
                    method: 'GET',
                    mode: 'cors',
                    credentials: 'omit',
                    headers: {{
                        'Accept': 'application/json',
                    }}
                }})
                    .then(response => {{
                        if (!response.ok) {{
                            throw new Error(`HTTP error! status: ${{response.status}}`);
                        }}
                        return response.text();
                    }})
                    .then(jsonText => {{
                        // 验证并解析 JSON 格式
                        let traceData;
                        try {{
                            traceData = JSON.parse(jsonText);
                        }} catch (e) {{
                            throw new Error('Invalid JSON data: ' + e.message);
                        }}
                        
                        // 直接使用 Perfetto UI，通过 blob URL 传递数据
                        // Perfetto UI 是 Google 官方的新 tracing 工具，更可靠
                        loadPerfettoUI(traceData, jsonUrl);
                    }})
                    .catch(error => {{
                        console.error('Error loading trace data:', error);
                        showError('Failed to load trace data: ' + error.message, jsonUrl);
                    }});
                
                function loadPerfettoUI(traceData, jsonUrl) {{
                    loading.textContent = 'Loading Perfetto UI...';
                    iframe.style.display = 'block';
                    
                    // 使用 Perfetto UI 的 postMessage API 传递 trace 数据
                    // 这样可以避免 CSP 限制，并且更加可靠
                    const perfettoUrl = 'https://ui.perfetto.dev/#!/';
                    iframe.src = perfettoUrl;
                    
                    let loaded = false;
                    let errorShown = false;
                    
                    // 监听来自 Perfetto UI 的消息
                    const messageHandler = function(event) {{
                        // 检查是否是来自 Perfetto UI 的消息
                        if (event.origin === 'https://ui.perfetto.dev') {{
                            if (event.data) {{
                                const dataStr = typeof event.data === 'string' ? event.data : JSON.stringify(event.data);
                                if (dataStr.includes('error') || dataStr.includes('Failed')) {{
                                    console.error('Perfetto UI error:', event.data);
                                    if (!loaded && !errorShown) {{
                                        errorShown = true;
                                        showError('Perfetto UI reported an error. Please check the trace data format.', jsonUrl);
                                        window.removeEventListener('message', messageHandler);
                                    }}
                                }} else if (dataStr.includes('loaded') || dataStr.includes('ready')) {{
                                    // Trace 加载成功
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
                        // 然后通过 postMessage 发送 trace 数据
                        let handshakeComplete = false;
                        let retryCount = 0;
                        const maxRetries = 10;
                        
                        // 监听来自 Perfetto UI 的 PONG 消息
                        // 注意：在 iframe 场景中，我们需要监听来自 iframe 的消息
                        const handshakeHandler = function(event) {{
                            // 检查消息是否来自 Perfetto UI iframe
                            if (event.origin === 'https://ui.perfetto.dev' || 
                                (event.source === iframe.contentWindow && event.data === 'PONG')) {{
                                if (event.data && event.data === 'PONG') {{
                                    handshakeComplete = true;
                                    window.removeEventListener('message', handshakeHandler);
                                    
                                    // Handshake 完成，发送 trace 数据
                                    try {{
                                        // 将 trace 数据转换为 JSON 字符串，然后转换为 ArrayBuffer
                                        const traceJson = JSON.stringify(traceData, null, 2);
                                        const encoder = new TextEncoder();
                                        const buffer = encoder.encode(traceJson).buffer;
                                        
                                        // 构建文件名（从 URL 提取或使用默认值）
                                        const urlParts = jsonUrl.split('/');
                                        const fileName = urlParts[urlParts.length - 1].split('?')[0] || 'trace.json';
                                        
                                        // 发送 trace 数据到 Perfetto UI
                                        // 使用 iframe.contentWindow.postMessage 发送消息
                                        iframe.contentWindow.postMessage({{
                                            perfetto: {{
                                                buffer: buffer,
                                                title: 'Chrome Tracing Data',
                                                fileName: fileName,
                                                url: jsonUrl
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
                                            showError('Failed to send trace data to Perfetto UI: ' + e.message, jsonUrl);
                                            window.removeEventListener('message', messageHandler);
                                        }}
                                    }}
                                }}
                            }}
                        }};
                        window.addEventListener('message', handshakeHandler);
                        
                        // 发送 PING 消息启动 handshake
                        // 注意：window.open 的消息通道不是缓冲的，所以我们需要等待 UI 准备好
                        // 在 iframe 场景中，我们也需要等待 iframe 加载完成
                        const sendPing = function() {{
                            if (!handshakeComplete && retryCount < maxRetries) {{
                                try {{
                                    // 确保 iframe 的 contentWindow 可用
                                    if (iframe.contentWindow) {{
                                        iframe.contentWindow.postMessage('PING', 'https://ui.perfetto.dev');
                                        retryCount++;
                                        if (retryCount < maxRetries) {{
                                            setTimeout(sendPing, 500);
                                        }} else {{
                                            // 如果 handshake 失败，尝试使用 URL 方式作为回退
                                            console.warn('PING/PONG handshake failed after ' + maxRetries + ' attempts, trying URL fallback');
                                            const traceJson = JSON.stringify(traceData, null, 2);
                                            const base64Data = btoa(unescape(encodeURIComponent(traceJson)));
                                            const dataUrl = 'data:application/json;base64,' + base64Data;
                                            iframe.src = 'https://ui.perfetto.dev/#!/?url=' + encodeURIComponent(dataUrl);
                                            window.removeEventListener('message', handshakeHandler);
                                        }}
                                    }} else {{
                                        // iframe 还没有加载完成，稍后重试
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
                        // 给 Perfetto UI 一些时间注册消息监听器
                        setTimeout(sendPing, 1500);
                        
                        // 超时处理
                        setTimeout(() => {{
                            if (!loaded && !errorShown) {{
                                // 如果 10 秒后还没有加载完成，假设加载成功
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
                            showError('Failed to load Perfetto UI', jsonUrl);
                            window.removeEventListener('message', messageHandler);
                        }}
                    }};
                    
                    // 设置超时，如果 30 秒后还没加载完成，显示错误
                    setTimeout(function() {{
                        if (!loaded && !errorShown) {{
                            errorShown = true;
                            showError('Loading timeout. The trace viewer is taking longer than expected. The trace data may be too large or the format may be invalid.', jsonUrl);
                            window.removeEventListener('message', messageHandler);
                        }}
                    }}, 30000);
                }}
                
                function showError(message, jsonUrl) {{
                    loading.innerHTML = `
                        <div style="padding: 20px; text-align: center;">
                            <h2>${{message}}</h2>
                            <p>You can view this trace in Chrome DevTools:</p>
                            <ol style="text-align: left; display: inline-block;">
                                <li>Open Chrome and navigate to <code>chrome://tracing</code></li>
                                <li>Click "Load" and select the trace file</li>
                                <li>Or download the JSON file from: <a href="${{jsonUrl}}" target="_blank" download="trace.json">${{jsonUrl}}</a></li>
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
                                            title: 'PyTorch Profiler Timeline',
                                            fileName: 'pytorch_timeline.json',
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

