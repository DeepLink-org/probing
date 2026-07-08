//! Runtime discovery of skill root directories (no compile-time embed).

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::process::Command;
use std::sync::OnceLock;

#[cfg(not(target_arch = "wasm32"))]
use anyhow::{Context, Result};
#[cfg(not(target_arch = "wasm32"))]
use serde::Deserialize;

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Deserialize)]
struct RootEntry {
    path: String,
    #[serde(rename = "label")]
    _label: String,
}

static CACHED_ROOTS: OnceLock<Vec<PathBuf>> = OnceLock::new();

pub fn all_skill_root_paths() -> Vec<PathBuf> {
    CACHED_ROOTS.get_or_init(discover_skill_roots).clone()
}

fn discover_skill_roots() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen = HashSet::new();

    let python_roots = python_discovered_roots();
    let use_bundled_fallback = python_roots.is_empty();

    let sources = python_roots
        .into_iter()
        .chain(fs_skill_roots())
        .chain(if use_bundled_fallback {
            bundled_skill_roots()
        } else {
            Vec::new()
        });

    for path in sources {
        let key = path.to_string_lossy().to_string();
        if seen.insert(key) {
            out.push(path);
        }
    }
    out
}

fn bundled_skill_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(path) = env::var("PROBING_BUNDLED_SKILLS") {
        let p = PathBuf::from(path);
        if p.join("catalog.yaml").is_file() {
            roots.push(p);
        }
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dev_bundled = manifest.join("../../python/probing/bundled_skills");
    if dev_bundled.join("catalog.yaml").is_file() {
        roots.push(dev_bundled);
    }
    if let Some(repo) = find_repo_root() {
        let skills = repo.join("skills");
        if skills.join("catalog.yaml").is_file() {
            roots.push(skills);
        }
        let bundled = repo.join("python/probing/bundled_skills");
        if bundled.join("catalog.yaml").is_file() {
            roots.push(bundled);
        }
    }
    roots
}

fn find_repo_root() -> Option<PathBuf> {
    let start = env::current_dir().ok()?;
    for directory in start.ancestors() {
        if directory.join("skills").join("catalog.yaml").is_file()
            && directory.join("pyproject.toml").is_file()
        {
            return Some(directory.to_path_buf());
        }
        if directory
            .join("python/probing/bundled_skills/catalog.yaml")
            .is_file()
            && directory.join("pyproject.toml").is_file()
        {
            return Some(directory.to_path_buf());
        }
    }
    None
}

fn python_discovered_roots() -> Vec<PathBuf> {
    #[cfg(target_arch = "wasm32")]
    {
        Vec::new()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        for python in python_candidates() {
            let output = Command::new(&python)
                .args(["-m", "probing.extensions", "skill-roots"])
                .output();
            let Ok(output) = output else {
                continue;
            };
            if !output.status.success() {
                continue;
            }
            let text = String::from_utf8_lossy(&output.stdout);
            let entries: Vec<RootEntry> = match serde_json::from_str(text.trim()) {
                Ok(v) => v,
                Err(_) => continue,
            };
            return entries
                .into_iter()
                .map(|e| PathBuf::from(e.path))
                .filter(|p| p.join("catalog.yaml").is_file() || p.is_dir())
                .collect();
        }
        Vec::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn python_candidates() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(exe) = env::var("PROBING_PYTHON") {
        out.push(exe);
    }
    for name in ["python3", "python"] {
        if let Ok(path) = which(name) {
            out.push(path);
        }
    }
    out
}

#[cfg(not(target_arch = "wasm32"))]
fn which(name: &str) -> Result<String> {
    let path_var = env::var_os("PATH").context("PATH not set")?;
    for dir in env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate.to_string_lossy().into_owned());
        }
    }
    anyhow::bail!("{name} not found on PATH")
}

pub fn home_skills_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(|home| PathBuf::from(home).join(".probing/skills"))
}

pub fn project_skills_dir() -> Option<PathBuf> {
    let cwd = env::current_dir().ok()?;
    for ancestor in cwd.ancestors() {
        let candidate = ancestor.join(".probing/skills");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    None
}

pub fn env_skills_dir() -> Option<PathBuf> {
    env::var("PROBING_SKILLS_DIR")
        .ok()
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

pub fn fs_skill_roots() -> Vec<PathBuf> {
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

use super::catalog::{load_fs_catalog, CatalogEntry};

pub fn fs_steps_path(root: &Path, id: &str) -> Option<PathBuf> {
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

pub fn load_fs_steps_yaml(id: &str) -> Option<String> {
    for root in all_skill_root_paths().into_iter().rev() {
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

pub fn semantic_yaml_path(name: &str) -> Option<PathBuf> {
    for root in all_skill_root_paths().into_iter().rev() {
        let path = root.join("semantic").join(name);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_discovery_returns_json_when_available() {
        let roots = python_discovered_roots();
        // In dev/CI with probing on PYTHONPATH this should be non-empty.
        if roots.is_empty() {
            return;
        }
        assert!(roots.iter().all(|p| p.is_dir()));
    }
}
