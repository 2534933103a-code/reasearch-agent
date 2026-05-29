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
use tauri::{Emitter, State};

pub struct AppState {
    config: Mutex<AppConfig>,
    llm: Mutex<Option<LlmBackend>>,
}

fn config_dir() -> Result<std::path::PathBuf, String> {
    // On Windows: %APPDATA%/paper-search/
    // On other: ~/.config/paper-search/
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var("APPDATA").map_err(|_| "找不到 APPDATA 目录".to_string())?;
        Ok(std::path::PathBuf::from(base).join("paper-search"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let base = dirs::config_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        Ok(base.join("paper-search"))
    }
}

fn config_path() -> Result<std::path::PathBuf, String> {
    let dir = config_dir()?;
    Ok(dir.join("config.yaml"))
}

fn load_config() -> AppConfig {
    if let Ok(path) = config_path() {
        if path.exists() {
            if let Ok(config) = AppConfig::load(&path) {
                return config;
            }
        }
    }
    AppConfig::default_config()
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

    // Immediate feedback before any async work
    window.emit("progress", types::ProgressEvent {
        phase: "start".into(),
        message: "引擎启动，正在连接 LLM...".into(),
        percent: 2,
        detail: String::new(),
    }).ok();

    let backend = OpenAlexBackend::new();

    Orchestrator::run(&window, &llm, &backend, query, &config)
        .await
        .map_err(|e| {
            // Try to emit the error as a progress event
            window.emit("progress", types::ProgressEvent {
                phase: "error".into(),
                message: format!("搜索出错: {}", e),
                percent: 0,
                detail: String::new(),
            }).ok();
            e.to_string()
        })
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
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("无法创建配置目录: {}", e))?;
    }
    new_config.save(&path).map_err(|e| format!("保存配置失败: {}", e))?;

    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    *config = new_config.clone();

    let mut llm = state.llm.lock().map_err(|e| e.to_string())?;
    *llm = Some(LlmBackend::new(new_config.llm));

    Ok(())
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
