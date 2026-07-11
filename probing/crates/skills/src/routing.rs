//! Skill intent / keyword routing (shared by Web agent and Python tools).

use std::collections::{HashMap, HashSet};

use super::catalog::{load_catalog, load_intents};
use super::loader::load_skill;

/// One intent route: keywords that map to skill ids.
#[derive(Debug, Clone)]
pub struct IntentRoute {
    pub keywords: Vec<String>,
    pub skills: Vec<String>,
}

/// Skill id + routing keywords (from tags + trigger keywords).
#[derive(Debug, Clone)]
pub struct SkillRoute {
    pub id: String,
    pub keywords: Vec<String>,
}

pub fn match_intent_routes(query: &str, limit: usize, routes: &[IntentRoute]) -> Vec<String> {
    let q = query.to_lowercase();
    let mut scored: Vec<(usize, String)> = Vec::new();
    for route in routes {
        let hits = route
            .keywords
            .iter()
            .filter(|kw| q.contains(kw.to_lowercase().as_str()))
            .count();
        if hits > 0 {
            for sid in &route.skills {
                scored.push((hits, sid.clone()));
            }
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

    let mut seen = HashSet::new();
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

pub fn match_routed_skills(
    query: &str,
    limit: usize,
    intent_routes: &[IntentRoute],
    skill_routes: &[SkillRoute],
) -> Vec<String> {
    let q = query.to_lowercase();
    let mut scored: HashMap<String, usize> = HashMap::new();

    for (rank, id) in match_intent_routes(query, 10, intent_routes)
        .into_iter()
        .enumerate()
    {
        *scored.entry(id).or_insert(0) += 3usize.saturating_mul(10 - rank);
    }

    for skill in skill_routes {
        let keyword_hits = skill
            .keywords
            .iter()
            .filter(|kw| q.contains(kw.as_str()))
            .count();
        let id_hit = q.contains(&skill.id.replace('_', " ")) || q.contains(&skill.id);
        if keyword_hits > 0 || id_hit {
            *scored.entry(skill.id.clone()).or_insert(0) += keyword_hits.max(1);
        }
    }

    let mut ranked: Vec<(usize, String)> =
        scored.into_iter().map(|(id, score)| (score, id)).collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    ranked.into_iter().take(limit).map(|(_, id)| id).collect()
}

pub fn match_skills(query: &str, limit: usize) -> Vec<String> {
    let intent_routes = load_intent_routes_from_yaml();
    let mut skill_routes = Vec::new();
    for entry in load_catalog() {
        let Ok(skill) = load_skill(&entry.id) else {
            continue;
        };
        skill_routes.push(SkillRoute {
            id: entry.id,
            keywords: skill.keywords,
        });
    }
    match_routed_skills(query, limit, &intent_routes, &skill_routes)
}

fn load_intent_routes_from_yaml() -> Vec<IntentRoute> {
    let Ok(intents) = load_intents() else {
        return Vec::new();
    };
    let Some(map) = intents.get("intents").and_then(|v| v.as_object()) else {
        return Vec::new();
    };

    map.values()
        .filter_map(|intent| {
            let keywords = intent
                .get("keywords")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let skills = intent
                .get("skills")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if keywords.is_empty() || skills.is_empty() {
                None
            } else {
                Some(IntentRoute { keywords, skills })
            }
        })
        .collect()
}
