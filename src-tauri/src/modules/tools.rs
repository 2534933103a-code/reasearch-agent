use crate::backends::search::SearchBackend;
use crate::types::{Paper, ToolDef};
use serde_json::Value;
use std::collections::HashSet;

// ── Tool Definitions ──

pub fn agent_tools() -> Vec<ToolDef> {
    vec![
        ToolDef::new(
            "search_papers",
            "Search for academic papers by keywords. Returns numbered papers with titles, authors, years, venues, abstracts, and citation counts. Use the index numbers in brackets (e.g., [0], [5]) to remember papers.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "English keywords for the search query. Be specific and use academic terminology."
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum number of results to return (default 15, max 20)",
                        "default": 15
                    }
                },
                "required": ["query"]
            }),
        ),
        ToolDef::new(
            "get_cited_papers",
            "Get papers that cite a specific paper (forward citation). Reference the paper by its index number from previous search results.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "paper_index": {
                        "type": "integer",
                        "description": "The index number of the paper shown in brackets [N] in previous results"
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum results (default 10)",
                        "default": 10
                    }
                },
                "required": ["paper_index"]
            }),
        ),
        ToolDef::new(
            "get_references",
            "Get papers that a specific paper cites (backward citation). Reference the paper by its index number from previous search results.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "paper_index": {
                        "type": "integer",
                        "description": "The index number of the paper shown in brackets [N] in previous results"
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum results (default 10)",
                        "default": 10
                    }
                },
                "required": ["paper_index"]
            }),
        ),
        ToolDef::new(
            "clarify_query",
            "Present 3-5 specific research directions when the query is ambiguous. Use at ANY stage:\n- Before search: vague queries like '鲜花', '电池', 'AI'\n- During search: if results span unrelated fields — stop and clarify\n- After search: when papers cluster into distinct groups — offer each as a focused direction\n\nBase the options on actual search results when available, not generic guesses. Each option should be a specific research direction, in Chinese.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "Explanation of why the query needs clarification, in Chinese"
                    },
                    "options": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "3-5 specific research directions, each in Chinese, e.g. '无人机自主导航与路径规划'"
                    }
                },
                "required": ["message", "options"]
            }),
        ),
        ToolDef::new(
            "finish_search",
            "Call this when you have gathered enough relevant papers. Provide a brief summary and optionally drop irrelevant papers.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "Brief assessment of search coverage and key findings"
                    },
                    "drop_indices": {
                        "type": "array",
                        "items": {"type": "integer"},
                        "description": "List of paper indices to REMOVE from the pool (papers that don't match the refinement). Only include indices you are confident should be dropped."
                    }
                },
                "required": ["message"]
            }),
        ),
    ]
}

// ── Tool Execution ──

fn make_key(p: &Paper) -> String {
    if !p.doi.is_empty() && p.doi != "N/A" {
        p.doi.clone()
    } else {
        p.id.clone()
    }
}

/// Format a paper for display to the LLM
fn format_paper(p: &Paper, index: usize) -> String {
    let authors = if p.authors.len() > 3 {
        format!("{} et al.", p.authors.first().map(|s| s.as_str()).unwrap_or("Unknown"))
    } else {
        p.authors.join(", ")
    };
    let abstract_snippet: String = p.abstract_text.chars().take(250).collect();
    let trunc = if p.abstract_text.len() > 250 { "..." } else { "" };
    format!(
        "[{}] {} ({})\n  Authors: {} | Venue: {} | Citations: {}\n  Abstract: {}{}",
        index, p.title, p.year, authors, p.venue, p.citation_count,
        abstract_snippet, trunc
    )
}

/// Format search results for the LLM context
pub fn format_search_results(query: &str, papers: &[Paper], start_idx: usize) -> String {
    if papers.is_empty() {
        return format!("Search for \"{}\" returned no results. Try different keywords.", query);
    }
    let header = format!(
        "Search for \"{}\" returned {} papers:\n",
        query, papers.len()
    );
    let entries: Vec<String> = papers
        .iter()
        .enumerate()
        .map(|(i, p)| format_paper(p, start_idx + i))
        .collect();
    header + &entries.join("\n\n")
}

/// Execute a tool call. `paper_list` is the ordered list of unique papers.
/// `seen_keys` is used for deduplication.
/// Returns the result string to send back to the LLM.
pub async fn execute_tool(
    name: &str,
    arguments: &str,
    backend: &dyn SearchBackend,
    paper_list: &mut Vec<Paper>,
    seen_keys: &mut HashSet<String>,
    search_calls: &mut u32,
) -> Result<String, anyhow::Error> {
    let args: Value = serde_json::from_str(arguments).unwrap_or(Value::Null);

    match name {
        "search_papers" => {
            let query = args["query"].as_str().unwrap_or("");
            let max_results = args["max_results"].as_u64().unwrap_or(15).min(20) as usize;

            *search_calls += 1;
            let papers = backend.search(query, max_results).await?;

            let start_idx = paper_list.len();
            let mut added = Vec::new();
            for paper in &papers {
                let key = make_key(paper);
                if seen_keys.insert(key) {
                    paper_list.push(paper.clone());
                    added.push(paper.clone());
                }
            }

            Ok(format_search_results(query, &added, start_idx))
        }

        "get_cited_papers" => {
            let paper_index = args["paper_index"].as_u64().unwrap_or(0) as usize;
            let max_results = args["max_results"].as_u64().unwrap_or(10).min(15) as usize;

            let paper_id = match paper_list.get(paper_index) {
                Some(p) => p.id.clone(),
                None => return Ok(format!("Error: paper index {} not found. Available indices: 0-{}", paper_index, paper_list.len().saturating_sub(1))),
            };

            *search_calls += 1;
            let papers = backend.get_cited_papers(&paper_id, max_results).await?;

            let start_idx = paper_list.len();
            let mut added = Vec::new();
            for paper in &papers {
                let key = make_key(paper);
                if seen_keys.insert(key) {
                    paper_list.push(paper.clone());
                    added.push(paper.clone());
                }
            }

            Ok(format_search_results(
                &format!("papers citing [{}]", paper_index),
                &added, start_idx,
            ))
        }

        "get_references" => {
            let paper_index = args["paper_index"].as_u64().unwrap_or(0) as usize;
            let max_results = args["max_results"].as_u64().unwrap_or(10).min(15) as usize;

            let paper_id = match paper_list.get(paper_index) {
                Some(p) => p.id.clone(),
                None => return Ok(format!("Error: paper index {} not found. Available indices: 0-{}", paper_index, paper_list.len().saturating_sub(1))),
            };

            *search_calls += 1;
            let papers = backend.get_references(&paper_id, max_results).await?;

            let start_idx = paper_list.len();
            let mut added = Vec::new();
            for paper in &papers {
                let key = make_key(paper);
                if seen_keys.insert(key) {
                    paper_list.push(paper.clone());
                    added.push(paper.clone());
                }
            }

            Ok(format_search_results(
                &format!("papers referenced by [{}]", paper_index),
                &added, start_idx,
            ))
        }

        _ => Ok(format!("Unknown tool: {}", name)),
    }
}

/// Parse clarify_query arguments
pub fn parse_clarify(arguments: &str) -> Result<(String, Vec<String>), anyhow::Error> {
    let args: Value = serde_json::from_str(arguments)?;
    let message = args["message"].as_str().unwrap_or("请选择研究方向:").to_string();
    let options: Vec<String> = args["options"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    Ok((message, options))
}

/// Parse finish_search arguments — message + optional drop list
pub fn parse_finish_message(arguments: &str) -> Result<(String, Vec<usize>), anyhow::Error> {
    let args: Value = serde_json::from_str(arguments)?;
    let message = args["message"].as_str().unwrap_or("Search completed.").to_string();
    let drop_indices: Vec<usize> = args["drop_indices"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_u64().map(|n| n as usize)).collect())
        .unwrap_or_default();
    Ok((message, drop_indices))
}
