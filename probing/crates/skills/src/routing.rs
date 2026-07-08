//! Skill intent / keyword routing (shared by Web agent and Python tools).

use std::collections::HashMap;

use super::catalog::load_catalog;
use super::loader::load_skill;

pub fn match_skills(query: &str, limit: usize) -> Vec<String> {
    let q = query.to_lowercase();
    let mut scored: HashMap<String, usize> = HashMap::new();

    for (rank, id) in match_intents(query, 10).into_iter().enumerate() {
        *scored.entry(id).or_insert(0) += 3usize.saturating_mul(10 - rank);
    }

    for entry in load_catalog() {
        let Ok(skill) = load_skill(&entry.id) else {
            continue;
        };
        let kw_score = skill
            .keywords
            .iter()
            .filter(|kw| q.contains(kw.as_str()))
            .count();
        if kw_score > 0 {
            *scored.entry(entry.id).or_insert(0) += kw_score;
        }
    }

    let mut ranked: Vec<(usize, String)> =
        scored.into_iter().map(|(id, score)| (score, id)).collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    ranked.into_iter().take(limit).map(|(_, id)| id).collect()
}

fn match_intents(query: &str, limit: usize) -> Vec<String> {
    let q = query.to_lowercase();
    let Ok(intents) = super::catalog::load_intents() else {
        return Vec::new();
    };
    let Some(map) = intents.get("intents").and_then(|v| v.as_object()) else {
        return Vec::new();
    };

    let mut scored: Vec<(usize, String)> = Vec::new();
    for intent in map.values() {
        let keywords = intent
            .get("keywords")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let hits = keywords
            .iter()
            .filter(|kw| q.contains(&kw.to_lowercase()))
            .count();
        if hits == 0 {
            continue;
        }
        if let Some(ids) = intent.get("skills").and_then(|v| v.as_array()) {
            for sid in ids {
                if let Some(id) = sid.as_str() {
                    scored.push((hits, id.to_string()));
                }
            }
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (_, id) in scored {
        if seen.insert(id.clone()) {
            out.push(id);
        }
        if out.len() >= limit {
            break;
        }
    }
    out
}
