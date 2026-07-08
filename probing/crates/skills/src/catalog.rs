//! Merged skill catalog and semantic routing overlays.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;

use super::discovery::{all_skill_root_paths, semantic_yaml_path};

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct CatalogEntry {
    pub id: String,
    #[serde(default)]
    pub path: String,
    #[serde(default, alias = "file")]
    pub file: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
struct CatalogFile {
    #[serde(default)]
    skills: Vec<CatalogEntry>,
}

pub fn load_catalog() -> Vec<CatalogEntry> {
    let mut merged: HashMap<String, CatalogEntry> = HashMap::new();
    for root in all_skill_root_paths() {
        if let Ok(entries) = load_fs_catalog(&root) {
            for entry in entries {
                merged.insert(entry.id.clone(), entry);
            }
        }
    }
    let mut out: Vec<CatalogEntry> = merged.into_values().collect();
    out.sort_by(|a, b| (a.priority, a.id.as_str()).cmp(&(b.priority, b.id.as_str())));
    out
}

pub fn load_fs_catalog(root: &Path) -> Result<Vec<CatalogEntry>> {
    let catalog_path = root.join("catalog.yaml");
    if !catalog_path.is_file() {
        return Ok(vec![]);
    }
    let text = fs::read_to_string(&catalog_path)
        .with_context(|| format!("read {}", catalog_path.display()))?;
    let file: CatalogFile = serde_yaml::from_str(&text)?;
    Ok(file.skills)
}

pub fn load_semantic_yaml(name: &str) -> Result<Value> {
    let path = semantic_yaml_path(name)
        .ok_or_else(|| anyhow::anyhow!("semantic file not found: {name}"))?;
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    serde_yaml::from_str(&text).context("parse semantic yaml")
}

pub fn load_intents() -> Result<Value> {
    load_semantic_yaml("intents.yaml")
}

pub fn load_pages() -> Result<Value> {
    load_semantic_yaml("pages.yaml")
}
