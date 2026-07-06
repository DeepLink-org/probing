//! Load diagnostic skills embedded at compile time from repo ``skills/``,
//! merged with user/project skill directories at runtime.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use include_dir::{include_dir, Dir};
use serde::Deserialize;

static SKILLS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../skills");
const CATALOG_YAML: &str = include_str!("../../../../../skills/catalog.yaml");

#[derive(Debug, Clone, Deserialize)]
struct CatalogFile {
    #[serde(default)]
    skills: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CatalogEntry {
    pub id: String,
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
    #[serde(default)]
    category: String,
    #[serde(default)]
    docs: String,
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
pub struct SkillStep {
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
pub struct Skill {
    pub id: String,
    pub title: String,
    pub category: String,
    pub docs: String,
    pub parameters: Vec<SkillParameter>,
    pub steps: Vec<SkillStep>,
    pub interpretation: Vec<InterpretRule>,
    pub summary_template: String,
    pub next_steps: Vec<String>,
}

fn catalog_entries() -> Vec<CatalogEntry> {
    let mut merged: HashMap<String, CatalogEntry> = HashMap::new();
    for entry in embedded_catalog_entries() {
        merged.insert(entry.id.clone(), entry);
    }
    for root in fs_skill_roots() {
        if let Ok(entries) = load_fs_catalog(&root) {
            for entry in entries {
                merged.insert(entry.id.clone(), entry);
            }
        }
    }
    let mut out: Vec<CatalogEntry> = merged.into_values().collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

fn embedded_catalog_entries() -> Vec<CatalogEntry> {
    let file: CatalogFile =
        serde_yaml::from_str(CATALOG_YAML).unwrap_or(CatalogFile { skills: vec![] });
    file.skills
}

fn home_skills_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(|home| PathBuf::from(home).join(".probing/skills"))
}

fn project_skills_dir() -> Option<PathBuf> {
    let cwd = env::current_dir().ok()?;
    for ancestor in cwd.ancestors() {
        let candidate = ancestor.join(".probing/skills");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    None
}

fn env_skills_dir() -> Option<PathBuf> {
    env::var("PROBING_SKILLS_DIR")
        .ok()
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

fn fs_skill_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(dir) = home_skills_dir() {
        if dir.is_dir() {
            roots.push(dir);
        }
    }
    if let Some(dir) = project_skills_dir() {
        if !roots.iter().any(|r| r == &dir) {
            roots.push(dir);
        }
    }
    if let Some(dir) = env_skills_dir() {
        if !roots.iter().any(|r| r == &dir) {
            roots.push(dir);
        }
    }
    roots
}

fn load_fs_catalog(root: &Path) -> Result<Vec<CatalogEntry>> {
    let catalog_path = root.join("catalog.yaml");
    if !catalog_path.is_file() {
        return Ok(vec![]);
    }
    let text = fs::read_to_string(&catalog_path)
        .with_context(|| format!("read {}", catalog_path.display()))?;
    let file: CatalogFile = serde_yaml::from_str(&text)?;
    Ok(file.skills)
}

fn fs_steps_path(root: &Path, id: &str) -> Option<PathBuf> {
    if let Ok(entries) = load_fs_catalog(root) {
        if let Some(entry) = entries.iter().find(|e| e.id == id) {
            let rel = entry_path(entry);
            let path = root.join(&rel);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    let direct = root.join(id).join("steps.yaml");
    if direct.is_file() {
        return Some(direct);
    }
    None
}

fn load_fs_steps_yaml(id: &str) -> Option<String> {
    for root in fs_skill_roots().into_iter().rev() {
        if let Some(path) = fs_steps_path(&root, id) {
            if let Ok(text) = fs::read_to_string(path) {
                return Some(text);
            }
        }
    }
    None
}

fn entry_path(entry: &CatalogEntry) -> String {
    if !entry.path.is_empty() {
        entry.path.clone()
    } else {
        entry.file.clone()
    }
}

pub fn list_skill_ids() -> Vec<String> {
    catalog_entries().into_iter().map(|e| e.id).collect()
}

fn embedded_steps_yaml(id: &str) -> Option<&'static str> {
    let entry = embedded_catalog_entries()
        .into_iter()
        .find(|e| e.id == id)?;
    let rel = entry_path(&entry);
    SKILLS.get_file(&rel).and_then(|f| f.contents_utf8())
}

pub fn load_skill(id: &str) -> Result<Skill> {
    let yaml = load_fs_steps_yaml(id)
        .or_else(|| embedded_steps_yaml(id).map(|s| s.to_string()))
        .ok_or_else(|| anyhow!("Unknown skill: {id}"))?;
    let file: SkillFile = serde_yaml::from_str(&yaml)?;
    let steps = file
        .spec
        .steps
        .into_iter()
        .map(|s| SkillStep {
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
    Ok(Skill {
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
    let nccl_proxy = if use_global {
        "global.nccl.proxy_ops".to_string()
    } else {
        "nccl.proxy_ops".to_string()
    };
    let nccl_coll = if use_global {
        "global.nccl.coll_perf".to_string()
    } else {
        "nccl.coll_perf".to_string()
    };
    let nccl_inflight = if use_global {
        "global.nccl.inflight_ops".to_string()
    } else {
        "nccl.inflight_ops".to_string()
    };
    let net_qp = if use_global {
        "global.nccl.net_qp".to_string()
    } else {
        "nccl.net_qp".to_string()
    };
    let nccl_counters = if use_global {
        "global.nccl.profiler_counters".to_string()
    } else {
        "nccl.profiler_counters".to_string()
    };
    let fr = if use_global {
        "global.python.torch_nccl_flight_record".to_string()
    } else {
        "python.torch_nccl_flight_record".to_string()
    };
    let fr_status = if use_global {
        "global.python.torch_nccl_pg_status".to_string()
    } else {
        "python.torch_nccl_pg_status".to_string()
    };
    let mut out = HashMap::new();
    out.insert("comm_table".to_string(), comm.clone());
    out.insert("table_comm".to_string(), comm);
    out.insert("nccl_proxy_table".to_string(), nccl_proxy);
    out.insert("nccl_coll_table".to_string(), nccl_coll);
    out.insert("nccl_inflight_table".to_string(), nccl_inflight);
    out.insert("net_qp_table".to_string(), net_qp);
    out.insert("nccl_counters_table".to_string(), nccl_counters);
    out.insert("fr_table".to_string(), fr);
    out.insert("fr_status_table".to_string(), fr_status);
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

pub fn build_context(pb: &Skill, overrides: &HashMap<String, String>) -> HashMap<String, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn normalize_sql(sql: &str) -> String {
        sql.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn slow_rank_rank_latency_sql_golden() {
        let skill = load_skill("slow_rank").expect("slow_rank skill");
        let overrides = HashMap::from([
            ("use_global".to_string(), "false".to_string()),
            ("step_window".to_string(), "5".to_string()),
        ]);
        let ctx = build_context(&skill, &overrides);
        let step = skill
            .steps
            .iter()
            .find(|s| s.id == "rank_latency")
            .expect("rank_latency step");
        let sql = expand_template(step.sql.as_ref().expect("sql"), &ctx);
        let normalized = normalize_sql(&sql);
        assert!(normalized.contains("FROM python.comm_collective"));
        assert!(!normalized.contains("global.python.comm_collective"));
        assert!(normalized.contains("- 5"));
    }

    #[test]
    fn slow_rank_rank_latency_global_sql_golden() {
        let skill = load_skill("slow_rank").expect("slow_rank skill");
        let overrides = HashMap::from([
            ("use_global".to_string(), "true".to_string()),
            ("step_window".to_string(), "10".to_string()),
        ]);
        let ctx = build_context(&skill, &overrides);
        let step = skill
            .steps
            .iter()
            .find(|s| s.id == "rank_latency")
            .expect("rank_latency step");
        let sql = expand_template(step.sql.as_ref().expect("sql"), &ctx);
        let normalized = normalize_sql(&sql);
        assert!(normalized.contains("FROM global.python.comm_collective"));
        assert!(normalized.contains("- 10"));
    }

    #[test]
    fn watchdog_timeout_flight_recorder_table_expansion() {
        let skill = load_skill("watchdog_timeout").expect("watchdog_timeout skill");
        let overrides = HashMap::from([
            ("use_global".to_string(), "false".to_string()),
            ("seq_window".to_string(), "7".to_string()),
        ]);
        let ctx = build_context(&skill, &overrides);
        let step = skill
            .steps
            .iter()
            .find(|s| s.id == "collective_alignment")
            .expect("collective_alignment step");
        let sql = expand_template(step.sql.as_ref().expect("sql"), &ctx);
        let normalized = normalize_sql(&sql);
        assert!(normalized.contains("FROM python.torch_nccl_flight_record"));
        assert!(!normalized.contains("global.python.torch_nccl_flight_record"));
        assert!(normalized.contains("- 7"));

        let overrides = HashMap::from([
            ("use_global".to_string(), "true".to_string()),
            ("seq_window".to_string(), "11".to_string()),
        ]);
        let ctx = build_context(&skill, &overrides);
        let sql = expand_template(step.sql.as_ref().expect("sql"), &ctx);
        let normalized = normalize_sql(&sql);
        assert!(normalized.contains("FROM global.python.torch_nccl_flight_record"));
        assert!(normalized.contains("- 11"));
    }

    #[test]
    fn comm_bottleneck_expands_nccl_coll_perf() {
        let skill = load_skill("comm_bottleneck").expect("comm_bottleneck skill");
        let overrides = HashMap::from([("use_global".to_string(), "false".to_string())]);
        let ctx = build_context(&skill, &overrides);
        let step = skill
            .steps
            .iter()
            .find(|s| s.id == "nccl_coll_bw")
            .expect("nccl_coll_bw step");
        let sql = expand_template(step.sql.as_ref().expect("sql"), &ctx);
        let normalized = normalize_sql(&sql);
        assert!(normalized.contains("FROM nccl.coll_perf"));
        assert!(normalized.contains("timing_source"));

        let overrides = HashMap::from([("use_global".to_string(), "true".to_string())]);
        let ctx = build_context(&skill, &overrides);
        let sql = expand_template(step.sql.as_ref().expect("sql"), &ctx);
        assert!(normalize_sql(&sql).contains("FROM global.nccl.coll_perf"));
    }

    #[test]
    fn sre_triage_expands_operational_tables() {
        let skill = load_skill("sre_triage").expect("sre_triage skill");
        let overrides = HashMap::from([
            ("use_global".to_string(), "true".to_string()),
            ("seq_window".to_string(), "13".to_string()),
        ]);
        let ctx = build_context(&skill, &overrides);
        let sql = skill
            .steps
            .iter()
            .filter_map(|s| s.sql.as_ref())
            .map(|sql| expand_template(sql, &ctx))
            .collect::<Vec<_>>()
            .join("\n");
        let normalized = normalize_sql(&sql);
        assert!(normalized.contains("global.python.comm_collective"));
        assert!(normalized.contains("global.python.torch_nccl_flight_record"));
        assert!(normalized.contains("global.nccl.proxy_ops"));
        assert!(normalized.contains("- 13"));
        assert!(!normalized.contains("{fr_table}"));
        assert!(!normalized.contains("{nccl_proxy_table}"));
    }
}
