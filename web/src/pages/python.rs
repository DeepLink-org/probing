use dioxus::prelude::*;
use crate::components::page::{PageContainer, PageHeader};
use crate::components::common::{LoadingState, ErrorState, EmptyState};
use crate::hooks::use_api;
use crate::api::{ApiClient};

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
                        div {
                            class: "space-y-2 max-h-96 overflow-y-auto",
                            {
                                functions.iter().map(|func| {
                                    let func_clone = func.clone();
                                    rsx! {
                                        div {
                                            class: "p-3 bg-gray-50 rounded-md hover:bg-gray-100 cursor-pointer",
                                            onclick: move |_| {
                                                *function_name.write() = func_clone.clone();
                                            },
                                            div {
                                                class: "font-medium text-gray-900",
                                                "{func}"
                                            }
                                        }
                                    }
                                })
                            }
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
