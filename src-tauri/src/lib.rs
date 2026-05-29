mod backends;
mod config;
mod modules;
mod orchestrator;
mod types;

use backends::llm::LlmBackend;
use backends::search::openalex::OpenAlexBackend;
use config::{AppConfig, LlmProfile};
use orchestrator::Orchestrator;
use std::sync::{Arc, Mutex};
use tauri::State;
use types::{Conversation, ConversationMessage};

pub struct AppState {
    config: Mutex<AppConfig>,
    llm: Mutex<Option<LlmBackend>>,
    progress: Arc<Mutex<Vec<types::ProgressEvent>>>,
    conversations: Mutex<Vec<Conversation>>,
}

fn config_dir() -> Result<std::path::PathBuf, String> {
    #[cfg(target_os = "windows")]
    { Ok(std::path::PathBuf::from(std::env::var("APPDATA").map_err(|_| "APPDATA")?).join("paper-search")) }
    #[cfg(not(target_os = "windows"))]
    { Ok(dirs::config_dir().unwrap_or_default().join("paper-search")) }
}

fn config_path() -> Result<std::path::PathBuf, String> { Ok(config_dir()?.join("config.yaml")) }
fn conversations_dir() -> Result<std::path::PathBuf, String> { Ok(config_dir()?.join("conversations")) }

fn load_config() -> AppConfig {
    if let Ok(path) = config_path() {
        if path.exists() { if let Ok(c) = AppConfig::load(&path) { return c; } }
    }
    AppConfig::default_config()
}

fn load_conversations() -> Vec<Conversation> {
    let dir = match conversations_dir() { Ok(d) => d, Err(_) => return vec![] };
    if !dir.exists() { return vec![]; }
    let mut cs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            if let Ok(data) = std::fs::read_to_string(e.path()) {
                if let Ok(c) = serde_json::from_str::<Conversation>(&data) { cs.push(c); }
            }
        }
    }
    cs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    cs
}

fn save_conversation(c: &Conversation) -> Result<(), String> {
    let dir = conversations_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let data = serde_json::to_string_pretty(c).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{}.json", c.id)), data).map_err(|e| e.to_string())
}

fn ensure_llm(state: &AppState) -> Result<LlmBackend, String> {
    let cfg = state.config.lock().map_err(|e| e.to_string())?.clone();
    let profile = cfg.llm.active_profile().cloned().unwrap_or_else(||
        LlmProfile::new("default", "Default", "openai", "deepseek-chat", "", "https://api.deepseek.com")
    );
    let mut guard = state.llm.lock().map_err(|e| e.to_string())?;
    if guard.is_none() || guard.as_ref().unwrap().config.api_key != profile.api_key {
        *guard = Some(LlmBackend::new(profile));
    }
    Ok(guard.as_ref().unwrap().clone())
}

fn now_ts() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

fn uuid_v4() -> String {
    let mut b = [0u8; 16];
    getrandom::getrandom(&mut b).ok();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format!("{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0],b[1],b[2],b[3],b[4],b[5],b[6],b[7],b[8],b[9],b[10],b[11],b[12],b[13],b[14],b[15])
}

// ── Commands ───────────────────────────────────────

#[tauri::command]
async fn search(
    state: State<'_, AppState>,
    query: String,
    conversation_id: Option<String>,
) -> Result<types::SearchResult, String> {
    let llm = ensure_llm(&state)?;
    let config = state.config.lock().map_err(|e| e.to_string())?.clone();

    state.progress.lock().unwrap().clear();
    let progress = state.progress.clone();
    let backend = OpenAlexBackend::new();

    let result = Orchestrator::run(&llm, &backend, query.clone(), &config, &progress).await;

    match result {
        Ok(r) => {
            let cid = conversation_id.unwrap_or_else(uuid_v4);
            let ts = now_ts();
            let conv = Conversation {
                id: cid.clone(),
                title: query.chars().take(50).collect(),
                messages: vec![
                    ConversationMessage { role: "user".into(), content: query, timestamp: ts },
                    ConversationMessage { role: "result".into(), content: r.summary.clone(), timestamp: ts },
                ],
                search_result: Some(r.clone()),
                created_at: ts,
            };
            let _ = save_conversation(&conv);
            if let Ok(mut cs) = state.conversations.lock() {
                cs.retain(|c| c.id != cid);
                cs.insert(0, conv);
            }
            Ok(r)
        }
        Err(e) => {
            state.progress.lock().unwrap().push(types::ProgressEvent {
                phase: "error".into(), message: format!("{}", e), percent: 0, detail: String::new(),
            });
            Err(e.to_string())
        }
    }
}

#[tauri::command]
async fn refine_search(
    state: State<'_, AppState>,
    conversation_id: String,
    refinement: String,
) -> Result<types::SearchResult, String> {
    let llm = ensure_llm(&state)?;
    let config = state.config.lock().map_err(|e| e.to_string())?.clone();
    let conv = {
        let cs = state.conversations.lock().map_err(|e| e.to_string())?;
        cs.iter().find(|c| c.id == conversation_id).cloned()
    };

    // Generate refined query from conversation context
    let refined_query = if let Some(ref c) = conv {
        let history = c.messages.iter()
            .filter(|m| m.role == "user")
            .map(|m| format!("- {}", m.content))
            .collect::<Vec<_>>().join("\n");
        let prompt = format!("用户之前的查询:\n{}\n\n用户反馈: {}\n\n请根据反馈生成更精确的英文搜索关键词。只输出关键词。", history, refinement);
        llm.chat_text("你是学术搜索助手。根据用户反馈生成优化后的搜索关键词。", &prompt).await.unwrap_or(refinement.clone())
    } else {
        refinement.clone()
    };

    state.progress.lock().unwrap().clear();
    let progress = state.progress.clone();
    let backend = OpenAlexBackend::new();

    let result = Orchestrator::run(&llm, &backend, refined_query.clone(), &config, &progress).await;

    match result {
        Ok(r) => {
            let ts = now_ts();
            let mut c = conv.unwrap_or(Conversation {
                id: conversation_id.clone(), title: refinement.clone(), messages: vec![], search_result: None, created_at: ts,
            });
            c.messages.push(ConversationMessage { role: "user".into(), content: refinement, timestamp: ts });
            c.messages.push(ConversationMessage { role: "result".into(), content: r.summary.clone(), timestamp: ts });
            c.search_result = Some(r.clone());
            let _ = save_conversation(&c);
            if let Ok(mut cs) = state.conversations.lock() {
                cs.retain(|c2| c2.id != c.id);
                cs.insert(0, c);
            }
            Ok(r)
        }
        Err(e) => {
            state.progress.lock().unwrap().push(types::ProgressEvent {
                phase: "error".into(), message: format!("{}", e), percent: 0, detail: String::new(),
            });
            Err(e.to_string())
        }
    }
}

#[tauri::command]
async fn get_progress(state: State<'_, AppState>) -> Result<Vec<types::ProgressEvent>, String> {
    Ok(state.progress.lock().map_err(|e| e.to_string())?.clone())
}

#[tauri::command]
async fn get_config(state: State<'_, AppState>) -> Result<AppConfig, String> {
    Ok(state.config.lock().map_err(|e| e.to_string())?.clone())
}

#[tauri::command]
async fn update_config(state: State<'_, AppState>, new_config: AppConfig) -> Result<(), String> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}", e))?;
    }
    new_config.save(&path).map_err(|e| format!("{}", e))?;
    // Reset LLM so it picks up new profile
    let mut llm = state.llm.lock().map_err(|e| e.to_string())?;
    if let Some(profile) = new_config.llm.active_profile().cloned() {
        *llm = Some(LlmBackend::new(profile));
    }
    let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
    *cfg = new_config;
    Ok(())
}

#[tauri::command]
async fn get_conversations(state: State<'_, AppState>) -> Result<Vec<Conversation>, String> {
    Ok(state.conversations.lock().map_err(|e| e.to_string())?.clone())
}

#[tauri::command]
async fn delete_conversation(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let _ = std::fs::remove_file(conversations_dir()?.join(format!("{}.json", id)));
    state.conversations.lock().map_err(|e| e.to_string())?.retain(|c| c.id != id);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let config = load_config();
    let conversations = load_conversations();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            config: Mutex::new(config),
            llm: Mutex::new(None),
            progress: Arc::new(Mutex::new(Vec::new())),
            conversations: Mutex::new(conversations),
        })
        .invoke_handler(tauri::generate_handler![
            search, refine_search, get_progress, get_config, update_config,
            get_conversations, delete_conversation
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
