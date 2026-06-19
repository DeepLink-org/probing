//! Load diagnostic playbooks embedded at compile time from ``playbooks/``.

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use include_dir::{include_dir, Dir};
use serde::Deserialize;

static DIAGNOSTICS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../playbooks/diagnostics");
const CATALOG_YAML: &str = include_str!("../../../../../playbooks/catalog.yaml");

#[derive(Debug, Clone, Deserialize)]
struct CatalogFile {
    #[serde(default)]
    playbooks: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CatalogEntry {
    pub id: String,
    pub file: String,
}

#[derive(Debug, Clone, Deserialize)]
struct PlaybookFile {
    metadata: PlaybookMeta,
    spec: PlaybookSpec,
}

#[derive(Debug, Clone, Deserialize)]
struct PlaybookMeta {
    id: String,
    title: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    docs: String,
}

#[derive(Debug, Clone, Deserialize)]
struct PlaybookSpec {
    #[serde(default)]
    parameters: Vec<PlaybookParameter>,
    #[serde(default)]
    steps: Vec<PlaybookStepRaw>,
    #[serde(default)]
    interpretation: InterpretationSpec,
    #[serde(default)]
    summary_template: String,
    #[serde(default)]
    next_steps: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaybookParameter {
    pub name: String,
    #[serde(default)]
    pub default: serde_yaml::Value,
}

#[derive(Debug, Clone, Deserialize)]
struct PlaybookStepRaw {
    id: String,
    title: String,
    #[serde(rename = "type", default = "default_step_type")]
    step_type: String,
    #[serde(default)]
    sql: Option<String>,
    #[serde(default, rename = "method")]
    _method: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    view: Option<String>,
    #[serde(default = "default_on_empty")]
    on_empty: String,
    #[serde(default)]
    empty_message: Option<String>,
    #[serde(default)]
    when: Option<String>,
    #[serde(default)]
    cluster: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct InterpretationSpec {
    #[serde(default)]
    rules: Vec<InterpretRule>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InterpretRule {
    pub id: String,
    pub when: String,
    #[serde(default = "default_severity")]
    pub severity: String,
    pub message: String,
}

fn default_severity() -> String {
    "info".to_string()
}

fn default_step_type() -> String {
    "sql".to_string()
}

fn default_on_empty() -> String {
    "skip".to_string()
}

#[derive(Debug, Clone)]
pub struct PlaybookStep {
    pub id: String,
    pub title: String,
    pub step_type: String,
    pub sql: Option<String>,
    pub path: Option<String>,
    pub view: Option<String>,
    pub on_empty: String,
    pub empty_message: Option<String>,
    pub when: Option<String>,
    pub cluster: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct Playbook {
    pub id: String,
    pub title: String,
    pub category: String,
    pub docs: String,
    pub parameters: Vec<PlaybookParameter>,
    pub steps: Vec<PlaybookStep>,
    pub interpretation: Vec<InterpretRule>,
    pub summary_template: String,
    pub next_steps: Vec<String>,
}

pub fn catalog_entries() -> Vec<CatalogEntry> {
    serde_yaml::from_str::<CatalogFile>(CATALOG_YAML)
        .map(|c| c.playbooks)
        .unwrap_or_default()
}

pub fn list_playbook_ids() -> Vec<String> {
    catalog_entries().into_iter().map(|e| e.id).collect()
}

fn diagnostics_yaml(id: &str) -> Option<&'static str> {
    let entry = catalog_entries().into_iter().find(|e| e.id == id)?;
    let file_name = entry.file.rsplit('/').next().unwrap_or(&entry.file);
    DIAGNOSTICS
        .get_file(file_name)
        .and_then(|f| f.contents_utf8())
}

pub fn load_playbook(id: &str) -> Result<Playbook> {
    let yaml = diagnostics_yaml(id).ok_or_else(|| anyhow!("Unknown playbook: {id}"))?;
    let file: PlaybookFile = serde_yaml::from_str(yaml)?;
    let steps = file
        .spec
        .steps
        .into_iter()
        .map(|s| PlaybookStep {
            id: s.id,
            title: s.title,
            step_type: s.step_type,
            sql: s.sql,
            path: s.path,
            view: s.view,
            on_empty: s.on_empty,
            empty_message: s.empty_message,
            when: s.when,
            cluster: s.cluster,
        })
        .collect();
    Ok(Playbook {
        id: file.metadata.id,
        title: file.metadata.title,
        category: file.metadata.category,
        docs: file.metadata.docs.trim().to_string(),
        parameters: file.spec.parameters,
        steps,
        interpretation: file.spec.interpretation.rules,
        summary_template: file.spec.summary_template.trim().to_string(),
        next_steps: file.spec.next_steps,
    })
}

pub fn default_parameters(pb: &Playbook) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for p in &pb.parameters {
        let val = match &p.default {
            serde_yaml::Value::Number(n) => n.to_string(),
            serde_yaml::Value::Bool(b) => b.to_string(),
            serde_yaml::Value::String(s) => s.clone(),
            _ => continue,
        };
        out.insert(p.name.clone(), val);
    }
    out
}

pub fn derive_variables(params: &HashMap<String, String>) -> HashMap<String, String> {
    let use_global = params
        .get("use_global")
        .map(|v| v == "true")
        .unwrap_or(false);
    let comm = if use_global {
        "global.python.comm_collective".to_string()
    } else {
        "python.comm_collective".to_string()
    };
    let mut out = HashMap::new();
    out.insert("comm_table".to_string(), comm.clone());
    out.insert("table_comm".to_string(), comm);
    out.insert(
        "global_prefix".to_string(),
        if use_global {
            "global.".to_string()
        } else {
            String::new()
        },
    );
    out
}

pub fn build_context(
    pb: &Playbook,
    overrides: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut ctx = default_parameters(pb);
    ctx.extend(derive_variables(&ctx));
    for (k, v) in overrides {
        ctx.insert(k.clone(), v.clone());
    }
    ctx.extend(derive_variables(&ctx));
    ctx
}

pub fn expand_template(template: &str, ctx: &HashMap<String, String>) -> String {
    let mut out = template.to_string();
    for (key, val) in ctx {
        out = out.replace(&format!("{{{key}}}"), val);
    }
    out
}
