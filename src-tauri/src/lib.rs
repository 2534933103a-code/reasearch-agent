mod backends;
mod config;
mod modules;
mod orchestrator;
mod types;

use backends::llm::LlmBackend;
use backends::search::openalex::OpenAlexBackend;
use config::{AppConfig, LlmProfile};
use orchestrator::Orchestrator;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use tauri::State;
use types::{Conversation, ConversationMessage};

pub struct AppState {
    config: Mutex<AppConfig>,
    llm: Mutex<Option<LlmBackend>>,
    progress: Arc<Mutex<Vec<types::ProgressEvent>>>,
    conversations: Mutex<Vec<Conversation>>,
    cancelled: Arc<AtomicBool>,
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

    state.cancelled.store(false, Ordering::SeqCst);
    let result = Orchestrator::run(&llm, &backend, query.clone(), &config, &progress, None, &[], &state.cancelled).await;
    match result {
        Ok(mut r) => {
            let cid = conversation_id.unwrap_or_else(uuid_v4);
            r.conversation_id = cid.clone();
            let ts = now_ts();
            let conv = Conversation {
                id: cid.clone(),
                title: query.chars().take(50).collect(),
                messages: vec![
                    ConversationMessage { role: "user".into(), content: query, timestamp: ts },
                    ConversationMessage { role: "result".into(), content: r.summary.clone(), timestamp: ts },
                ],
                search_results: vec![r.clone()],
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
                phase: "error".into(), message: format!("{}", e), percent: 0, detail: String::new(), tokens: 0,
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

    // Get original query from conversation (first user message)
    let original_query = conv.as_ref().and_then(|c| {
        c.messages.iter()
            .filter(|m| m.role == "user")
            .map(|m| m.content.clone())
            .next()
    }).unwrap_or_else(|| refinement.clone());

    // Build refinement chain from all previous refinements
    let refinement_context = {
        let prev: Vec<String> = conv.as_ref().map(|c| {
            c.messages.iter()
                .filter(|m| m.role == "user")
                .skip(1)  // skip original query
                .map(|m| m.content.clone())
                .collect()
        }).unwrap_or_default();

        if prev.is_empty() {
            format!("用户细化要求: {}", refinement)
        } else {
            format!("之前的细化链: {}\n当前最新细化: {}", prev.join(" → "), refinement)
        }
    };

    // Collect all papers from previous search rounds
    let existing_papers: Vec<types::Paper> = conv.as_ref().map(|c| {
        c.search_results.iter().flat_map(|sr| {
            sr.tiers.high_relevance.iter()
                .chain(sr.tiers.partial_relevance.iter())
                .map(|sp| sp.paper.clone())
                .collect::<Vec<_>>()
        }).collect()
    }).unwrap_or_default();

    state.progress.lock().unwrap().clear();
    let progress = state.progress.clone();
    let backend = OpenAlexBackend::new();

    // Pass full refinement chain + existing papers — agent sees complete history
    state.cancelled.store(false, Ordering::SeqCst);
    let result = Orchestrator::run(&llm, &backend, original_query.clone(), &config, &progress, Some(&refinement_context), &existing_papers, &state.cancelled).await;

    match result {
        Ok(mut r) => {
            r.conversation_id = conversation_id.clone();
            let ts = now_ts();
            let mut c = conv.unwrap_or(Conversation {
                id: conversation_id.clone(), title: refinement.clone(), messages: vec![], search_results: vec![], created_at: ts,
            });
            c.messages.push(ConversationMessage { role: "user".into(), content: refinement, timestamp: ts });
            c.messages.push(ConversationMessage { role: "result".into(), content: r.summary.clone(), timestamp: ts });
            c.search_results.push(r.clone());
            let _ = save_conversation(&c);
            if let Ok(mut cs) = state.conversations.lock() {
                cs.retain(|c2| c2.id != c.id);
                cs.insert(0, c);
            }
            Ok(r)
        }
        Err(e) => {
            state.progress.lock().unwrap().push(types::ProgressEvent {
                phase: "error".into(), message: format!("{}", e), percent: 0, detail: String::new(), tokens: 0,
            });
            Err(e.to_string())
        }
    }
}

#[tauri::command]
async fn cancel_search(state: State<'_, AppState>) -> Result<(), String> {
    state.cancelled.store(true, Ordering::SeqCst);
    state.progress.lock().map_err(|e| e.to_string())?.push(types::ProgressEvent {
        phase: "cancelled".into(),
        message: "用户取消了搜索".into(),
        percent: 0,
        detail: String::new(),
        tokens: 0,
    });
    Ok(())
}

#[tauri::command]
async fn export_papers(papers: Vec<types::ScoredPaper>, format: String) -> Result<String, String> {
    match format.as_str() {
        "bibtex" => {
            let mut out = String::new();
            for (i, sp) in papers.iter().enumerate() {
                let p = &sp.paper;
                let key = format!(
                    "{}{}",
                    p.authors.first().map(|a| a.split_whitespace().last().unwrap_or("unknown")).unwrap_or("unknown"),
                    p.year
                );
                out.push_str(&format!(
                    "@article{{{}_{},\n  title = {{{}}},\n  author = {{{}}},\n  year = {{{}}},\n  journal = {{{}}},\n  doi = {{{}}},\n  url = {{{}}}\n}}\n\n",
                    key, i,
                    p.title,
                    p.authors.join(" and "),
                    p.year,
                    p.venue,
                    p.doi,
                    p.url
                ));
            }
            Ok(out)
        }
        "markdown" => {
            let mut out = String::from("| # | Title | Authors | Year | Venue | Citations | Score |\n");
            out.push_str("|---|-------|---------|------|-------|-----------|-------|\n");
            for (i, sp) in papers.iter().enumerate() {
                let p = &sp.paper;
                out.push_str(&format!(
                    "| {} | {} | {} | {} | {} | {} | {} |\n",
                    i + 1,
                    p.title,
                    p.authors.first().map(|s| s.as_str()).unwrap_or(""),
                    p.year,
                    p.venue,
                    p.citation_count,
                    sp.score
                ));
            }
            Ok(out)
        }
        _ => Err("Unsupported format. Use 'bibtex' or 'markdown'.".into()),
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
            cancelled: Arc::new(AtomicBool::new(false)),
        })
        .invoke_handler(tauri::generate_handler![
            search, refine_search, cancel_search, export_papers, get_progress, get_config, update_config,
            get_conversations, delete_conversation
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
