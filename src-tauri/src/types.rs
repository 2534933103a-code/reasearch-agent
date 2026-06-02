use serde::{Deserialize, Serialize};

// ── LLM multi-turn message (for agentic conversations) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmMessage {
    pub role: String, // "system" | "user" | "assistant" | "tool"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl LlmMessage {
    pub fn system(content: &str) -> Self {
        Self { role: "system".into(), content: Some(content.into()), tool_calls: None, tool_call_id: None }
    }
    pub fn user(content: &str) -> Self {
        Self { role: "user".into(), content: Some(content.into()), tool_calls: None, tool_call_id: None }
    }
    pub fn assistant(content: &str) -> Self {
        Self { role: "assistant".into(), content: Some(content.into()), tool_calls: None, tool_call_id: None }
    }
    pub fn assistant_with_tools(tool_calls: Vec<ToolCall>) -> Self {
        Self { role: "assistant".into(), content: None, tool_calls: Some(tool_calls), tool_call_id: None }
    }
    pub fn tool_result(call_id: String, content: String) -> Self {
        Self { role: "tool".into(), content: Some(content), tool_calls: None, tool_call_id: Some(call_id) }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String, // "function"
    pub function: ToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: String, // JSON string
}

// Tool definition sent to LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    #[serde(rename = "type")]
    pub tool_type: String, // always "function"
    pub function: ToolFnDef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFnDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value, // JSON Schema
}

impl ToolDef {
    pub fn new(name: &str, description: &str, parameters: serde_json::Value) -> Self {
        Self {
            tool_type: "function".into(),
            function: ToolFnDef { name: name.into(), description: description.into(), parameters },
        }
    }
}

// ── Original types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubQuery {
    pub query: String,
    pub dimension: String,
    pub weight: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryConstraints {
    pub year_range: Option<(u32, u32)>,
    pub venues: Vec<String>,
    pub methodology_required: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryPlan {
    pub original: String,
    pub sub_queries: Vec<SubQuery>,
    pub constraints: QueryConstraints,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paper {
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub year: u32,
    pub venue: String,
    pub doi: String,
    pub abstract_text: String,
    pub citation_count: u32,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredPaper {
    pub paper: Paper,
    pub score: u8,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TieredResults {
    pub high_relevance: Vec<ScoredPaper>,
    pub partial_relevance: Vec<ScoredPaper>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    #[serde(default)]
    pub conversation_id: String,
    pub summary: String,
    pub tiers: TieredResults,
    pub total_candidates: usize,
    pub rounds_used: u32,
    #[serde(default)]
    pub needs_clarification: bool,
    #[serde(default)]
    pub clarification_options: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressEvent {
    pub phase: String,
    pub message: String,
    pub percent: u8,
    pub detail: String,
    pub tokens: u32,  // cumulative total tokens used so far
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub source: usize,
    pub target: usize,
    pub relation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub index: usize,
    pub title: String,
    pub cluster: u8,
}

// ── Conversation ────────────────────────────────────
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub role: String,  // "user" | "assistant" | "result"
    pub content: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub messages: Vec<ConversationMessage>,
    #[serde(default)]
    pub search_results: Vec<SearchResult>,
    pub created_at: u64,
}
