use crate::config::LlmProfile;
use crate::types::{LlmMessage, ToolCall, ToolDef};
use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

#[derive(Clone)]
pub struct LlmBackend {
    client: Client,
    pub config: LlmProfile,
}

pub struct ChatResponse {
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub tokens: u32,
}

impl LlmBackend {
    pub fn new(config: LlmProfile) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .unwrap_or_else(|_| Client::new()),
            config,
        }
    }

    fn extract_tokens(json: &Value) -> u32 {
        // Standard OpenAI format: usage.total_tokens
        if let Some(n) = json["usage"]["total_tokens"].as_u64() {
            return n as u32;
        }
        // Some providers use usage.total_tokens as i64
        if let Some(n) = json["usage"]["total_tokens"].as_i64() {
            return n.max(0) as u32;
        }
        // Fallback: sum prompt + completion
        let prompt = json["usage"]["prompt_tokens"].as_u64().unwrap_or(0)
            .max(json["usage"]["prompt_tokens"].as_i64().unwrap_or(0).max(0) as u64);
        let completion = json["usage"]["completion_tokens"].as_u64().unwrap_or(0)
            .max(json["usage"]["completion_tokens"].as_i64().unwrap_or(0).max(0) as u64);
        if prompt + completion > 0 {
            return (prompt + completion) as u32;
        }
        // Last resort: rough estimate from response content length (≈ chars / 2 for Chinese, chars / 4 for English)
        let content_len = json["choices"][0]["message"]["content"]
            .as_str().unwrap_or("").len() as u32;
        if content_len > 0 {
            return content_len / 2; // rough char→token for mixed Chinese/English
        }
        0
    }

    fn extract_tool_calls(choice: &Value) -> Option<Vec<ToolCall>> {
        let calls = choice["message"]["tool_calls"].as_array()?;
        if calls.is_empty() {
            return None;
        }
        let mut result = Vec::new();
        for tc in calls {
            result.push(ToolCall {
                id: tc["id"].as_str()?.to_string(),
                call_type: tc["type"].as_str().unwrap_or("function").to_string(),
                function: crate::types::ToolCallFunction {
                    name: tc["function"]["name"].as_str()?.to_string(),
                    arguments: tc["function"]["arguments"].as_str().unwrap_or("{}").to_string(),
                },
            });
        }
        Some(result)
    }

    /// Send a multi-turn conversation with tools. Supports function calling.
    /// Retries up to 2 times on network errors.
    pub async fn chat_with_tools(
        &self,
        messages: &[LlmMessage],
        tools: &[ToolDef],
    ) -> Result<ChatResponse> {
        let url = format!(
            "{}/v1/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );

        let body = serde_json::json!({
            "model": self.config.model,
            "messages": messages,
            "temperature": 0.3,
            "max_tokens": 4096,
            "tools": tools,
            "tool_choice": "auto"
        });

        let mut last_err: Option<anyhow::Error> = None;

        for attempt in 0..3 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(500 * (1 << attempt))).await;
                // On JSON parse failure, add a hint to the next request
                if let Some(ref err) = last_err {
                    if err.to_string().contains("JSON") || err.to_string().contains("parse") {
                        // Don't modify body for parse errors on retry — just retry with same input
                    }
                }
            }

            let resp = match self
                .client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.config.api_key))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    last_err = Some(anyhow::anyhow!("HTTP request failed: {}", e));
                    continue;
                }
            };

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                // Don't retry on 4xx (except 429)
                if status.is_client_error() && status.as_u16() != 429 {
                    anyhow::bail!("LLM API error {}: {}", status, text);
                }
                last_err = Some(anyhow::anyhow!("LLM API error {}: {}", status, text));
                continue;
            }

            let json: Value = match resp.json().await {
                Ok(j) => j,
                Err(e) => {
                    last_err = Some(anyhow::anyhow!("Failed to parse JSON response: {}", e));
                    continue;
                }
            };

            let choice = &json["choices"][0];
            let content = choice["message"]["content"].as_str().map(|s| s.to_string());
            let tool_calls = Self::extract_tool_calls(choice);
            let tokens = Self::extract_tokens(&json);

            return Ok(ChatResponse { content, tool_calls, tokens });
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("LLM API request failed after 3 retries")))
    }

    // ── Legacy methods (kept for backward compat) ──

    pub async fn chat(&self, system_prompt: &str, user_prompt: &str) -> Result<ChatResponse> {
        // Legacy JSON-mode call — kept for backward compatibility with existing modules
        let url = format!(
            "{}/v1/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );

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

        let resp = self
            .client
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
        let tokens = Self::extract_tokens(&json);

        Ok(ChatResponse { content: Some(content), tool_calls: None, tokens })
    }

    pub async fn chat_text(&self, system_prompt: &str, user_prompt: &str) -> Result<ChatResponse> {
        let url = format!(
            "{}/v1/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );

        let body = serde_json::json!({
            "model": self.config.model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt}
            ],
            "temperature": 0.5,
            "max_tokens": 2000
        });

        let resp = self
            .client
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
        let tokens = Self::extract_tokens(&json);

        Ok(ChatResponse { content: Some(content), tool_calls: None, tokens })
    }
}
