//! Diagnostic skills loaded at runtime from ``/apis/pythonext/skills/*``.

use std::collections::HashMap;
use std::sync::OnceLock;

use probing_skills::{
    match_routed_skills, match_skills, skill_from_api, IntentRoute, Skill, SkillRoute,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct RoutingPayload {
    pub catalog: CatalogPayload,
    pub intents: IntentCatalogFile,
    pub pages: PageCatalogFile,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogPayload {
    pub skills: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogEntry {
    pub id: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub pages: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IntentCatalogFile {
    #[serde(default)]
    pub intents: HashMap<String, IntentEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IntentEntry {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PageCatalogFile {
    #[serde(default)]
    pub pages: HashMap<String, PageEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PageEntry {
    pub title: String,
    pub path: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub skills: Vec<String>,
}

pub type SkillPayload = Value;

#[derive(Debug, Default)]
struct SkillStore {
    skills: HashMap<String, Skill>,
    catalog: Vec<CatalogEntry>,
    intents: HashMap<String, IntentEntry>,
    pages: HashMap<String, PageEntry>,
    loaded: bool,
}

static STORE: OnceLock<std::sync::RwLock<SkillStore>> = OnceLock::new();

fn store() -> &'static std::sync::RwLock<SkillStore> {
    STORE.get_or_init(|| std::sync::RwLock::new(SkillStore::default()))
}

pub fn skill_store_loaded() -> bool {
    store().read().map(|s| s.loaded).unwrap_or(false)
}

pub fn populate_skill_store(routing: RoutingPayload, payloads: Vec<SkillPayload>) {
    let mut skills = HashMap::new();
    for payload in payloads {
        if let Ok(skill) = skill_from_api(&payload) {
            let id = skill.id.clone();
            skills.insert(id, skill);
        }
    }
    if let Ok(mut guard) = store().write() {
        guard.catalog = routing.catalog.skills;
        guard.intents = routing.intents.intents;
        guard.pages = routing.pages.pages;
        guard.skills = skills;
        guard.loaded = true;
    }
}

pub fn catalog_entries() -> Vec<CatalogEntry> {
    store()
        .read()
        .map(|s| s.catalog.clone())
        .unwrap_or_default()
}

pub fn intent_catalog() -> HashMap<String, IntentEntry> {
    store()
        .read()
        .map(|s| s.intents.clone())
        .unwrap_or_default()
}

pub fn page_catalog() -> HashMap<String, PageEntry> {
    store().read().map(|s| s.pages.clone()).unwrap_or_default()
}

pub fn list_skill_ids() -> Vec<String> {
    store()
        .read()
        .map(|s| {
            let mut ids: Vec<String> = s.skills.keys().cloned().collect();
            ids.sort();
            ids
        })
        .unwrap_or_default()
}

pub fn load_skill(id: &str) -> Option<Skill> {
    store().read().ok()?.skills.get(id).cloned()
}

pub fn match_skills_for_query(query: &str, limit: usize) -> Vec<String> {
    if skill_store_loaded() {
        let Ok(guard) = store().read() else {
            return match_skills(query, limit);
        };
        let intent_routes: Vec<IntentRoute> = guard
            .intents
            .values()
            .map(|intent| IntentRoute {
                keywords: intent.keywords.clone(),
                skills: intent.skills.clone(),
            })
            .collect();
        let skill_routes: Vec<SkillRoute> = guard
            .skills
            .values()
            .map(|skill| SkillRoute {
                id: skill.id.clone(),
                keywords: skill.keywords.clone(),
            })
            .collect();
        return match_routed_skills(query, limit, &intent_routes, &skill_routes);
    }
    match_skills(query, limit)
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
    let matched = match_skills_for_query(trimmed, 1);
    matched.into_iter().next()
}
