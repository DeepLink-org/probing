//! Fetch skills from probing server and populate the in-memory skill store.

use super::ApiClient;
use crate::agent::{populate_skill_store, RoutingPayload, SkillPayload};
use crate::utils::error::Result;

impl ApiClient {
    pub async fn fetch_skills_routing(&self) -> Result<RoutingPayload> {
        let text = self.get_request("/apis/pythonext/skills/routing").await?;
        Self::parse_json(&text)
    }

    pub async fn fetch_skill_payload(&self, id: &str) -> Result<SkillPayload> {
        let path = format!("/apis/pythonext/skills/load?id={}", urlencoding::encode(id));
        let text = self.get_request(&path).await?;
        Self::parse_json(&text)
    }

    pub async fn load_skill_store(&self) -> Result<()> {
        let routing = self.fetch_skills_routing().await?;
        let mut payloads = Vec::new();
        for entry in &routing.catalog.skills {
            if let Ok(payload) = self.fetch_skill_payload(&entry.id).await {
                payloads.push(payload);
            }
        }
        populate_skill_store(routing, payloads);
        Ok(())
    }
}
