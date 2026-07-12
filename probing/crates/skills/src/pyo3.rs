//! PyO3 bindings for skill discovery (wheel / `_core` module — not L2 collector).

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

use crate::api;
use crate::catalog::{load_intents, load_pages};
use crate::routing::match_skills;
use crate::runner::plan_skill;

fn runtime_err(err: serde_json::Error) -> PyErr {
    PyRuntimeError::new_err(err.to_string())
}

#[pyfunction]
pub fn skills_intents() -> PyResult<String> {
    let value = load_intents().map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    serde_json::to_string(&value).map_err(runtime_err)
}

#[pyfunction]
pub fn skills_pages() -> PyResult<String> {
    let value = load_pages().map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    serde_json::to_string(&value).map_err(runtime_err)
}

#[pyfunction]
pub fn skills_load(id: &str) -> PyResult<String> {
    api::load_skill_json(id).map_err(|e| PyRuntimeError::new_err(e.to_string()))
}

#[pyfunction]
pub fn skills_catalog() -> PyResult<String> {
    api::catalog_json().map_err(|e| PyRuntimeError::new_err(e.to_string()))
}

#[pyfunction]
pub fn skills_routing() -> PyResult<String> {
    api::routing_json().map_err(|e| PyRuntimeError::new_err(e.to_string()))
}

#[pyfunction]
#[pyo3(signature = (query=None, limit=20))]
pub fn skills_list(query: Option<&str>, limit: usize) -> PyResult<String> {
    api::list_skills_json(query, limit).map_err(|e| PyRuntimeError::new_err(e.to_string()))
}

#[pyfunction]
#[pyo3(signature = (skill_id, params_json=None))]
pub fn skills_plan(skill_id: &str, params_json: Option<&str>) -> PyResult<String> {
    let mut overrides = std::collections::HashMap::new();
    if let Some(raw) = params_json.filter(|s| !s.trim().is_empty()) {
        let parsed: serde_json::Value =
            serde_json::from_str(raw).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        if let Some(map) = parsed.as_object() {
            for (k, v) in map {
                overrides.insert(k.clone(), json_scalar_to_string(v));
            }
        }
    }
    let plan = plan_skill(skill_id, overrides).map_err(|e| PyRuntimeError::new_err(e.0))?;
    serde_json::to_string_pretty(&plan).map_err(runtime_err)
}

#[pyfunction]
#[pyo3(signature = (query, limit=3))]
pub fn skills_match(query: &str, limit: usize) -> PyResult<String> {
    let ids = match_skills(query, limit);
    serde_json::to_string(&ids).map_err(runtime_err)
}

fn json_scalar_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Register `skills_*` callables on the probing `_core` module.
pub fn register_skills_bindings(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(skills_load, m)?)?;
    m.add_function(wrap_pyfunction!(skills_catalog, m)?)?;
    m.add_function(wrap_pyfunction!(skills_routing, m)?)?;
    m.add_function(wrap_pyfunction!(skills_list, m)?)?;
    m.add_function(wrap_pyfunction!(skills_plan, m)?)?;
    m.add_function(wrap_pyfunction!(skills_match, m)?)?;
    m.add_function(wrap_pyfunction!(skills_intents, m)?)?;
    m.add_function(wrap_pyfunction!(skills_pages, m)?)?;
    Ok(())
}
