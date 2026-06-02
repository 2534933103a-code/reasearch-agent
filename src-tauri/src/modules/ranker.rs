use crate::backends::llm::LlmBackend;
use crate::types::{Paper, QueryPlan, ScoredPaper, TieredResults};
use serde_json::Value;

pub struct Ranker;

impl Ranker {
    pub fn fast_filter(
        papers: Vec<Paper>,
        plan: &QueryPlan,
    ) -> Vec<Paper> {
        let keywords: Vec<String> = plan
            .sub_queries
            .iter()
            .flat_map(|sq| sq.query.split_whitespace().map(|s| s.to_lowercase()))
            .collect();

        papers
            .into_iter()
            .filter(|p| {
                if let Some((start, end)) = plan.constraints.year_range {
                    if p.year < start || p.year > end {
                        return false;
                    }
                }
                let text = format!("{} {}", p.title.to_lowercase(), p.abstract_text.to_lowercase());
                keywords.iter().any(|kw| text.contains(kw))
            })
            .collect()
    }

    pub async fn llm_score(
        llm: &LlmBackend,
        papers: &[Paper],
        original_query: &str,
    ) -> Result<(Vec<ScoredPaper>, u32), anyhow::Error> {
        let mut scored = Vec::new();
        let mut total_tokens = 0u32;
        let batch_size = 20;

        for chunk in papers.chunks(batch_size) {
            let papers_text: Vec<String> = chunk
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    format!(
                        "[{}] Title: {}\nAuthors: {}\nYear: {}\nVenue: {}\nAbstract: {}",
                        i, p.title,
                        p.authors.first().map(|s| s.as_str()).unwrap_or("Unknown"),
                        p.year, p.venue,
                        p.abstract_text
                    )
                })
                .collect();

            let system = r#"你对每篇论文与查询的相关性打分1-10。输出JSON格式:
{"scores": [{"index": 0, "score": 8, "rationale": "理由"}, ...]}"#;

            let user_prompt = format!(
                "原始查询: {}\n论文列表:\n{}",
                original_query,
                papers_text.join("\n\n")
            );

            let resp = llm.chat(system, &user_prompt).await?;
            total_tokens += resp.tokens;
            let content = resp.content.unwrap_or_default();
            let json: Value = serde_json::from_str(&content)?;

            if let Some(scores) = json["scores"].as_array() {
                for entry in scores {
                    let idx = entry["index"].as_u64().unwrap_or(0) as usize;
                    if idx < chunk.len() {
                        scored.push(ScoredPaper {
                            paper: chunk[idx].clone(),
                            score: entry["score"].as_u64().unwrap_or(5) as u8,
                            rationale: entry["rationale"].as_str().unwrap_or("").to_string(),
                        });
                    }
                }
            }
        }

        Ok((scored, total_tokens))
    }

    pub fn partition(scored: Vec<ScoredPaper>) -> TieredResults {
        let mut high = Vec::new();
        let mut partial = Vec::new();

        for sp in scored {
            if sp.score >= 7 {
                high.push(sp);
            } else if sp.score >= 4 {
                partial.push(sp);
            }
        }

        high.sort_by(|a, b| b.score.cmp(&a.score));
        partial.sort_by(|a, b| b.score.cmp(&a.score));

        TieredResults {
            high_relevance: high,
            partial_relevance: partial,
        }
    }
}
