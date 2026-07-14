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
        Route::StackDistributedFullPage {} => "stacks/distributed".into(),
        Route::StackDistributedPyPage {} => "stacks/distributed/py".into(),
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
