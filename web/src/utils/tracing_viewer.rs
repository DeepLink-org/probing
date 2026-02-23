//! Shared Chrome/Perfetto tracing viewer HTML generator.

/// Generate HTML page containing Chrome tracing viewer.
/// Embeds trace JSON and loads Perfetto UI via postMessage API.
pub fn get_tracing_viewer_html(trace_json: &str) -> String {
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
                                            title: 'Chrome Tracing Data',
                                            fileName: 'trace.json',
                                        }}
                                    }}, 'https://ui.perfetto.dev');

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
