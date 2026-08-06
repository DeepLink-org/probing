//! Load diagnostic skills from discovered roots (Python packages + overrides).

use std::collections::HashMap;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use super::catalog;
use super::discovery;

pub use discovery::all_skill_root_paths;

pub use catalog::CatalogEntry;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillFile {
    #[serde(rename = "apiVersion")]
    api_version: String,
    kind: String,
    metadata: SkillMeta,
    spec: SkillSpec,
}

#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeywordsSpec {
    #[serde(default)]
    pub zh: Vec<String>,
    #[serde(default)]
    pub en: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TriggersSpec {
    #[serde(default)]
    keywords: KeywordsSpec,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillMeta {
    id: String,
    title: String,
    #[serde(default, rename = "title_en")]
    _title_en: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    docs: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    triggers: TriggersSpec,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(default)]
    variables: HashMap<String, String>,
    #[serde(default)]
    requires: RequiresSpec,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SkillParameterType {
    Integer,
    Number,
    Boolean,
    #[default]
    String,
}

impl SkillParameterType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Integer => "integer",
            Self::Number => "number",
            Self::Boolean => "boolean",
            Self::String => "string",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillParameter {
    pub name: String,
    #[serde(rename = "type")]
    pub parameter_type: SkillParameterType,
    #[serde(default)]
    pub default: serde_yaml::Value,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillStepRaw {
    id: String,
    title: String,
    #[serde(rename = "type", default = "default_step_type")]
    step_type: String,
    #[serde(default)]
    sql: Option<String>,
    #[serde(default, rename = "method")]
    method: Option<String>,
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
    #[serde(default)]
    platform: Option<SkillPlatform>,
    #[serde(default)]
    action: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct InterpretationSpec {
    #[serde(default)]
    rules: Vec<InterpretRule>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterpretRule {
    pub id: String,
    pub when: String,
    #[serde(default = "default_severity")]
    pub severity: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequiresSpec {
    #[serde(default)]
    pub any_tables: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SkillPlatform {
    Linux,
    Macos,
    Windows,
}

impl SkillPlatform {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Macos => "macos",
            Self::Windows => "windows",
        }
    }

    pub fn is_current(self) -> bool {
        self.as_str() == std::env::consts::OS
    }
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
    pub method: Option<String>,
    pub path: Option<String>,
    pub view: Option<String>,
    pub on_empty: String,
    pub empty_message: Option<String>,
    pub when: Option<String>,
    pub cluster: Option<bool>,
    pub platform: Option<SkillPlatform>,
    pub action: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Skill {
    pub id: String,
    pub title: String,
    pub category: String,
    pub docs: String,
    pub tags: Vec<String>,
    pub keywords: Vec<String>,
    pub trigger_keywords: KeywordsSpec,
    pub parameters: Vec<SkillParameter>,
    pub steps: Vec<SkillStep>,
    pub interpretation: Vec<InterpretRule>,
    pub summary_template: String,
    pub next_steps: Vec<String>,
    pub variables: HashMap<String, String>,
    pub requires: RequiresSpec,
}

impl Skill {
    pub fn routing_keywords_json(&self) -> KeywordsSpec {
        self.trigger_keywords.clone()
    }
}

fn catalog_entries() -> Vec<CatalogEntry> {
    catalog::load_catalog()
}

pub fn list_skill_ids() -> Vec<String> {
    catalog_entries().into_iter().map(|e| e.id).collect()
}

pub fn load_skill(id: &str) -> Result<Skill> {
    let yaml = discovery::load_fs_steps_yaml(id).ok_or_else(|| anyhow!("Unknown skill: {id}"))?;
    let file: SkillFile =
        serde_yaml::from_str(&yaml).with_context(|| format!("invalid skill schema for `{id}`"))?;
    validate_skill_file(id, &file)?;
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
            view: s.view,
            on_empty: s.on_empty,
            empty_message: s.empty_message,
            when: s.when,
            cluster: s.cluster,
            platform: s.platform,
            action: s.action,
        })
        .collect();
    let keywords = collect_keywords(&file.metadata);
    let skill = Skill {
        id: file.metadata.id,
        title: file.metadata.title,
        category: file.metadata.category,
        docs: file.metadata.docs.trim().to_string(),
        tags: file.metadata.tags,
        keywords,
        trigger_keywords: file.metadata.triggers.keywords,
        parameters: file.spec.parameters,
        steps,
        interpretation: file.spec.interpretation.rules,
        summary_template: file.spec.summary_template.trim().to_string(),
        next_steps: file.spec.next_steps,
        variables: file.spec.variables,
        requires: file.spec.requires,
    };
    validate_skill_contract(&skill)?;
    Ok(skill)
}

fn validate_default_sql(skill: &Skill) -> Result<()> {
    let context = build_context(skill, &HashMap::new());
    for step in &skill.steps {
        let Some(sql) = &step.sql else { continue };
        let expanded = expand_template(sql, &context);
        if contains_template_placeholder(&expanded) {
            bail!("step `{}` contains an unresolved SQL template", step.id);
        }
        crate::sql_guard::ensure_read_only_sql(&expanded)
            .map_err(|error| anyhow!("step `{}`: {error}", step.id))?;
    }
    Ok(())
}

fn contains_template_placeholder(value: &str) -> bool {
    let mut rest = value;
    while let Some(open) = rest.find('{') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find('}') else {
            return false;
        };
        let name = &rest[..close];
        if !name.is_empty()
            && name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            return true;
        }
        rest = &rest[close + 1..];
    }
    false
}

fn validate_skill_file(requested_id: &str, file: &SkillFile) -> Result<()> {
    if file.api_version != "probing.dev/v1" {
        bail!("unsupported apiVersion `{}`", file.api_version);
    }
    if file.kind != "Skill" {
        bail!("expected kind `Skill`, got `{}`", file.kind);
    }
    if file.metadata.id != requested_id {
        bail!(
            "skill id `{}` does not match requested id `{requested_id}`",
            file.metadata.id
        );
    }

    Ok(())
}

pub(crate) fn validate_skill_contract(skill: &Skill) -> Result<()> {
    let mut parameter_names = std::collections::HashSet::new();
    for parameter in &skill.parameters {
        if !parameter_names.insert(parameter.name.as_str()) {
            bail!("duplicate parameter id `{}`", parameter.name);
        }
        validate_parameter_default(parameter)?;
    }

    let mut step_ids = std::collections::HashSet::new();
    for step in &skill.steps {
        if !step_ids.insert(step.id.as_str()) {
            bail!("duplicate step id `{}`", step.id);
        }
        if !matches!(step.step_type.as_str(), "sql" | "api" | "ui" | "config") {
            bail!(
                "step `{}` has unsupported type `{}`",
                step.id,
                step.step_type
            );
        }
        if !matches!(step.on_empty.as_str(), "skip" | "warn" | "abort") {
            bail!(
                "step `{}` has invalid on_empty `{}`",
                step.id,
                step.on_empty
            );
        }
        match step.step_type.as_str() {
            "sql" if step.sql.as_ref().is_none_or(|sql| sql.trim().is_empty()) => {
                bail!("SQL step `{}` is missing sql", step.id)
            }
            "api" if step.path.as_ref().is_none_or(|path| path.trim().is_empty()) => {
                bail!("API step `{}` is missing path", step.id)
            }
            "api"
                if step
                    .method
                    .as_deref()
                    .is_some_and(|method| !method.eq_ignore_ascii_case("GET")) =>
            {
                bail!("API step `{}` only supports method GET", step.id)
            }
            "ui" if step.view.as_ref().is_none_or(|view| view.trim().is_empty()) => {
                bail!("UI step `{}` is missing view", step.id)
            }
            _ => {}
        }
    }

    let mut rule_ids = std::collections::HashSet::new();
    for rule in &skill.interpretation {
        if !rule_ids.insert(rule.id.as_str()) {
            bail!("duplicate interpretation rule id `{}`", rule.id);
        }
        if !matches!(rule.severity.as_str(), "error" | "warning" | "info") {
            bail!(
                "rule `{}` has invalid severity `{}`",
                rule.id,
                rule.severity
            );
        }
        crate::interpret::validate_rule_expression(&rule.when)
            .map_err(|error| anyhow!("rule `{}`: {error}", rule.id))?;
    }
    validate_default_sql(skill)
}

fn validate_parameter_default(parameter: &SkillParameter) -> Result<()> {
    let valid = match parameter.parameter_type {
        SkillParameterType::Integer => parameter.default.as_i64().is_some(),
        SkillParameterType::Number => parameter.default.as_f64().is_some(),
        SkillParameterType::Boolean => parameter.default.as_bool().is_some(),
        SkillParameterType::String => parameter.default.as_str().is_some(),
    };
    if valid {
        Ok(())
    } else {
        bail!(
            "parameter `{}` default does not match type `{}`",
            parameter.name,
            parameter.parameter_type.as_str()
        )
    }
}

/// Validate and normalize user-supplied parameter values before expansion.
pub fn normalize_parameter_overrides(
    skill: &Skill,
    overrides: &mut HashMap<String, String>,
) -> Result<()> {
    for (name, value) in overrides.iter_mut() {
        let parameter = skill
            .parameters
            .iter()
            .find(|parameter| parameter.name == *name)
            .ok_or_else(|| anyhow!("unknown parameter `{name}` for skill `{}`", skill.id))?;
        let normalized = match parameter.parameter_type {
            SkillParameterType::Integer => value
                .parse::<i64>()
                .map(|value| value.to_string())
                .map_err(|_| anyhow!("parameter `{name}` must be an integer"))?,
            SkillParameterType::Number => {
                let number = value
                    .parse::<f64>()
                    .map_err(|_| anyhow!("parameter `{name}` must be a number"))?;
                if !number.is_finite() {
                    bail!("parameter `{name}` must be a finite number");
                }
                number.to_string()
            }
            SkillParameterType::Boolean => match value.to_ascii_lowercase().as_str() {
                "true" => "true".to_string(),
                "false" => "false".to_string(),
                _ => bail!("parameter `{name}` must be true or false"),
            },
            SkillParameterType::String => value.clone(),
        };
        *value = normalized;
    }
    Ok(())
}

fn collect_keywords(meta: &SkillMeta) -> Vec<String> {
    let mut words: Vec<String> = meta.tags.iter().map(|t| t.to_lowercase()).collect();
    for kw in meta
        .triggers
        .keywords
        .zh
        .iter()
        .chain(meta.triggers.keywords.en.iter())
    {
        words.push(kw.to_lowercase());
    }
    words
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
    // String parameters are authored as SQL literal contents (for example
    // `stage = '{stage_filter}'`). Escape quotes before template expansion.
    for parameter in &pb.parameters {
        if parameter.parameter_type == SkillParameterType::String {
            if let Some(value) = ctx.get_mut(&parameter.name) {
                *value = value.replace('\'', "''");
            }
        }
    }
    for (key, template) in &pb.variables {
        let expanded = expand_template(template, &ctx);
        ctx.insert(key.clone(), expanded);
    }
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
    fn every_catalog_skill_compiles() {
        for id in list_skill_ids() {
            load_skill(&id).unwrap_or_else(|error| panic!("skill {id}: {error:#}"));
        }
    }

    #[test]
    fn parameter_overrides_are_typed_and_normalized() {
        let skill = load_skill("slow_rank").expect("slow_rank skill");
        let mut valid = HashMap::from([
            ("step_window".to_string(), "0042".to_string()),
            ("use_global".to_string(), "TRUE".to_string()),
        ]);
        normalize_parameter_overrides(&skill, &mut valid).expect("valid overrides");
        assert_eq!(valid["step_window"], "42");
        assert_eq!(valid["use_global"], "true");

        let mut invalid = HashMap::from([("step_window".to_string(), "1; SELECT 1".to_string())]);
        assert!(normalize_parameter_overrides(&skill, &mut invalid).is_err());
        let mut unknown = HashMap::from([("typo".to_string(), "1".to_string())]);
        assert!(normalize_parameter_overrides(&skill, &mut unknown).is_err());

        let string_skill = load_skill("module_bottleneck").expect("module_bottleneck skill");
        let string_overrides =
            HashMap::from([("stage_filter".to_string(), "x' OR '1'='1".to_string())]);
        let context = build_context(&string_skill, &string_overrides);
        assert_eq!(context["stage_filter"], "x'' OR ''1''=''1");
    }

    #[test]
    fn yaml_schema_rejects_unknown_fields() {
        let yaml = r#"
apiVersion: probing.dev/v1
kind: Skill
metadata:
  id: strict
  title: Strict
spec:
  parameters:
    - name: limit
      type: integer
      default: 1
      typo: ignored-before
  steps: []
"#;
        let error = serde_yaml::from_str::<SkillFile>(yaml).expect_err("unknown field must fail");
        assert!(error.to_string().contains("unknown field `typo`"));
    }

    #[test]
    fn unresolved_template_detection_ignores_non_template_braces() {
        assert!(contains_template_placeholder("SELECT {missing_value}"));
        assert!(!contains_template_placeholder(
            "SELECT '{\"json\": true}' AS payload"
        ));
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

    #[test]
    fn derive_variables_match_fixture() {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/skill_derived_variables.yaml");
        let raw = std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", fixture_path.display()));
        let doc: serde_yaml::Value = serde_yaml::from_str(&raw).expect("parse fixture yaml");
        let cases = doc
            .get("cases")
            .and_then(|v| v.as_sequence())
            .expect("cases array");

        for case in cases {
            let name = case
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unnamed");
            let params_val = case.get("params").expect("params");
            let mut params = HashMap::new();
            if let Some(map) = params_val.as_mapping() {
                for (k, v) in map {
                    let key = k.as_str().expect("param key");
                    let val = match v {
                        serde_yaml::Value::Bool(b) => b.to_string(),
                        serde_yaml::Value::String(s) => s.clone(),
                        serde_yaml::Value::Number(n) => n.to_string(),
                        _ => panic!("unsupported param value for {name}.{key}"),
                    };
                    params.insert(key.to_string(), val);
                }
            }
            let got = derive_variables(&params);
            let expected = case
                .get("expected")
                .and_then(|v| v.as_mapping())
                .expect("expected map");
            for (k, v) in expected {
                let key = k.as_str().expect("expected key");
                let want = v.as_str().expect("expected value");
                assert_eq!(
                    got.get(key).map(String::as_str),
                    Some(want),
                    "case {name}: {key}"
                );
            }
            assert_eq!(got.len(), expected.len(), "case {name}: extra keys");
        }
    }
}
