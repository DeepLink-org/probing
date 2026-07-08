//! JSON serialization for skill HTTP / MCP discovery APIs.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use super::catalog::{load_catalog, load_intents, load_pages, CatalogEntry};
use super::loader::{load_skill, InterpretRule, KeywordsSpec, Skill, SkillParameter, SkillStep};
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
                "default": yaml_to_json(&p.default),
            })
        }).collect::<Vec<_>>(),
        "steps": skill.steps.iter().map(step_to_json).collect::<Vec<_>>(),
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
        "path": step.path,
        "view": step.view,
        "on_empty": step.on_empty,
        "empty_message": step.empty_message,
        "when": step.when,
        "cluster": step.cluster,
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
    let id = value
        .get("id")
        .and_then(|v| v.as_str())
        .context("skill id")?
        .to_string();
    let title = value
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or(&id)
        .to_string();
    let category = value
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("general")
        .to_string();
    let docs = value
        .get("docs")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let tags: Vec<String> = value
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let keywords_obj = value.get("keywords").cloned().unwrap_or(json!({}));
    let trigger_keywords = KeywordsSpec {
        zh: keywords_obj
            .get("zh")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        en: keywords_obj
            .get("en")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
    };
    let mut keywords: Vec<String> = tags.iter().map(|t| t.to_lowercase()).collect();
    for kw in trigger_keywords.zh.iter().chain(trigger_keywords.en.iter()) {
        keywords.push(kw.to_lowercase());
    }
    let parameters = value
        .get("parameters")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let name = item.get("name")?.as_str()?.to_string();
                    let default = item.get("default").cloned().unwrap_or(Value::Null);
                    Some(SkillParameter {
                        name,
                        default: json_to_yaml(&default),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let steps = value
        .get("steps")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    Some(SkillStep {
                        id: item.get("id")?.as_str()?.to_string(),
                        title: item.get("title")?.as_str()?.to_string(),
                        step_type: item
                            .get("type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("sql")
                            .to_string(),
                        sql: item.get("sql").and_then(|v| v.as_str()).map(str::to_string),
                        path: item
                            .get("path")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        view: item
                            .get("view")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        on_empty: item
                            .get("on_empty")
                            .and_then(|v| v.as_str())
                            .unwrap_or("skip")
                            .to_string(),
                        empty_message: item
                            .get("empty_message")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        when: item
                            .get("when")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        cluster: item.get("cluster").and_then(|v| v.as_bool()),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let interpretation = value
        .get("interpretation")
        .and_then(|v| v.get("rules"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    Some(InterpretRule {
                        id: item.get("id")?.as_str()?.to_string(),
                        when: item.get("when")?.as_str()?.to_string(),
                        severity: item
                            .get("severity")
                            .and_then(|v| v.as_str())
                            .unwrap_or("info")
                            .to_string(),
                        message: item.get("message")?.as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let summary_template = value
        .get("summary_template")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let next_steps = value
        .get("next_steps")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Ok(Skill {
        id,
        title,
        category,
        docs,
        tags,
        keywords,
        trigger_keywords,
        parameters,
        steps,
        interpretation,
        summary_template,
        next_steps,
        variables: HashMap::new(),
    })
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
