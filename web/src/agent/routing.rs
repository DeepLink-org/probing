//! Catalog, intent, and page routing — shared skill index (embedded YAML).

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::app::Route;
use crate::state::profiling::normalize_profiling_view;

const CATALOG_YAML: &str = include_str!("../../../skills/catalog.yaml");
const INTENTS_YAML: &str = include_str!("../../../skills/semantic/intents.yaml");
const PAGES_YAML: &str = include_str!("../../../skills/semantic/pages.yaml");

#[derive(Debug, Deserialize)]
struct CatalogFile {
    #[serde(default)]
    skills: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct CatalogEntry {
    id: String,
    #[serde(default)]
    priority: i32,
    #[serde(default)]
    description: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    pages: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct IntentCatalogFile {
    #[serde(default)]
    intents: HashMap<String, IntentEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct IntentEntry {
    #[serde(default)]
    label: String,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    skills: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PageCatalogFile {
    #[serde(default)]
    pages: HashMap<String, PageEntry>,
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

#[derive(Debug, Clone)]
pub struct PageDescriptor {
    pub page_id: String,
    pub title: String,
    pub path: String,
    pub description: String,
    pub suggested_skills: Vec<String>,
}

fn catalog_file() -> &'static CatalogFile {
    static CACHE: OnceLock<CatalogFile> = OnceLock::new();
    CACHE.get_or_init(|| {
        serde_yaml::from_str(CATALOG_YAML).unwrap_or(CatalogFile { skills: vec![] })
    })
}

fn catalog_entries() -> Vec<CatalogEntry> {
    catalog_file().skills.clone()
}

fn intent_file() -> &'static IntentCatalogFile {
    static CACHE: OnceLock<IntentCatalogFile> = OnceLock::new();
    CACHE.get_or_init(|| {
        serde_yaml::from_str(INTENTS_YAML).unwrap_or(IntentCatalogFile {
            intents: HashMap::new(),
        })
    })
}

fn page_file() -> &'static PageCatalogFile {
    static CACHE: OnceLock<PageCatalogFile> = OnceLock::new();
    CACHE.get_or_init(|| {
        serde_yaml::from_str(PAGES_YAML).unwrap_or(PageCatalogFile {
            pages: HashMap::new(),
        })
    })
}

pub fn catalog_skill_ids() -> Vec<String> {
    let mut entries = catalog_entries();
    entries.sort_by_key(|e| e.priority);
    entries.into_iter().map(|e| e.id).collect()
}

fn catalog_entry(id: &str) -> Option<CatalogEntry> {
    catalog_entries().into_iter().find(|e| e.id == id)
}

pub fn page_id_for_route(route: &Route) -> String {
    match route {
        Route::DashboardPage {} => "dashboard".into(),
        Route::AgentPage {} => "agent".into(),
        Route::ClusterPage {} => "cluster".into(),
        Route::StackPage {} => "stacks".into(),
        Route::StackWithTidPage { tid } => format!("stacks/{tid}"),
        Route::ProfilingViewPage { view } => {
            format!("profiling/{}", normalize_profiling_view(view))
        }
        Route::ProfilingRedirect {} | Route::ChromeTracingRedirect {} => "profiling".into(),
        Route::AnalyticsPage {} => "analytics".into(),
        Route::PythonPage {} => "python".into(),
        Route::SpansPage {} | Route::TracesRedirect {} => "spans".into(),
        Route::PulsingPage {} => "pulsing".into(),
        Route::TrainingPage {} => "training".into(),
    }
}

pub fn describe_route(route: &Route) -> PageDescriptor {
    let page_id = page_id_for_route(route);
    if let Some(entry) = page_file().pages.get(&page_id) {
        return PageDescriptor {
            page_id: page_id.clone(),
            title: entry.title.clone(),
            path: entry.path.clone(),
            description: entry.description.clone(),
            suggested_skills: entry.skills.clone(),
        };
    }
    if page_id.starts_with("stacks/") {
        if let Some(entry) = page_file().pages.get("stacks") {
            return PageDescriptor {
                page_id: page_id.clone(),
                title: format!("{} · {}", entry.title, page_id),
                path: format!("/{}", page_id),
                description: entry.description.clone(),
                suggested_skills: entry.skills.clone(),
            };
        }
    }
    PageDescriptor {
        page_id: page_id.clone(),
        title: page_id.clone(),
        path: "/".into(),
        description: String::new(),
        suggested_skills: vec!["health_overview".into()],
    }
}

pub fn match_intents(query: &str, limit: usize) -> Vec<String> {
    let q = query.to_lowercase();
    let mut scored: Vec<(usize, String)> = Vec::new();
    for intent in intent_file().intents.values() {
        let hits = intent
            .keywords
            .iter()
            .filter(|kw| q.contains(kw.to_lowercase().as_str()))
            .count();
        if hits > 0 {
            for sid in &intent.skills {
                scored.push((hits, sid.clone()));
            }
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.dedup_by(|a, b| a.1 == b.1);
    scored.into_iter().take(limit).map(|(_, id)| id).collect()
}

pub fn routing_context_for_llm() -> String {
    let mut lines = vec!["Skill catalog (by priority):".to_string()];
    for id in catalog_skill_ids() {
        if let Some(entry) = catalog_entry(&id) {
            lines.push(format!(
                "- {} [{}]: {} (pages: {})",
                id,
                entry.category,
                entry.description,
                entry.pages.join(", ")
            ));
        }
    }
    lines.push(String::new());
    lines.push("Intent routing (user language → skills):".to_string());
    for (intent_id, intent) in &intent_file().intents {
        lines.push(format!(
            "- {}: {} → {}",
            intent_id,
            intent.label,
            intent.skills.join(", ")
        ));
    }
    lines.join("\n")
}
