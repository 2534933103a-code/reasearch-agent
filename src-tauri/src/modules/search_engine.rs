use crate::backends::llm::LlmBackend;
use crate::backends::search::SearchBackend;
use crate::types::{Paper, QueryPlan};
use std::collections::HashSet;

pub struct SearchEngine;

impl SearchEngine {
    pub async fn phase1_broad_search(
        backend: &dyn SearchBackend,
        plan: &QueryPlan,
        max_per_query: usize,
    ) -> Result<Vec<Paper>, anyhow::Error> {
        let mut all_papers: Vec<Paper> = Vec::new();
        let mut seen_dois: HashSet<String> = HashSet::new();

        for sq in &plan.sub_queries {
            let papers = backend.search(&sq.query, max_per_query).await?;
            for paper in papers {
                let dedup_key = if !paper.doi.is_empty() && paper.doi != "N/A" {
                    paper.doi.clone()
                } else {
                    paper.title.to_lowercase()
                };

                if seen_dois.insert(dedup_key) {
                    all_papers.push(paper);
                }
            }
        }

        Ok(all_papers)
    }

    pub async fn phase2_iterate(
        llm: &LlmBackend,
        backend: &dyn SearchBackend,
        current_papers: &[Paper],
        plan: &QueryPlan,
        _round: u32,
        max_per_query: usize,
    ) -> Result<Vec<Paper>, anyhow::Error> {
        let paper_summaries: Vec<String> = current_papers
            .iter()
            .take(30)
            .map(|p| {
                format!(
                    "- {} ({}): {}",
                    p.title,
                    p.year,
                    &p.abstract_text[..200.min(p.abstract_text.len())]
                )
            })
            .collect();

        let system = "你是一个学术文献检索专家。分析已有论文列表，找出遗漏的研究子主题，生成1-2个新的搜索关键词。输出JSON: {\"new_queries\": [\"query1\", \"query2\"]}";
        let user_prompt = format!(
            "原始查询: {}\n已有论文(部分):\n{}\n请找出遗漏的子主题，生成新的搜索关键词。",
            plan.original,
            paper_summaries.join("\n")
        );

        let response = llm.chat(system, &user_prompt).await?;
        let json: serde_json::Value = serde_json::from_str(&response)?;

        let new_queries: Vec<String> = json["new_queries"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let mut new_papers = Vec::new();
        for q in &new_queries {
            let results = backend.search(q, max_per_query).await?;
            new_papers.extend(results);
        }

        Ok(new_papers)
    }

    pub async fn citation_expansion(
        backend: &dyn SearchBackend,
        top_papers: &[&Paper],
    ) -> Result<Vec<Paper>, anyhow::Error> {
        let mut expanded = Vec::new();

        for paper in top_papers.iter().take(10) {
            if let Ok(refs) = backend.get_references(&paper.id, 10).await {
                expanded.extend(refs);
            }
            if paper.citation_count > 100 {
                if let Ok(cited) = backend.get_cited_papers(&paper.id, 10).await {
                    expanded.extend(cited);
                }
            }
        }

        Ok(expanded)
    }

    pub fn merge_dedup(existing: &mut Vec<Paper>, new: Vec<Paper>) {
        let seen_dois: HashSet<String> = existing
            .iter()
            .map(|p| {
                if !p.doi.is_empty() && p.doi != "N/A" {
                    p.doi.clone()
                } else {
                    p.title.to_lowercase()
                }
            })
            .collect();

        for paper in new {
            let key = if !paper.doi.is_empty() && paper.doi != "N/A" {
                paper.doi.clone()
            } else {
                paper.title.to_lowercase()
            };

            if !seen_dois.contains(&key) {
                existing.push(paper);
            }
        }
    }
}
