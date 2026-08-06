//! Catalog, intent, and page routing — loaded from probing server at runtime.

use crate::agent::skill::{catalog_entries, intent_catalog, CatalogEntry};

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
