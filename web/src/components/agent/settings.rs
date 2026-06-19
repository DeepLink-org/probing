//! LLM API key & model settings (localStorage).

use dioxus::prelude::*;

use crate::components::icon::Icon;
use crate::state::llm_config::{clear_llm_api_key, save_llm_config, LLM_CONFIG, LLM_SETTINGS_OPEN};

#[component]
pub fn LlmSettingsOverlay() -> Element {
    if !*LLM_SETTINGS_OPEN.read() {
        return rsx! {};
    }

    let cfg = LLM_CONFIG.read().clone();
    let mut draft = use_signal(|| cfg.clone());

    rsx! {
        div {
            class: "fixed inset-0 z-[9998] flex items-center justify-center bg-black/30 backdrop-blur-sm p-4",
            onclick: move |_| *LLM_SETTINGS_OPEN.write() = false,
            div {
                class: "w-full max-w-md bg-white rounded-xl shadow-xl border border-gray-200 overflow-hidden",
                onclick: move |e| e.stop_propagation(),
                div { class: "flex items-center justify-between px-4 py-3 border-b border-gray-100 bg-gray-50",
                    div { class: "flex items-center gap-2",
                        Icon { icon: &icondata::AiSettingOutlined, class: "w-4 h-4 text-gray-600" }
                        h2 { class: "text-sm font-semibold text-gray-900", "LLM settings" }
                    }
                    button {
                        class: "p-1 rounded-md text-gray-400 hover:text-gray-700",
                        onclick: move |_| *LLM_SETTINGS_OPEN.write() = false,
                        Icon { icon: &icondata::AiCloseOutlined, class: "w-4 h-4" }
                    }
                }
                div { class: "px-4 py-4 space-y-3 text-sm",
                    p { class: "text-xs text-gray-500 leading-relaxed",
                        "API key is saved in this browser only (localStorage). \
                         Default: DeepSeek (`api.deepseek.com`). Endpoint must be OpenAI-compatible \
                         and allow browser CORS."
                    }
                    label { class: "block space-y-1",
                        span { class: "text-xs font-medium text-gray-700", "API base URL" }
                        input {
                            class: "w-full px-3 py-2 border border-gray-300 rounded-lg text-sm font-mono",
                            value: "{draft.read().api_base}",
                            oninput: move |e| draft.write().api_base = e.value(),
                        }
                    }
                    label { class: "block space-y-1",
                        span { class: "text-xs font-medium text-gray-700", "API key" }
                        input {
                            r#type: "password",
                            class: "w-full px-3 py-2 border border-gray-300 rounded-lg text-sm font-mono",
                            placeholder: "sk-…",
                            value: "{draft.read().api_key}",
                            oninput: move |e| draft.write().api_key = e.value(),
                        }
                        if !cfg.api_key.is_empty() {
                            p { class: "text-[10px] text-gray-400", "Saved: {cfg.masked_key_hint()}" }
                        }
                    }
                    label { class: "block space-y-1",
                        span { class: "text-xs font-medium text-gray-700", "Model" }
                        input {
                            class: "w-full px-3 py-2 border border-gray-300 rounded-lg text-sm font-mono",
                            value: "{draft.read().model}",
                            oninput: move |e| draft.write().model = e.value(),
                        }
                    }
                }
                div { class: "flex items-center justify-between gap-2 px-4 py-3 border-t border-gray-100 bg-gray-50",
                    button {
                        class: "text-xs text-red-600 hover:underline disabled:opacity-40",
                        disabled: cfg.api_key.is_empty(),
                        onclick: move |_| {
                            clear_llm_api_key();
                            draft.write().api_key.clear();
                        },
                        "Clear key"
                    }
                    div { class: "flex gap-2",
                        button {
                            class: "px-3 py-1.5 text-sm rounded-lg border border-gray-300 text-gray-700 hover:bg-gray-100",
                            onclick: move |_| *LLM_SETTINGS_OPEN.write() = false,
                            "Cancel"
                        }
                        button {
                            class: "px-3 py-1.5 text-sm rounded-lg bg-blue-600 text-white hover:bg-blue-700",
                            onclick: move |_| {
                                let to_save = draft.read().clone();
                                save_llm_config(&to_save);
                                *LLM_SETTINGS_OPEN.write() = false;
                            },
                            "Save"
                        }
                    }
                }
            }
        }
    }
}
