//! Playbook definitions embedded from ``playbooks/diagnostics/*.yaml``.

use serde::Deserialize;
use std::collections::HashMap;

const PLAYBOOK_BLOBS: &[(&str, &str)] = &[
    (
        "health_overview",
        include_str!("../../../playbooks/diagnostics/health_overview.yaml"),
    ),
    (
        "training_hang",
        include_str!("../../../playbooks/diagnostics/training_hang.yaml"),
    ),
    (
        "slow_rank",
        include_str!("../../../playbooks/diagnostics/slow_rank.yaml"),
    ),
    (
        "memory_leak",
        include_str!("../../../playbooks/diagnostics/memory_leak.yaml"),
    ),
    (
        "module_bottleneck",
        include_str!("../../../playbooks/diagnostics/module_bottleneck.yaml"),
    ),
    (
        "comm_bottleneck",
        include_str!("../../../playbooks/diagnostics/comm_bottleneck.yaml"),
    ),
    (
        "gpu_pressure",
        include_str!("../../../playbooks/diagnostics/gpu_pressure.yaml"),
    ),
];

#[derive(Debug, Clone, Deserialize)]
struct PlaybookFile {
    metadata: PlaybookMeta,
    spec: PlaybookSpec,
}

#[derive(Debug, Clone, Deserialize)]
struct PlaybookMeta {
    id: String,
    title: String,
    category: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    triggers: Triggers,
    #[serde(default)]
    docs: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct Triggers {
    keywords: KeywordsMap,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct KeywordsMap {
    #[serde(default)]
    zh: Vec<String>,
    #[serde(default)]
    en: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct PlaybookSpec {
    #[serde(default)]
    parameters: Vec<PlaybookParameter>,
    #[serde(default)]
    steps: Vec<PlaybookStepRaw>,
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
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    view: Option<String>,
    #[serde(default = "default_on_empty")]
    on_empty: String,
    #[serde(default)]
    empty_message: Option<String>,
    #[serde(default)]
    when: Option<String>,
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
    pub method: Option<String>,
    pub path: Option<String>,
    pub action: Option<String>,
    pub view: Option<String>,
    pub on_empty: String,
    pub empty_message: Option<String>,
    pub when: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Playbook {
    pub id: String,
    pub title: String,
    pub category: String,
    pub tags: Vec<String>,
    pub docs: String,
    pub parameters: Vec<PlaybookParameter>,
    pub steps: Vec<PlaybookStep>,
    pub summary_template: String,
    pub next_steps: Vec<String>,
    keywords: Vec<String>,
}

pub fn list_playbook_ids() -> Vec<&'static str> {
    PLAYBOOK_BLOBS.iter().map(|(id, _)| *id).collect()
}

pub fn load_playbook(id: &str) -> Option<Playbook> {
    let yaml = PLAYBOOK_BLOBS.iter().find(|(pid, _)| *pid == id)?.1;
    let file: PlaybookFile = serde_yaml::from_str(yaml).ok()?;
    let mut keywords: Vec<String> = file.metadata.tags.iter().map(|t| t.to_lowercase()).collect();
    keywords.extend(
        file.metadata
            .triggers
            .keywords
            .zh
            .iter()
            .map(|s| s.to_lowercase()),
    );
    keywords.extend(
        file.metadata
            .triggers
            .keywords
            .en
            .iter()
            .map(|s| s.to_lowercase()),
    );
    let steps = file
        .spec
        .steps
        .into_iter()
        .map(|s| PlaybookStep {
            id: s.id,
            title: s.title,
            step_type: s.step_type,
            sql: s.sql,
            method: s.method,
            path: s.path,
            action: s.action,
            view: s.view,
            on_empty: s.on_empty,
            empty_message: s.empty_message,
            when: s.when,
        })
        .collect();
    Some(Playbook {
        id: file.metadata.id,
        title: file.metadata.title,
        category: file.metadata.category,
        tags: file.metadata.tags,
        docs: file.metadata.docs.trim().to_string(),
        parameters: file.spec.parameters,
        steps,
        summary_template: file.spec.summary_template.trim().to_string(),
        next_steps: file.spec.next_steps,
        keywords,
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

pub fn expand_sql(template: &str, ctx: &HashMap<String, String>) -> String {
    let mut out = template.to_string();
    for (key, val) in ctx {
        out = out.replace(&format!("{{{key}}}"), val);
    }
    out
}

pub fn build_context(pb: &Playbook, overrides: &HashMap<String, String>) -> HashMap<String, String> {
    let mut ctx = default_parameters(pb);
    ctx.extend(derive_variables(&ctx));
    for (k, v) in overrides {
        ctx.insert(k.clone(), v.clone());
    }
    ctx.extend(derive_variables(&ctx));
    ctx
}

pub fn match_playbooks(query: &str, limit: usize) -> Vec<String> {
    let q = query.to_lowercase();
    let mut scored: Vec<(usize, String)> = Vec::new();
    for id in list_playbook_ids() {
        let Some(pb) = load_playbook(id) else {
            continue;
        };
        let score = pb.keywords.iter().filter(|kw| q.contains(kw.as_str())).count();
        if score > 0 || q.contains(&pb.id.replace('_', " ")) || q.contains(&pb.id) {
            scored.push((score.max(1), pb.id));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.dedup_by(|a, b| a.1 == b.1);
    scored.into_iter().take(limit).map(|(_, id)| id).collect()
}

pub fn resolve_playbook_id(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.starts_with('/') {
        return load_playbook(trimmed.trim_start_matches('/')).map(|p| p.id);
    }
    if let Some(rest) = trimmed.strip_prefix("run ") {
        return load_playbook(rest.trim()).map(|p| p.id);
    }
    if load_playbook(trimmed).is_some() {
        return Some(trimmed.to_string());
    }
    let matched = match_playbooks(trimmed, 1);
    matched.into_iter().next()
}
