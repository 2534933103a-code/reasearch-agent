use crate::config::LlmConfig;
use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::Value;

pub struct LlmBackend {
    client: Client,
    config: LlmConfig,
}

impl LlmBackend {
    pub fn new(config: LlmConfig) -> Self {
        Self {
            client: Client::new(),
            config,
        }
    }

    pub async fn chat(&self, system_prompt: &str, user_prompt: &str) -> Result<String> {
        let url = format!("{}/v1/chat/completions", self.config.base_url.trim_end_matches('/'));

        let body = serde_json::json!({
            "model": self.config.model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt}
            ],
            "temperature": 0.3,
            "max_tokens": 2000,
            "response_format": {"type": "json_object"}
        });

        let resp = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("LLM API request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("LLM API error {}: {}", status, text);
        }

        let json: Value = resp.json().await?;
        let content = json["choices"][0]["message"]["content"]
            .as_str()
            .context("Missing content in LLM response")?
            .to_string();

        Ok(content)
    }

    pub async fn chat_text(&self, system_prompt: &str, user_prompt: &str) -> Result<String> {
        let url = format!("{}/v1/chat/completions", self.config.base_url.trim_end_matches('/'));

        let body = serde_json::json!({
            "model": self.config.model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt}
            ],
            "temperature": 0.5,
            "max_tokens": 2000
        });

        let resp = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("LLM API request failed")?;

        let json: Value = resp.json().await?;
        let content = json["choices"][0]["message"]["content"]
            .as_str()
            .context("Missing content in LLM response")?
            .to_string();

        Ok(content)
    }
}
