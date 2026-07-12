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
        let mut failed = Vec::new();
        for entry in &routing.catalog.skills {
            match self.fetch_skill_payload(&entry.id).await {
                Ok(payload) => payloads.push(payload),
                Err(err) => {
                    log::warn!("skill store: failed to load {}: {}", entry.id, err);
                    failed.push(entry.id.clone());
                }
            }
        }
        if payloads.is_empty() && !routing.catalog.skills.is_empty() {
            return Err(crate::utils::error::AppError::Api(format!(
                "failed to load any skills ({} errors)",
                failed.len()
            )));
        }
        populate_skill_store(routing, payloads);
        Ok(())
    }
}
