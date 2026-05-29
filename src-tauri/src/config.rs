use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub provider: String,
    pub model: String,
    pub api_key: String,
    pub base_url: String,
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
        let config: AppConfig = serde_yaml::from_str(&content)?;
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
                provider: "openai".into(),
                model: "deepseek-chat".into(),
                api_key: String::new(),
                base_url: "https://api.deepseek.com".into(),
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
