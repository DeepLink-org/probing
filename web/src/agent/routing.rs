//! Catalog, intent, and page routing — loaded from probing server at runtime.

use crate::agent::skill::{catalog_entries, intent_catalog, page_catalog, CatalogEntry};
use crate::app::Route;
use crate::state::profiling::normalize_profiling_view;

#[derive(Debug, Clone)]
pub struct PageDescriptor {
    pub page_id: String,
    pub title: String,
    pub path: String,
    pub description: String,
    pub suggested_skills: Vec<String>,
}

pub fn catalog_skills() -> Vec<CatalogEntry> {
    catalog_entries()
}

pub fn catalog_skill_ids() -> Vec<String> {
    let mut entries = catalog_skills();
    entries.sort_by_key(|e| e.priority);
    entries.into_iter().map(|e| e.id).collect()
}

fn catalog_entry(id: &str) -> Option<CatalogEntry> {
    catalog_skills().into_iter().find(|e| e.id == id)
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
    let pages = page_catalog();
    if let Some(entry) = pages.get(&page_id) {
        return PageDescriptor {
            page_id: page_id.clone(),
            title: entry.title.clone(),
            path: entry.path.clone(),
            description: entry.description.clone(),
            suggested_skills: entry.skills.clone(),
        };
    }
    if page_id.starts_with("stacks/") {
        if let Some(entry) = pages.get("stacks") {
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
    let intents = intent_catalog();
    let mut scored: Vec<(usize, String)> = Vec::new();
    for intent in intents.values() {
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

#[allow(dead_code)]
pub fn route_for_page(page_id: &str) -> Option<Route> {
    let pages = page_catalog();
    let entry = pages.get(page_id)?;
    let path = entry.path.trim();
    if path.is_empty() {
        return None;
    }
    if path.starts_with("/profiling") {
        let view = path
            .trim_start_matches("/profiling/")
            .trim_start_matches('/');
        let view = if view.is_empty() { "overview" } else { view };
        return Some(Route::ProfilingViewPage {
            view: normalize_profiling_view(view).to_string(),
        });
    }
    match path {
        "/" | "/dashboard" => Some(Route::DashboardPage {}),
        "/agent" => Some(Route::AgentPage {}),
        "/cluster" => Some(Route::ClusterPage {}),
        "/stacks" => Some(Route::StackPage {}),
        "/analytics" => Some(Route::AnalyticsPage {}),
        "/python" => Some(Route::PythonPage {}),
        "/spans" => Some(Route::SpansPage {}),
        "/pulsing" => Some(Route::PulsingPage {}),
        "/training" => Some(Route::TrainingPage {}),
        _ => None,
    }
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
    let intents = intent_catalog();
    if !intents.is_empty() {
        lines.push(String::new());
        lines.push("Intent routing (user language → skills):".to_string());
        for (intent_id, intent) in &intents {
            lines.push(format!(
                "- {}: {} → {}",
                intent_id,
                intent.label,
                intent.skills.join(", ")
            ));
        }
    }
    lines.join("\n")
}
