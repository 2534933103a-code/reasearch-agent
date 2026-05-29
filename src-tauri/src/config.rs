use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmProfile {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub model: String,
    pub api_key: String,
    pub base_url: String,
}

impl LlmProfile {
    pub fn new(id: &str, name: &str, provider: &str, model: &str, api_key: &str, base_url: &str) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            api_key: api_key.to_string(),
            base_url: base_url.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub profiles: Vec<LlmProfile>,
    pub active_profile_id: String,
}

impl LlmConfig {
    pub fn active_profile(&self) -> Option<&LlmProfile> {
        self.profiles.iter().find(|p| p.id == self.active_profile_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchBackendConfig {
    pub name: String,
    pub enabled: bool,
    pub priority: u8,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchConfig {
    pub backends: Vec<SearchBackendConfig>,
    pub max_results_per_query: usize,
    pub max_rounds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetConfig {
    pub max_llm_calls: u32,
    pub max_search_calls: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub llm: LlmConfig,
    pub search: SearchConfig,
    pub budget: BudgetConfig,
}

impl AppConfig {
    pub fn load(path: &PathBuf) -> Result<Self, anyhow::Error> {
        let content = std::fs::read_to_string(path)?;

        // Try new format first; fall back to legacy single-profile format
        let mut config: serde_json::Value = serde_yaml::from_str(&content)?;

        // Convert legacy format: llm.provider/model/api_key/base_url → llm.profiles[]
        if !config["llm"].is_null() && config["llm"]["profiles"].is_null() {
            let legacy = &config["llm"];
            let default_profile = serde_json::json!({
                "id": "default",
                "name": "Default",
                "provider": legacy["provider"].as_str().unwrap_or("openai"),
                "model": legacy["model"].as_str().unwrap_or("deepseek-chat"),
                "api_key": legacy["api_key"].as_str().unwrap_or(""),
                "base_url": legacy["base_url"].as_str().unwrap_or("https://api.deepseek.com"),
            });
            config["llm"] = serde_json::json!({
                "profiles": [default_profile],
                "active_profile_id": "default",
            });
        }

        let config: AppConfig = serde_json::from_value(config)?;
        Ok(config)
    }

    pub fn save(&self, path: &PathBuf) -> Result<(), anyhow::Error> {
        let content = serde_yaml::to_string(self)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn default_config() -> Self {
        Self {
            llm: LlmConfig {
                profiles: vec![LlmProfile::new(
                    "default", "Default", "openai", "deepseek-chat", "", "https://api.deepseek.com",
                )],
                active_profile_id: "default".into(),
            },
            search: SearchConfig {
                backends: vec![SearchBackendConfig {
                    name: "openalex".into(),
                    enabled: true,
                    priority: 1,
                    api_key: None,
                }],
                max_results_per_query: 15,
                max_rounds: 3,
            },
            budget: BudgetConfig {
                max_llm_calls: 10,
                max_search_calls: 30,
            },
        }
    }
}
