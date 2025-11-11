
use dioxus::prelude::*;
use crate::components::page::{PageContainer, PageHeader};
use crate::components::common::{LoadingState, ErrorState, EmptyState};
use crate::hooks::{use_api, use_api_simple};
use crate::api::{ApiClient, TraceableItem, VariableRecord};

/// Safely format timestamp to string
/// Uses simple math to avoid SystemTime/DateTime issues in wasm
fn format_timestamp_safe(timestamp: f64) -> String {
    if timestamp.is_nan() || timestamp.is_infinite() {
        return "Invalid".to_string();
    }
    
    // For negative timestamps, just return the raw value
    if timestamp < 0.0 {
        return format!("{:.3}", timestamp);
    }
    
    // Calculate hours, minutes, seconds, and milliseconds
    let total_secs = timestamp.floor() as u64;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    let millis = ((timestamp - total_secs as f64) * 1000.0).floor() as u64;
    
    // Format as HH:MM:SS.mmm
    format!("{:02}:{:02}:{:02}.{:03}", hours, minutes, seconds, millis)
}

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
                    class: "flex space-x-8",
                    button {
                        class: if *selected_tab.read() == "trace" {
                            "py-4 px-1 border-b-2 border-blue-500 font-medium text-sm text-blue-600"
                        } else {
                            "py-4 px-1 border-b-2 border-transparent font-medium text-sm text-gray-500 hover:text-gray-700 hover:border-gray-300"
                        },
                        onclick: move |_| *selected_tab.write() = "trace".to_string(),
                        "Trace"
                    }
                }
            }
            
            // Tab content
            if *selected_tab.read() == "trace" {
                TraceView {}
            }
        }
    }
}

#[component]
fn TraceView() -> Element {
    let mut selected_function_filter = use_signal(|| Option::<String>::None);
    let mut refresh_key = use_signal(|| 0);
    
    // Load traceable functions (always includes variables)
    let functions_state = use_api(move || {
        let client = ApiClient::new();
        async move {
            client.get_traceable_items(None).await
        }
    });
    
    // Load current trace info
    let trace_info_state = use_api(move || {
        let client = ApiClient::new();
        async move {
            client.get_trace_info().await
        }
    });
    
    // Variable records preview state
    let records_state = use_api_simple::<Vec<VariableRecord>>();
    let mut preview_function_name = use_signal(|| String::new());
    let mut preview_open = use_signal(|| false);
    
    // Dialog state
    let mut dialog_open = use_signal(|| false);
    let mut dialog_function_name = use_signal(|| String::new());
    let mut dialog_watch_vars = use_signal(|| String::new());
    let mut dialog_depth = use_signal(|| 1);
    
    rsx! {
        div {
            class: "space-y-6",
            // Active Traces Card (moved to top)
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
                        {
                            let mut loading = records_state.loading;
                            let mut data = records_state.data;
                            let mut preview_fn = preview_function_name;
                            let mut preview_op = preview_open;
                            
                            rsx! {
                                div {
                                    class: "space-y-4",
                                    {
                                        traces.iter().map(|func_name| {
                                            let func_name_clone = func_name.clone();
                                            let func_name_for_preview = func_name.clone();
                                            
                                            rsx! {
                                                div {
                                                    class: "p-4 bg-gray-50 rounded-md border border-gray-200 cursor-pointer hover:bg-gray-100 transition-colors",
                                                    onclick: move |_| {
                                                        *preview_fn.write() = func_name_for_preview.clone();
                                                        *preview_op.write() = true;
                                                        let func_name = func_name_for_preview.clone();
                                                        spawn(async move {
                                                            *loading.write() = true;
                                                            let client = ApiClient::new();
                                                            let resp = client.get_variable_records(Some(&func_name), Some(100)).await;
                                                            *data.write() = Some(resp);
                                                            *loading.write() = false;
                                                        });
                                                    },
                                                    div {
                                                        class: "flex items-center justify-between",
                                                        div {
                                                            class: "font-medium text-gray-900",
                                                            "{func_name}"
                                                        }
                                                        button {
                                                            class: "px-3 py-1 bg-red-600 text-white text-sm rounded hover:bg-red-700",
                                                            onclick: move |e| {
                                                                e.stop_propagation();
                                                                let func = func_name_clone.clone();
                                                                let mut refresh = refresh_key;
                                                                
                                                                spawn(async move {
                                                                    let client = ApiClient::new();
                                                                    match client.stop_trace(&func).await {
                                                                        Ok(_resp) => {
                                                                            *refresh.write() += 1;
                                                                        }
                                                                        Err(_e) => {
                                                                            // Error handling could be added here
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
                        }
                    }
                } else if let Some(Err(err)) = trace_info_state.data.read().as_ref() {
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
                } else if let Some(Ok(items)) = functions_state.data.read().as_ref() {
                    if items.is_empty() {
                        EmptyState { message: "No traceable functions found".to_string() }
                    } else {
                        {
                            let mut dialog_fn = dialog_function_name;
                            let mut dialog_vars = dialog_watch_vars;
                            let mut dialog_dp = dialog_depth;
                            let mut dialog_op = dialog_open;
                            
                            let mut click_signal = use_signal(|| (String::new(), Vec::new()));
                            
                            // Watch for changes in click_signal
                            use_effect(move || {
                                let (func_name, vars) = click_signal.read().clone();
                                if !func_name.is_empty() {
                                    *dialog_fn.write() = func_name.clone();
                                    *dialog_vars.write() = vars.join(", ");
                                    *dialog_dp.write() = 1;
                                    *dialog_op.write() = true;
                                    // Reset signal
                                    *click_signal.write() = (String::new(), Vec::new());
                                }
                            });
                            
                            rsx! {
                                TraceableFunctionsList {
                                    items: items.clone(),
                                    on_function_click: click_signal,
                                }
                            }
                        }
                    }
                } else if let Some(Err(err)) = functions_state.data.read().as_ref() {
                    ErrorState { error: format!("{:?}", err), title: None }
                } else {
                    EmptyState { message: "No data available".to_string() }
                }
            }
            
            // Variable Records Preview Modal
            if *preview_open.read() {
                div {
                    class: "fixed inset-0 z-50 flex items-center justify-center",
                    // Background overlay
                    div {
                        class: "absolute inset-0 bg-black/50",
                        onclick: move |_| {
                            *preview_open.write() = false;
                        }
                    }
                    // Content container
                    div {
                        class: "relative bg-white rounded-lg shadow-lg max-w-5xl w-[90vw] max-h-[80vh] overflow-auto p-4",
                        // Header
                        div {
                            class: "flex items-center justify-between mb-3",
                            h3 {
                                class: "text-lg font-semibold text-gray-900",
                                "Variable Records: {preview_function_name.read()}"
                            }
                            button {
                                class: "px-3 py-1 text-sm rounded bg-gray-100 hover:bg-gray-200",
                                onclick: move |_| {
                                    *preview_open.write() = false;
                                },
                                "Close"
                            }
                        }
                        // Content
                        if records_state.is_loading() {
                            LoadingState { message: Some("Loading records...".to_string()) }
                        } else if let Some(Ok(records)) = records_state.data.read().as_ref() {
                            div {
                                class: "max-h-[60vh] overflow-y-auto",
                                table {
                                    class: "min-w-full divide-y divide-gray-200 text-xs",
                                    thead {
                                        class: "bg-gray-50 sticky top-0",
                                        tr {
                                            th {
                                                class: "px-2 py-2 text-left text-xs font-medium text-gray-500 uppercase",
                                                "File"
                                            }
                                            th {
                                                class: "px-2 py-2 text-left text-xs font-medium text-gray-500 uppercase",
                                                "Line"
                                            }
                                            th {
                                                class: "px-2 py-2 text-left text-xs font-medium text-gray-500 uppercase",
                                                "Variable"
                                            }
                                            th {
                                                class: "px-2 py-2 text-left text-xs font-medium text-gray-500 uppercase",
                                                "Value"
                                            }
                                            th {
                                                class: "px-2 py-2 text-left text-xs font-medium text-gray-500 uppercase",
                                                "Type"
                                            }
                                            th {
                                                class: "px-2 py-2 text-left text-xs font-medium text-gray-500 uppercase",
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
                                                            class: "px-2 py-2 text-xs text-gray-500",
                                                            "{record.filename}"
                                                        }
                                                        td {
                                                            class: "px-2 py-2 text-xs text-gray-500",
                                                            "{record.lineno}"
                                                        }
                                                        td {
                                                            class: "px-2 py-2 text-xs font-medium text-blue-600",
                                                            "{record.variable_name}"
                                                        }
                                                        td {
                                                            class: "px-2 py-2 text-xs text-gray-900 max-w-xs truncate",
                                                            title: "{record.value.clone()}",
                                                            "{record.value}"
                                                        }
                                                        td {
                                                            class: "px-2 py-2 text-xs text-gray-500",
                                                            "{record.value_type}"
                                                        }
                                                        td {
                                                            class: "px-2 py-2 text-xs text-gray-500",
                                                            "{format_timestamp_safe(record.timestamp)}"
                                                        }
                                                    }
                                                }
                                            })
                                        }
                                    }
                                }
                            }
                        } else if let Some(Err(err)) = records_state.data.read().as_ref() {
                            ErrorState { error: format!("{:?}", err), title: None }
                        } else {
                            span {
                                class: "text-gray-500",
                                "Preparing records..."
                            }
                        }
                    }
                }
            }
            
            // Start Trace Confirmation Dialog
            if *dialog_open.read() {
                div {
                    class: "fixed inset-0 z-50 flex items-center justify-center",
                    // Background overlay
                    div {
                        class: "absolute inset-0 bg-black/50",
                        onclick: move |_| {
                            *dialog_open.write() = false;
                        }
                    }
                    // Dialog content
                    div {
                        class: "relative bg-white rounded-lg shadow-lg max-w-md w-[90vw] p-6",
                        onclick: move |e| {
                            e.stop_propagation();
                        },
                        h3 {
                            class: "text-lg font-semibold text-gray-900 mb-4",
                            "Start Tracing"
                        }
                        
                        div {
                            class: "space-y-4",
                            // Function name (read-only)
                            div {
                                class: "space-y-2",
                                label {
                                    class: "block text-sm font-medium text-gray-700",
                                    "Function Name"
                                }
                                input {
                                    class: "w-full px-3 py-2 border border-gray-300 rounded-md bg-gray-50",
                                    r#type: "text",
                                    readonly: true,
                                    value: "{dialog_function_name.read()}",
                                }
                            }
                            
                            // Watch Variables (pre-filled with clicked variable, can add more)
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
                                    value: "{dialog_watch_vars.read()}",
                                    oninput: move |e| *dialog_watch_vars.write() = e.value(),
                                }
                                div {
                                    class: "text-xs text-gray-500 mt-1",
                                    "Tip: You can add more variables separated by commas"
                                }
                            }
                            
                            // Depth
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
                                    value: "{dialog_depth.read()}",
                                    oninput: move |e| {
                                        if let Ok(v) = e.value().parse::<i32>() {
                                            *dialog_depth.write() = v.max(1);
                                        }
                                    },
                                }
                            }
                            
                            // Buttons
                            div {
                                class: "flex gap-3 justify-end pt-4",
                                button {
                                    class: "px-4 py-2 border border-gray-300 rounded-md hover:bg-gray-50 focus:outline-none focus:ring-2 focus:ring-gray-500",
                                    onclick: move |_| {
                                        *dialog_open.write() = false;
                                    },
                                    "Cancel"
                                }
                                button {
                                    class: "px-4 py-2 bg-blue-600 text-white rounded-md hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-blue-500",
                                    onclick: move |_| {
                                        let func = dialog_function_name.read().clone();
                                        let watch = dialog_watch_vars.read().clone();
                                        let depth_val = *dialog_depth.read();
                                        let mut refresh = refresh_key;
                                        let mut dialog_op = dialog_open;
                                        
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
                                                        *refresh.write() += 1;
                                                        *dialog_op.write() = false;
                                                    }
                                                }
                                                Err(_) => {
                                                    // Error handling could be added here
                                                }
                                            }
                                        });
                                    },
                                    "Start Trace"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn TraceableFunctionsList(
    items: Vec<TraceableItem>,
    on_function_click: Signal<(String, Vec<String>)>,
) -> Element {
    rsx! {
        div {
            class: "space-y-2",
            {
                items.iter().map(|item| {
                    rsx! {
                        TraceableFunctionItem {
                            name: item.name.clone(),
                            item_type: item.item_type.clone(),
                            variables: item.variables.clone(),
                            is_module: item.item_type == "M",
                            on_function_click: on_function_click,
                            function_name: item.name.clone(),
                            function_vars: item.variables.clone(),
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
    variables: Vec<String>,
    is_module: bool,
    on_function_click: Signal<(String, Vec<String>)>,
    function_name: String,
    function_vars: Vec<String>,
) -> Element {
    let mut expanded = use_signal(|| false);
    // Default to expanded if variables exist
    let mut variables_expanded = use_signal(|| !variables.is_empty());
    let variables_list = use_signal(|| variables.clone());
    let children_state = use_signal(|| Option::<Vec<TraceableItem>>::None);
    let mut loading = use_signal(|| false);
    
    let name_for_display = name.clone();
    let name_for_click = name.clone();
    let name_for_expand_module = name.clone();
    let name_for_expand_vars = name.clone();
    let item_type_clone = item_type.clone();
    let variables_clone = variables_list.read().clone();
    
    rsx! {
        div {
            class: "border border-gray-200 rounded-md",
            div {
                class: "flex items-center justify-between p-3 hover:bg-gray-50",
                div {
                    class: "flex items-center gap-2 flex-1",
                    // Expand/collapse button for modules
                    if is_module {
                        button {
                            class: "text-gray-500 hover:text-gray-700",
                            onclick: move |_| {
                                let expanded_val = *expanded.read();
                                *expanded.write() = !expanded_val;
                                
                                // Load children if expanding
                                if !expanded_val {
                                    let name_for_load = name_for_expand_module.clone();
                                    let mut children = children_state;
                                    let mut load_state = loading;
                                    
                                    spawn(async move {
                                        *load_state.write() = true;
                                        let client = ApiClient::new();
                                        match client.get_traceable_items(Some(&name_for_load)).await {
                                            Ok(items) => {
                                                *children.write() = Some(items);
                                            }
                                            Err(_) => {
                                                *children.write() = None;
                                            }
                                        }
                                        *load_state.write() = false;
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
                            class: "w-4",
                        }
                    }
                    
                    // Type badge
                    {
                        let item_type_clone = item_type_clone.clone();
                        if item_type_clone == "F" {
                            rsx! {
                                span {
                                    class: "text-xs px-1.5 py-0.5 rounded font-mono bg-blue-100 text-blue-700",
                                    "[F]"
                                }
                            }
                        } else if item_type_clone == "M" {
                            rsx! {
                                span {
                                    class: "text-xs px-1.5 py-0.5 rounded font-mono bg-green-100 text-green-700",
                                    "[M]"
                                }
                            }
                        } else {
                            rsx! {
                                span {
                                    class: "text-xs px-1.5 py-0.5 rounded font-mono bg-gray-100 text-gray-700",
                                    "[{item_type_clone}]"
                                }
                            }
                        }
                    }
                    
                    // Function/module name
                    div {
                        class: "font-medium text-gray-900",
                        "{name_for_display}"
                    }
                    
                    // Expand/collapse button for variables (for functions)
                    if item_type_clone == "F" {
                        button {
                            class: "ml-auto text-xs text-gray-500 hover:text-gray-700",
                            onclick: move |e| {
                                e.stop_propagation();
                                let expanded_val = *variables_expanded.read();
                                *variables_expanded.write() = !expanded_val;
                            },
                            if *variables_expanded.read() {
                                "Hide Variables"
                            } else {
                                "Show Variables"
                            }
                        }
                    }
                }
            }
            
            // Variables list (for functions) - always show if variables exist and expanded
            if item_type_clone == "F" && *variables_expanded.read() {
                {
                    let vars = variables_list.read();
                    if !vars.is_empty() {
                        rsx! {
                            div {
                                class: "pl-6 pr-2 pb-2 border-t border-gray-200 bg-gray-50",
                                div {
                                    class: "pt-2 text-xs font-medium text-gray-600 mb-1",
                                    "Traceable Variables:"
                                }
                                div {
                                    class: "flex flex-wrap gap-1",
                                    {
                                        vars.iter().map(|var| {
                                            let var_clone = var.clone();
                                            let mut click_signal = on_function_click.clone();
                                            let func_name = function_name.clone();
                                            
                                            rsx! {
                                                span {
                                                    class: "text-xs px-2 py-1 bg-blue-50 text-blue-700 rounded border border-blue-200 cursor-pointer hover:bg-blue-100 transition-colors",
                                                    onclick: move |_| {
                                                        // Click variable to open dialog with this function and variable
                                                        *click_signal.write() = (func_name.clone(), vec![var_clone.clone()]);
                                                    },
                                                    "{var_clone}"
                                                }
                                            }
                                        })
                                    }
                                }
                            }
                        }
                    } else {
                        rsx! {
                            div {
                                class: "pl-6 pr-2 pb-2 border-t border-gray-200 bg-gray-50",
                                div {
                                    class: "pt-2 text-xs text-gray-500",
                                    "No variables found"
                                }
                            }
                        }
                    }
                }
            }
            
            // Children list (for modules)
            if is_module && *expanded.read() {
                if *loading.read() {
                    div {
                        class: "pl-6 pr-2 pb-2 border-t border-gray-200 bg-gray-50",
                        div {
                            class: "pt-2 text-xs text-gray-500",
                            "Loading..."
                        }
                    }
                } else if let Some(children) = children_state.read().as_ref() {
                    if children.is_empty() {
                        div {
                            class: "pl-6 pr-2 pb-2 border-t border-gray-200 bg-gray-50",
                            div {
                                class: "pt-2 text-xs text-gray-500",
                                "No items found"
                            }
                        }
                    } else {
                        div {
                            class: "pl-6 pr-2 pb-2 border-t border-gray-200 bg-gray-50",
                            {
                                children.iter().map(|child_item| {
                                    let child_name = child_item.name.clone();
                                    let child_vars = child_item.variables.clone();
                                    rsx! {
                                        TraceableFunctionItem {
                                            name: child_item.name.clone(),
                                            item_type: child_item.item_type.clone(),
                                            variables: child_item.variables.clone(),
                                            is_module: child_item.item_type == "M",
                                            on_function_click: on_function_click,
                                            function_name: child_name.clone(),
                                            function_vars: child_vars.clone(),
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

