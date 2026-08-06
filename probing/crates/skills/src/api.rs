//! JSON serialization for skill HTTP / MCP discovery APIs.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

use super::catalog::{load_catalog, load_intents, load_pages, CatalogEntry};
use super::loader::{
    load_skill, validate_skill_contract, InterpretRule, KeywordsSpec, RequiresSpec, Skill,
    SkillParameter, SkillParameterType, SkillPlatform, SkillStep,
};
use super::routing::match_skills;

pub fn skill_to_json(skill: &Skill) -> Value {
    let keywords = skill.routing_keywords_json();
    json!({
        "id": skill.id,
        "title": skill.title,
        "category": skill.category,
        "tags": skill.tags,
        "docs": skill.docs,
        "parameters": skill.parameters.iter().map(|p| {
            json!({
                "name": p.name,
                "type": p.parameter_type.as_str(),
                "default": yaml_to_json(&p.default),
                "description": p.description,
            })
        }).collect::<Vec<_>>(),
        "steps": skill.steps.iter().map(step_to_json).collect::<Vec<_>>(),
        "requires": {
            "any_tables": skill.requires.any_tables,
        },
        "interpretation": {
            "rules": skill.interpretation.iter().map(|r| {
                json!({
                    "id": r.id,
                    "when": r.when,
                    "severity": r.severity,
                    "message": r.message,
                })
            }).collect::<Vec<_>>(),
        },
        "summary_template": skill.summary_template,
        "next_steps": skill.next_steps,
        "keywords": {
            "zh": keywords.zh,
            "en": keywords.en,
        },
    })
}

fn step_to_json(step: &SkillStep) -> Value {
    json!({
        "id": step.id,
        "title": step.title,
        "type": step.step_type,
        "sql": step.sql,
        "method": step.method,
        "path": step.path,
        "view": step.view,
        "on_empty": step.on_empty,
        "empty_message": step.empty_message,
        "when": step.when,
        "cluster": step.cluster,
        "platform": step.platform.map(SkillPlatform::as_str),
        "action": step.action,
    })
}

pub fn catalog_entry_to_json(entry: &CatalogEntry) -> Value {
    json!({
        "id": entry.id,
        "path": entry.path,
        "category": entry.category,
        "priority": entry.priority,
        "description": entry.description,
    })
}

pub fn catalog_to_json() -> Result<Value> {
    let skills = load_catalog();
    Ok(json!({
        "skills": skills.iter().map(catalog_entry_to_json).collect::<Vec<_>>(),
    }))
}

pub fn load_skill_json(id: &str) -> Result<String> {
    let skill = load_skill(id)?;
    serde_json::to_string(&skill_to_json(&skill)).context("serialize skill")
}

pub fn skill_from_api(value: &Value) -> Result<Skill> {
    let payload: SkillApiPayload =
        serde_json::from_value(value.clone()).context("deserialize skill")?;
    let skill = payload.into_skill();
    validate_skill_contract(&skill).context("validate skill contract")?;
    Ok(skill)
}

#[derive(Debug, Deserialize)]
struct SkillApiPayload {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default = "default_category")]
    category: String,
    #[serde(default)]
    docs: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    keywords: KeywordsSpec,
    #[serde(default)]
    parameters: Vec<SkillParameterApi>,
    #[serde(default)]
    steps: Vec<SkillStepApi>,
    #[serde(default)]
    requires: RequiresSpec,
    #[serde(default)]
    interpretation: InterpretationApi,
    #[serde(default)]
    summary_template: String,
    #[serde(default)]
    next_steps: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SkillParameterApi {
    name: String,
    #[serde(rename = "type", default)]
    parameter_type: SkillParameterType,
    #[serde(default)]
    default: Value,
    #[serde(default)]
    description: String,
}

#[derive(Debug, Deserialize)]
struct SkillStepApi {
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

#[derive(Debug, Default, Deserialize)]
struct InterpretationApi {
    #[serde(default)]
    rules: Vec<InterpretRuleApi>,
}

#[derive(Debug, Deserialize)]
struct InterpretRuleApi {
    id: String,
    when: String,
    #[serde(default = "default_severity")]
    severity: String,
    message: String,
}

fn default_category() -> String {
    "general".to_string()
}

fn default_step_type() -> String {
    "sql".to_string()
}

fn default_on_empty() -> String {
    "skip".to_string()
}

fn default_severity() -> String {
    "info".to_string()
}

impl SkillApiPayload {
    fn into_skill(self) -> Skill {
        let mut keywords: Vec<String> = self.tags.iter().map(|t| t.to_lowercase()).collect();
        for kw in self.keywords.zh.iter().chain(self.keywords.en.iter()) {
            keywords.push(kw.to_lowercase());
        }
        let title = if self.title.is_empty() {
            self.id.clone()
        } else {
            self.title
        };
        Skill {
            id: self.id,
            title,
            category: self.category,
            docs: self.docs.trim().to_string(),
            tags: self.tags,
            keywords,
            trigger_keywords: self.keywords,
            parameters: self
                .parameters
                .into_iter()
                .map(|p| SkillParameter {
                    name: p.name,
                    parameter_type: p.parameter_type,
                    default: json_to_yaml(&p.default),
                    description: p.description,
                })
                .collect(),
            steps: self
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
                .collect(),
            interpretation: self
                .interpretation
                .rules
                .into_iter()
                .map(|r| InterpretRule {
                    id: r.id,
                    when: r.when,
                    severity: r.severity,
                    message: r.message,
                })
                .collect(),
            summary_template: self.summary_template,
            next_steps: self.next_steps,
            variables: HashMap::new(),
            requires: self.requires,
        }
    }
}

pub fn catalog_json() -> Result<String> {
    serde_json::to_string(&catalog_to_json()?).context("serialize catalog")
}

pub fn routing_json() -> Result<String> {
    let catalog = catalog_to_json()?;
    let intents = load_intents()?;
    let pages = load_pages()?;
    serde_json::to_string(&json!({
        "catalog": catalog,
        "intents": intents,
        "pages": pages,
    }))
    .context("serialize routing")
}

pub fn list_skills_json(query: Option<&str>, limit: usize) -> Result<String> {
    let catalog = load_catalog();
    let mut summaries: Vec<Value> = Vec::new();
    for entry in &catalog {
        let title = load_skill(&entry.id)
            .map(|s| s.title)
            .unwrap_or_else(|_| entry.id.clone());
        summaries.push(json!({
            "id": entry.id,
            "category": entry.category,
            "description": entry.description,
            "priority": entry.priority,
            "title": title,
        }));
    }
    if let Some(q) = query.filter(|s| !s.trim().is_empty()) {
        let ranked = match_skills(q, limit);
        let by_id: HashMap<String, Value> = summaries
            .iter()
            .map(|s| {
                let id = s["id"].as_str().unwrap_or_default().to_string();
                (id, s.clone())
            })
            .collect();
        let ordered: Vec<Value> = ranked
            .into_iter()
            .filter_map(|id| by_id.get(&id).cloned())
            .collect();
        if !ordered.is_empty() {
            return serde_json::to_string_pretty(&ordered[..ordered.len().min(limit)])
                .context("serialize list");
        }
    }
    serde_json::to_string_pretty(&summaries[..summaries.len().min(limit)]).context("serialize list")
}

fn json_to_yaml(value: &Value) -> serde_yaml::Value {
    match value {
        Value::Null => serde_yaml::Value::Null,
        Value::Bool(b) => serde_yaml::Value::Bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                serde_yaml::Value::Number(i.into())
            } else if let Some(u) = n.as_u64() {
                serde_yaml::Value::Number(u.into())
            } else {
                serde_yaml::Value::Number(serde_yaml::Number::from(n.as_f64().unwrap_or(0.0)))
            }
        }
        Value::String(s) => serde_yaml::Value::String(s.clone()),
        Value::Array(arr) => serde_yaml::Value::Sequence(arr.iter().map(json_to_yaml).collect()),
        Value::Object(map) => {
            let mut out = serde_yaml::Mapping::new();
            for (k, v) in map {
                out.insert(serde_yaml::Value::String(k.clone()), json_to_yaml(v));
            }
            serde_yaml::Value::Mapping(out)
        }
    }
}

fn yaml_to_json(value: &serde_yaml::Value) -> Value {
    match value {
        serde_yaml::Value::Null => Value::Null,
        serde_yaml::Value::Bool(b) => json!(b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                json!(i)
            } else if let Some(u) = n.as_u64() {
                json!(u)
            } else {
                json!(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_yaml::Value::String(s) => json!(s),
        serde_yaml::Value::Sequence(seq) => {
            json!(seq.iter().map(yaml_to_json).collect::<Vec<_>>())
        }
        serde_yaml::Value::Mapping(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                let key = k.as_str().unwrap_or_default().to_string();
                out.insert(key, yaml_to_json(v));
            }
            Value::Object(out)
        }
        serde_yaml::Value::Tagged(tagged) => yaml_to_json(&tagged.value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_json_preserves_parameter_and_platform_contracts() {
        let skill = load_skill("comm_bottleneck").expect("comm_bottleneck skill");
        let value = skill_to_json(&skill);
        assert_eq!(value["parameters"][0]["type"], "integer");
        assert!(value["parameters"][0]["description"].is_string());
        let rdma = value["steps"]
            .as_array()
            .expect("steps")
            .iter()
            .find(|step| step["id"] == "rdma_hint")
            .expect("rdma_hint step");
        assert_eq!(rdma["platform"], "linux");

        let roundtrip = skill_from_api(&value).expect("valid roundtrip contract");
        assert_eq!(
            roundtrip.parameters[0].parameter_type,
            SkillParameterType::Integer
        );
    }
}
