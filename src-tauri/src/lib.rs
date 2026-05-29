mod backends;
mod config;
mod modules;
mod orchestrator;
mod types;

use backends::llm::LlmBackend;
use backends::search::openalex::OpenAlexBackend;
use config::AppConfig;
use orchestrator::Orchestrator;
use std::sync::Mutex;
use tauri::State;

pub struct AppState {
    config: Mutex<AppConfig>,
    llm: Mutex<Option<LlmBackend>>,
}

#[tauri::command]
async fn search(
    state: State<'_, AppState>,
    window: tauri::Window,
    query: String,
) -> Result<types::SearchResult, String> {
    let (config, llm) = {
        let config_guard = state.config.lock().map_err(|e| e.to_string())?;
        let config = config_guard.clone();
        drop(config_guard);

        let mut llm_guard = state.llm.lock().map_err(|e| e.to_string())?;
        if llm_guard.is_none() {
            *llm_guard = Some(LlmBackend::new(config.llm.clone()));
        }
        let llm = llm_guard.as_ref().unwrap().clone();
        (config, llm)
    };

    let backend = OpenAlexBackend::new();

    Orchestrator::run(&window, &llm, &backend, query, &config)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_config(
    state: State<'_, AppState>,
) -> Result<AppConfig, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;
    Ok(config.clone())
}

#[tauri::command]
async fn update_config(
    state: State<'_, AppState>,
    new_config: AppConfig,
) -> Result<(), String> {
    let config_path = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join("config.yaml");

    new_config.save(&config_path).map_err(|e| e.to_string())?;

    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    *config = new_config.clone();

    // Reset LLM backend so it picks up new config on next search
    let mut llm = state.llm.lock().map_err(|e| e.to_string())?;
    *llm = Some(LlmBackend::new(new_config.llm));

    Ok(())
}

fn load_config() -> AppConfig {
    let resource_path = std::env::current_dir()
        .ok()
        .map(|p| p.join("config.yaml"));

    if let Some(ref path) = resource_path {
        if path.exists() {
            if let Ok(config) = AppConfig::load(path) {
                return config;
            }
        }
    }

    AppConfig::default_config()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let config = load_config();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            config: Mutex::new(config),
            llm: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![search, get_config, update_config])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
