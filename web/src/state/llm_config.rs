//! LLM settings persisted in browser localStorage (bring-your-own-key).

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

const STORAGE_KEY: &str = "probing_llm_config";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LlmConfig {
    /// OpenAI-compatible base URL, e.g. `https://api.deepseek.com/v1`
    pub api_base: String,
    /// Bearer token — stored only in this browser's localStorage.
    pub api_key: String,
    pub model: String,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            api_base: "https://api.deepseek.com/v1".to_string(),
            api_key: String::new(),
            model: "deepseek-chat".to_string(),
        }
    }
}

impl LlmConfig {
    pub fn is_configured(&self) -> bool {
        !self.api_key.trim().is_empty() && !self.api_base.trim().is_empty()
    }

    pub fn masked_key_hint(&self) -> String {
        let k = self.api_key.trim();
        if k.is_empty() {
            return "Not set".to_string();
        }
        if k.len() <= 8 {
            return "••••".to_string();
        }
        format!("{}…{}", &k[..4], &k[k.len() - 4..])
    }
}

pub static LLM_CONFIG: GlobalSignal<LlmConfig> = Signal::global(LlmConfig::default);
pub static LLM_SETTINGS_OPEN: GlobalSignal<bool> = Signal::global(|| false);

pub fn load_llm_config() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(storage) = window.local_storage().ok().flatten() else {
        return;
    };
    let Ok(Some(raw)) = storage.get_item(STORAGE_KEY) else {
        return;
    };
    if let Ok(cfg) = serde_json::from_str::<LlmConfig>(&raw) {
        *LLM_CONFIG.write() = cfg;
    }
}

pub fn save_llm_config(cfg: &LlmConfig) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(storage) = window.local_storage().ok().flatten() else {
        return;
    };
    if let Ok(raw) = serde_json::to_string(cfg) {
        let _ = storage.set_item(STORAGE_KEY, &raw);
    }
    *LLM_CONFIG.write() = cfg.clone();
}

pub fn clear_llm_api_key() {
    let mut cfg = LLM_CONFIG.read().clone();
    cfg.api_key.clear();
    save_llm_config(&cfg);
}
