//! Extension HTTP routing contract (no `/python/` alias for `pythonext`).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use probing_core::core::{EngineError, ProbeExtension, ProbeExtensionCall, ProbeExtensionManager};

#[derive(Debug)]
struct PythonextStub;

#[async_trait]
impl ProbeExtensionCall for PythonextStub {
    async fn call(
        &self,
        path: &str,
        _params: &HashMap<String, String>,
        _body: &[u8],
    ) -> Result<Vec<u8>, EngineError> {
        Ok(format!("ok:{path}").into_bytes())
    }
}

impl ProbeExtension for PythonextStub {
    fn name(&self) -> String {
        "pythonext".to_string()
    }
}

fn load_spec() -> serde_json::Value {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/spec/api_spec.json");
    let text = std::fs::read_to_string(path).expect("read api_spec.json");
    serde_json::from_str(&text).expect("parse api_spec.json")
}

async fn register_pythonext_stub() -> ProbeExtensionManager {
    let mut manager = ProbeExtensionManager::default();
    manager
        .register(
            "pythonext".to_string(),
            Arc::new(tokio::sync::Mutex::new(PythonextStub)),
        )
        .await;
    manager
}

#[tokio::test]
async fn pythonext_prefix_is_required() {
    let manager = register_pythonext_stub().await;

    let params = HashMap::new();
    let ok = manager
        .call("/pythonext/trace/list", &params, b"")
        .await
        .expect("canonical prefix");
    assert_eq!(ok, b"ok:trace/list");

    let err = manager.call("/python/trace/list", &params, b"").await;
    assert!(err.is_err(), "legacy /python/ prefix must not match");
}

#[tokio::test]
async fn spec_pythonext_paths_route_to_local_suffix() {
    let manager = register_pythonext_stub().await;
    let spec = load_spec();
    let ext = spec["routing"]["python_http_extension_name"]
        .as_str()
        .unwrap();

    let params = HashMap::new();
    for handler in spec["pythonext_handlers"].as_array().unwrap() {
        let local = handler["local_path"].as_str().unwrap();
        let path = format!("/{ext}/{local}");
        let out = manager.call(&path, &params, b"").await.unwrap_or_else(|e| {
            panic!("expected {path} to route: {e}");
        });
        assert_eq!(out, format!("ok:{local}").as_bytes(), "path={path}");
    }
}
