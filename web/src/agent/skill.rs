//! Diagnostic skill definitions loaded at compile time from repo ``skills/``.

use include_dir::{include_dir, Dir};
use serde::Deserialize;
use std::collections::HashMap;

static SKILLS_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../skills");
const CATALOG_YAML: &str = include_str!("../../../skills/catalog.yaml");

#[derive(Debug, Clone, Deserialize)]
struct CatalogFile {
    #[serde(default)]
    skills: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct CatalogEntry {
    id: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    file: String,
}

#[derive(Debug, Clone, Deserialize)]
struct SkillFile {
    metadata: SkillMeta,
    spec: SkillSpec,
}

#[derive(Debug, Clone, Deserialize)]
struct SkillMeta {
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
struct SkillSpec {
    #[serde(default)]
    parameters: Vec<SkillParameter>,
    #[serde(default)]
    steps: Vec<SkillStepRaw>,
    #[serde(default)]
    interpretation: InterpretationSpec,
    #[serde(default)]
    summary_template: String,
    #[serde(default)]
    next_steps: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct InterpretationSpec {
    #[serde(default)]
    rules: Vec<InterpretRuleRaw>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InterpretRuleRaw {
    pub id: String,
    pub when: String,
    #[serde(default = "default_severity")]
    pub severity: String,
    pub message: String,
}

fn default_severity() -> String {
    "info".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct SkillParameter {
    pub name: String,
    #[serde(default)]
    pub default: serde_yaml::Value,
}

#[derive(Debug, Clone, Deserialize)]
struct SkillStepRaw {
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
    #[serde(default)]
    cluster: Option<bool>,
}

fn default_step_type() -> String {
    "sql".to_string()
}

fn default_on_empty() -> String {
    "skip".to_string()
}

#[derive(Debug, Clone)]
pub struct SkillStep {
    pub id: String,
    pub title: String,
    pub step_type: String,
    pub sql: Option<String>,
    #[allow(dead_code)]
    pub method: Option<String>,
    pub path: Option<String>,
    #[allow(dead_code)]
    pub action: Option<String>,
    pub view: Option<String>,
    pub on_empty: String,
    pub empty_message: Option<String>,
    pub when: Option<String>,
    pub cluster: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct Skill {
    pub id: String,
    pub title: String,
    pub category: String,
    #[allow(dead_code)]
    pub tags: Vec<String>,
    pub docs: String,
    pub parameters: Vec<SkillParameter>,
    pub steps: Vec<SkillStep>,
    pub interpretation: Vec<InterpretRuleRaw>,
    pub summary_template: String,
    pub next_steps: Vec<String>,
    keywords: Vec<String>,
}

fn catalog_entries() -> Vec<CatalogEntry> {
    let file: CatalogFile =
        serde_yaml::from_str(CATALOG_YAML).unwrap_or(CatalogFile { skills: vec![] });
    file.skills
}

fn entry_path(entry: &CatalogEntry) -> String {
    if !entry.path.is_empty() {
        entry.path.clone()
    } else {
        entry.file.clone()
    }
}

fn steps_yaml_for_id(id: &str) -> Option<&'static str> {
    let entry = catalog_entries().into_iter().find(|e| e.id == id)?;
    let rel = entry_path(&entry);
    SKILLS_DIR.get_file(&rel).and_then(|f| f.contents_utf8())
}

pub fn list_skill_ids() -> Vec<&'static str> {
    catalog_entries()
        .into_iter()
        .filter(|e| steps_yaml_for_id(&e.id).is_some())
        .map(|e| leak_str(&e.id))
        .collect()
}

fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

pub fn load_skill(id: &str) -> Option<Skill> {
    let yaml = steps_yaml_for_id(id)?;
    let file: SkillFile = serde_yaml::from_str(yaml).ok()?;
    let mut keywords: Vec<String> = file
        .metadata
        .tags
        .iter()
        .map(|t| t.to_lowercase())
        .collect();
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
        .map(|s| SkillStep {
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
            cluster: s.cluster,
        })
        .collect();
    Some(Skill {
        id: file.metadata.id,
        title: file.metadata.title,
        category: file.metadata.category,
        tags: file.metadata.tags,
        docs: file.metadata.docs.trim().to_string(),
        parameters: file.spec.parameters,
        steps,
        interpretation: file.spec.interpretation.rules,
        summary_template: file.spec.summary_template.trim().to_string(),
        next_steps: file.spec.next_steps,
        keywords,
    })
}

pub fn default_parameters(pb: &Skill) -> HashMap<String, String> {
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

pub fn build_context(pb: &Skill, overrides: &HashMap<String, String>) -> HashMap<String, String> {
    let mut ctx = default_parameters(pb);
    ctx.extend(derive_variables(&ctx));
    for (k, v) in overrides {
        ctx.insert(k.clone(), v.clone());
    }
    ctx.extend(derive_variables(&ctx));
    ctx
}

pub fn match_skills(query: &str, limit: usize) -> Vec<String> {
    use std::collections::HashMap;

    let q = query.to_lowercase();
    let mut scored: HashMap<String, usize> = HashMap::new();

    for (rank, id) in super::routing::match_intents(query, 10)
        .into_iter()
        .enumerate()
    {
        *scored.entry(id).or_insert(0) += 3usize.saturating_mul(10 - rank);
    }

    for id in list_skill_ids() {
        let Some(pb) = load_skill(id) else {
            continue;
        };
        let keyword_hits = pb
            .keywords
            .iter()
            .filter(|kw| q.contains(kw.as_str()))
            .count();
        let id_hit = q.contains(&pb.id.replace('_', " ")) || q.contains(&pb.id);
        if keyword_hits > 0 || id_hit {
            *scored.entry(pb.id).or_insert(0) += keyword_hits.max(1);
        }
    }

    let mut ranked: Vec<(usize, String)> =
        scored.into_iter().map(|(id, score)| (score, id)).collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    ranked.into_iter().take(limit).map(|(_, id)| id).collect()
}

pub fn resolve_skill_id(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.starts_with('/') {
        return load_skill(trimmed.trim_start_matches('/')).map(|p| p.id);
    }
    if let Some(rest) = trimmed.strip_prefix("run ") {
        return load_skill(rest.trim()).map(|p| p.id);
    }
    if load_skill(trimmed).is_some() {
        return Some(trimmed.to_string());
    }
    let matched = match_skills(trimmed, 1);
    matched.into_iter().next()
}
