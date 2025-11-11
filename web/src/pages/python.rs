use dioxus::prelude::*;
use crate::components::page::{PageContainer, PageHeader};
use crate::components::common::{LoadingState, ErrorState, EmptyState};
use crate::hooks::use_api;
use crate::api::ApiClient;
use chrono::{DateTime, Utc};
use std::time::{SystemTime, Duration};

#[component]
pub fn Python() -> Element {
    let mut selected_tab = use_signal(|| "trace".to_string());
    
    rsx! {
        PageContainer {
            PageHeader {
                title: "Python".to_string(),
                subtitle: Some("Inspect and debug Python processes".to_string())
            }
            
            // Tab navigation
            div {
                class: "mb-6 border-b border-gray-200",
                div {
                    class: "flex space-x-1",
                    button {
                        class: format!("px-4 py-2 font-medium text-sm border-b-2 transition-colors {}",
                            if selected_tab.read().as_str() == "trace" {
                                "border-blue-600 text-blue-600"
                            } else {
                                "border-transparent text-gray-500 hover:text-gray-700"
                            }
                        ),
                        onclick: move |_| *selected_tab.write() = "trace".to_string(),
                        "Trace"
                    }
                }
            }
            
            // Trace tab content
            if selected_tab.read().as_str() == "trace" {
                TraceView {}
            }
        }
    }
}

#[component]
fn TraceView() -> Element {
    let mut function_name = use_signal(|| String::new());
    let mut watch_vars = use_signal(|| String::new());
    let mut depth = use_signal(|| 1);
    let action_result = use_signal(|| Option::<String>::None);
    let mut selected_function_filter = use_signal(|| Option::<String>::None);
    let mut refresh_key = use_signal(|| 0);
    
    // Load traceable functions
    let functions_state = use_api(move || {
        let client = ApiClient::new();
        async move {
            client.get_traceable_functions(None).await
        }
    });
    
    // Load current trace info
    let trace_info_state = use_api(move || {
        let client = ApiClient::new();
        async move {
            client.get_trace_info().await
        }
    });
    
    // Load variable records
    let variables_state = use_api(move || {
        let client = ApiClient::new();
        let func_filter = selected_function_filter.read().clone();
        let _refresh = *refresh_key.read();
        async move {
            client.get_variable_records(func_filter.as_deref(), Some(100)).await
        }
    });
    
    rsx! {
        div {
            class: "space-y-6",
            // Start Trace Card
            div {
                class: "bg-white shadow-md rounded-lg p-6",
                h2 {
                    class: "text-xl font-semibold mb-4",
                    "Start Tracing"
                }
                div {
                    class: "space-y-4",
                    div {
                        class: "space-y-2",
                        label {
                            class: "block text-sm font-medium text-gray-700",
                            "Function Name"
                        }
                        input {
                            class: "w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500",
                            r#type: "text",
                            placeholder: "e.g., my_module.my_function",
                            value: "{function_name.read()}",
                            oninput: move |e| *function_name.write() = e.value(),
                        }
                    }
                    
                    div {
                        class: "grid grid-cols-2 gap-4",
                        div {
                            class: "space-y-2",
                            label {
                                class: "block text-sm font-medium text-gray-700",
                                "Watch Variables (comma-separated)"
                            }
                            input {
                                class: "w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500",
                                r#type: "text",
                                placeholder: "e.g., x, y, z",
                                value: "{watch_vars.read()}",
                                oninput: move |e| *watch_vars.write() = e.value(),
                            }
                        }
                        
                        div {
                            class: "space-y-2",
                            label {
                                class: "block text-sm font-medium text-gray-700",
                                "Depth"
                            }
                            input {
                                class: "w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500",
                                r#type: "number",
                                min: "1",
                                value: "{depth.read()}",
                                oninput: move |e| {
                                    if let Ok(v) = e.value().parse::<i32>() {
                                        *depth.write() = v.max(1);
                                    }
                                },
                            }
                        }
                    }
                    
                    if let Some(msg) = action_result.read().as_ref() {
                        div {
                            class: "p-3 rounded-md bg-blue-50 border border-blue-200 text-blue-800 text-sm",
                            "{msg}"
                        }
                    }
                    
                    button {
                        class: "w-full px-4 py-2 bg-blue-600 text-white rounded-md hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-50 disabled:cursor-not-allowed",
                        disabled: function_name.read().is_empty(),
                        onclick: move |_| {
                            let func = function_name.read().clone();
                            let watch = watch_vars.read().clone();
                            let depth_val = *depth.read();
                            let mut result = action_result;
                            
                            spawn(async move {
                                let client = ApiClient::new();
                                let watch_list: Vec<String> = if watch.is_empty() {
                                    vec![]
                                } else {
                                    watch.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
                                };
                                
                                match client.start_trace(&func, Some(watch_list), Some(depth_val)).await {
                                    Ok(resp) => {
                                        if resp.success {
                                            *result.write() = Some(resp.message.unwrap_or_else(|| "Trace started successfully".to_string()));
                                            *refresh_key.write() += 1;
                                        } else {
                                            *result.write() = Some(resp.error.unwrap_or_else(|| "Failed to start trace".to_string()));
                                        }
                                    }
                                    Err(e) => {
                                        *result.write() = Some(format!("Error: {:?}", e));
                                    }
                                }
                            });
                        },
                        "Start Trace"
                    }
                }
            }
            
            // Variable Watch Records Card
            div {
                class: "bg-white shadow-md rounded-lg p-6",
                div {
                    class: "flex items-center justify-between mb-4",
                    h2 {
                        class: "text-xl font-semibold",
                        "Variable Watch Records"
                    }
                    div {
                        class: "flex items-center gap-2",
                        select {
                            class: "px-3 py-1 border border-gray-300 rounded-md text-sm focus:outline-none focus:ring-2 focus:ring-blue-500",
                            onchange: move |e| {
                                let val = e.value();
                                if val.is_empty() {
                                    *selected_function_filter.write() = None;
                                } else {
                                    *selected_function_filter.write() = Some(val);
                                }
                                *refresh_key.write() += 1;
                            },
                            option {
                                value: "",
                                "All Functions"
                            }
                            if let Some(Ok(traces)) = trace_info_state.data.read().as_ref() {
                                {
                                    traces.iter().map(|func| {
                                        rsx! {
                                            option {
                                                value: "{func}",
                                                "{func}"
                                            }
                                        }
                                    })
                                }
                            }
                        }
                        button {
                            class: "px-3 py-1 bg-blue-600 text-white text-sm rounded hover:bg-blue-700",
                            onclick: move |_| {
                                *refresh_key.write() += 1;
                            },
                            "Refresh"
                        }
                    }
                }
                if variables_state.is_loading() {
                    LoadingState { message: Some("Loading variable records...".to_string()) }
                } else if let Some(Ok(records)) = variables_state.data.read().as_ref() {
                    if records.is_empty() {
                        EmptyState { message: "No variable records found".to_string() }
                    } else {
                        div {
                            class: "overflow-x-auto",
                            table {
                                class: "min-w-full divide-y divide-gray-200",
                                thead {
                                    class: "bg-gray-50",
                                    tr {
                                        th {
                                            class: "px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider",
                                            "Function"
                                        }
                                        th {
                                            class: "px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider",
                                            "File"
                                        }
                                        th {
                                            class: "px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider",
                                            "Line"
                                        }
                                        th {
                                            class: "px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider",
                                            "Variable"
                                        }
                                        th {
                                            class: "px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider",
                                            "Value"
                                        }
                                        th {
                                            class: "px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider",
                                            "Type"
                                        }
                                        th {
                                            class: "px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider",
                                            "Time"
                                        }
                                    }
                                }
                                tbody {
                                    class: "bg-white divide-y divide-gray-200",
                                    {
                                        records.iter().map(|record| {
                                            rsx! {
                                                tr {
                                                    class: "hover:bg-gray-50",
                                                    td {
                                                        class: "px-4 py-3 whitespace-nowrap text-sm font-medium text-gray-900",
                                                        "{record.function_name}"
                                                    }
                                                    td {
                                                        class: "px-4 py-3 whitespace-nowrap text-sm text-gray-500",
                                                        "{record.filename}"
                                                    }
                                                    td {
                                                        class: "px-4 py-3 whitespace-nowrap text-sm text-gray-500",
                                                        "{record.lineno}"
                                                    }
                                                    td {
                                                        class: "px-4 py-3 whitespace-nowrap text-sm font-medium text-blue-600",
                                                        "{record.variable_name}"
                                                    }
                                                    td {
                                                        class: "px-4 py-3 text-sm text-gray-900 max-w-xs truncate",
                                                        title: "{record.value.clone()}",
                                                        "{record.value}"
                                                    }
                                                    td {
                                                        class: "px-4 py-3 whitespace-nowrap text-sm text-gray-500",
                                                        "{record.value_type}"
                                                    }
                                                    td {
                                                        class: "px-4 py-3 whitespace-nowrap text-sm text-gray-500",
                                                        {
                                                            let timestamp = record.timestamp;
                                                            // Format timestamp as readable time
                                                            let secs = timestamp as i64;
                                                            let nanos = ((timestamp - secs as f64) * 1_000_000_000.0) as u32;
                                                            let datetime: DateTime<Utc> = (SystemTime::UNIX_EPOCH
                                                                + Duration::from_secs(secs as u64)
                                                                + Duration::from_nanos(nanos as u64))
                                                                .into();
                                                            format!("{}", datetime.format("%H:%M:%S%.3f"))
                                                        }
                                                    }
                                                }
                                            }
                                        })
                                    }
                                }
                            }
                        }
                    }
                } else if let Some(Err(err)) = variables_state.data.read().as_ref() {
                    ErrorState { error: format!("{:?}", err), title: None }
                } else {
                    EmptyState { message: "No data available".to_string() }
                }
            }
            
            // Traceable Functions Card
            div {
                class: "bg-white shadow-md rounded-lg p-6",
                h2 {
                    class: "text-xl font-semibold mb-4",
                    "Traceable Functions"
                }
                if functions_state.is_loading() {
                    LoadingState { message: Some("Loading traceable functions...".to_string()) }
                } else if let Some(Ok(functions)) = functions_state.data.read().as_ref() {
                    if functions.is_empty() {
                        EmptyState { message: "No traceable functions found".to_string() }
                    } else {
                        TraceableFunctionsList {
                            functions: functions.clone(),
                            function_name: function_name,
                        }
                    }
                } else if let Some(Err(err)) = functions_state.data.read().as_ref() {
                    ErrorState { error: format!("{:?}", err), title: None }
                } else {
                    EmptyState { message: "No data available".to_string() }
                }
            }
            
            // Active Traces Card
            div {
                class: "bg-white shadow-md rounded-lg p-6",
                h2 {
                    class: "text-xl font-semibold mb-4",
                    "Active Traces"
                }
                if trace_info_state.is_loading() {
                    LoadingState { message: Some("Loading trace information...".to_string()) }
                } else if let Some(Ok(traces)) = trace_info_state.data.read().as_ref() {
                    if traces.is_empty() {
                        EmptyState { message: "No active traces".to_string() }
                    } else {
                        div {
                            class: "space-y-4",
                            {
                                traces.iter().map(|func_name| {
                                    let func_name_clone = func_name.clone();
                                    let mut result = action_result;
                                    rsx! {
                                        div {
                                            class: "p-4 bg-gray-50 rounded-md border border-gray-200",
                                            div {
                                                class: "flex items-center justify-between",
                                                div {
                                                    class: "font-medium text-gray-900",
                                                    "{func_name}"
                                                }
                                                button {
                                                    class: "px-3 py-1 bg-red-600 text-white text-sm rounded hover:bg-red-700",
                                                    onclick: move |_| {
                                                        let func = func_name_clone.clone();
                                                        let mut result = result;
                                                        
                                                        spawn(async move {
                                                            let client = ApiClient::new();
                                                            match client.stop_trace(&func).await {
                                                                Ok(resp) => {
                                                                    if resp.success {
                                                                        *result.write() = Some(resp.message.unwrap_or_else(|| "Trace stopped successfully".to_string()));
                                                                    } else {
                                                                        *result.write() = Some(resp.error.unwrap_or_else(|| "Failed to stop trace".to_string()));
                                                                    }
                                                                }
                                                                Err(e) => {
                                                                    *result.write() = Some(format!("Error: {:?}", e));
                                                                }
                                                            }
                                                        });
                                                    },
                                                    "Stop"
                                                }
                                            }
                                        }
                                    }
                                })
                            }
                        }
                    }
                } else if let Some(Err(err)) = trace_info_state.data.read().as_ref() {
                    ErrorState { error: format!("{:?}", err), title: None }
                } else {
                    EmptyState { message: "No data available".to_string() }
                }
            }
        }
    }
}

#[component]
fn TraceableFunctionsList(
    functions: Vec<String>,
    function_name: Signal<String>,
) -> Element {
    rsx! {
        div {
            class: "space-y-1 max-h-96 overflow-y-auto",
            {
                functions.iter().map(|func_str| {
                    let func_name_clone = function_name.clone();
                    
                    // Parse function string: "[TYPE] name"
                    let (item_type, name) = if let Some(bracket_end) = func_str.find(']') {
                        let item_type = func_str[1..bracket_end].to_string();
                        let name = func_str[bracket_end + 2..].to_string();
                        (item_type, name)
                    } else {
                        ("".to_string(), func_str.clone())
                    };
                    
                    let is_module = item_type == "M";
                    
                    rsx! {
                        TraceableFunctionItem {
                            name: name,
                            item_type: item_type,
                            is_module: is_module,
                            function_name: func_name_clone,
                        }
                    }
                })
            }
        }
    }
}

#[component]
fn TraceableFunctionItem(
    name: String,
    item_type: String,
    is_module: bool,
    function_name: Signal<String>,
) -> Element {
    let mut expanded = use_signal(|| false);
    let mut children_state = use_signal(|| Option::<Vec<String>>::None);
    let mut loading = use_signal(|| false);
    
    let name_for_display = name.clone();
    let name_for_click = name.clone();
    let name_for_expand = name.clone();
    let item_type_clone = item_type.clone();
    
    rsx! {
        div {
            class: "border border-gray-200 rounded-md",
            div {
                class: "flex items-center gap-2 p-2 hover:bg-gray-50",
                // Expand/collapse button for modules
                if is_module {
                    button {
                        class: "text-gray-400 hover:text-gray-600 flex-shrink-0 w-4 h-4 flex items-center justify-center",
                        onclick: move |_| {
                            let current = *expanded.read();
                            *expanded.write() = !current;
                            
                            if !current && children_state.read().is_none() {
                                // Load children when expanding
                                *loading.write() = true;
                                let prefix = name_for_expand.clone();
                                let mut children = children_state;
                                let mut loading_state = loading;
                                
                                spawn(async move {
                                    let client = ApiClient::new();
                                    match client.get_traceable_functions(Some(&prefix)).await {
                                        Ok(child_functions) => {
                                            *children.write() = Some(child_functions);
                                            *loading_state.write() = false;
                                        }
                                        Err(e) => {
                                            *loading_state.write() = false;
                                            log::error!("Failed to load children: {:?}", e);
                                        }
                                    }
                                });
                            }
                        },
                        if *expanded.read() {
                            "▼"
                        } else {
                            "▶"
                        }
                    }
                } else {
                    div {
                        class: "w-4 h-4",
                    }
                }
                
                // Type badge
                {
                    if !item_type_clone.is_empty() {
                        let color_class = match item_type_clone.as_str() {
                            "F" => "bg-blue-100 text-blue-800",
                            "C" => "bg-green-100 text-green-800",
                            "M" => "bg-purple-100 text-purple-800",
                            "V" => "bg-yellow-100 text-yellow-800",
                            _ => "bg-gray-100 text-gray-800",
                        };
                        rsx! {
                            span {
                                class: "text-xs px-1.5 py-0.5 rounded font-mono {color_class}",
                                "[{item_type_clone}]"
                            }
                        }
                    } else {
                        rsx! { div {} }
                    }
                }
                
                // Function name (clickable to select)
                div {
                    class: "flex-1 font-medium text-gray-900 cursor-pointer",
                    onclick: move |_| {
                        *function_name.write() = name_for_click.clone();
                    },
                    "{name_for_display}"
                }
            }
            
            // Children list (for modules)
            if is_module && *expanded.read() {
                div {
                    class: "pl-6 pr-2 pb-2 border-t border-gray-200",
                    if *loading.read() {
                        div {
                            class: "p-2 text-sm text-gray-500",
                            "Loading..."
                        }
                    } else if let Some(children) = children_state.read().as_ref() {
                        if children.is_empty() {
                            div {
                                class: "p-2 text-sm text-gray-500",
                                "No items found"
                            }
                        } else {
                            div {
                                class: "space-y-1 mt-1",
                                {
                                    children.iter().map(|child_str| {
                                        let child_str_clone = child_str.clone();
                                        let func_name_clone = function_name.clone();
                                        
                                        // Parse child function string
                                        let (child_type, child_name) = if let Some(bracket_end) = child_str.find(']') {
                                            let child_type = child_str[1..bracket_end].to_string();
                                            let child_name = child_str[bracket_end + 2..].to_string();
                                            (child_type, child_name)
                                        } else {
                                            ("".to_string(), child_str.clone())
                                        };
                                        
                                        let child_is_module = child_type == "M";
                                        let child_name_clone = child_name.clone();
                                        
                                        rsx! {
                                            TraceableFunctionItem {
                                                name: child_name,
                                                item_type: child_type,
                                                is_module: child_is_module,
                                                function_name: func_name_clone,
                                            }
                                        }
                                    })
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
