use serde::{Deserialize, Serialize};

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
    pub summary: String,
    pub tiers: TieredResults,
    pub total_candidates: usize,
    pub rounds_used: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressEvent {
    pub phase: String,
    pub message: String,
    pub percent: u8,
    pub detail: String,
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
    pub search_result: Option<SearchResult>,
    pub created_at: u64,
}
