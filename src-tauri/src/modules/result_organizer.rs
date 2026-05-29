use crate::backends::llm::LlmBackend;
use crate::types::{GraphData, GraphEdge, GraphNode, SearchResult, TieredResults};

pub struct ResultOrganizer;

impl ResultOrganizer {
    pub async fn generate_summary(
        llm: &LlmBackend,
        tiers: &TieredResults,
        original_query: &str,
    ) -> Result<(String, u32), anyhow::Error> {
        let high_count = tiers.high_relevance.len();
        let partial_count = tiers.partial_relevance.len();

        let high_sample: Vec<String> = tiers
            .high_relevance
            .iter()
            .take(10)
            .map(|sp| format!("- {} ({})", sp.paper.title, sp.paper.year))
            .collect();

        let system = "你是一个学术研究助手。请根据搜索结果，用2-3句话总结主要发现的研究方向和代表论文。用中文输出。";
        let user_prompt = format!(
            "查询: {}\n高度相关({}):\n{}\n部分相关({})\n请总结主要研究方向。",
            original_query, high_count, high_sample.join("\n"), partial_count
        );

        let resp = llm.chat_text(system, &user_prompt).await?;
        Ok((resp.content, resp.tokens))
    }

    pub fn build_graph(tiers: &TieredResults) -> GraphData {
        let all_papers: Vec<&crate::types::ScoredPaper> = tiers
            .high_relevance
            .iter()
            .chain(tiers.partial_relevance.iter())
            .collect();

        let nodes: Vec<GraphNode> = all_papers
            .iter()
            .enumerate()
            .map(|(i, sp)| GraphNode {
                index: i,
                title: sp.paper.title.clone(),
                cluster: if sp.score >= 7 { 0 } else { 1 },
            })
            .collect();

        let mut edges = Vec::new();
        for i in 0..nodes.len() {
            for j in (i + 1)..nodes.len() {
                let ti_words: Vec<&str> = nodes[i].title.split_whitespace().collect();
                let tj_words: Vec<&str> = nodes[j].title.split_whitespace().collect();
                let common = ti_words
                    .iter()
                    .filter(|w| w.len() > 3 && tj_words.contains(w))
                    .count();
                if common >= 3 {
                    edges.push(GraphEdge {
                        source: i,
                        target: j,
                        relation: "topic_similarity".into(),
                    });
                }
            }
        }

        GraphData { nodes, edges }
    }

    pub fn assemble_result(
        summary: String,
        tiers: TieredResults,
        total_candidates: usize,
        rounds_used: u32,
    ) -> SearchResult {
        SearchResult {
            summary,
            tiers,
            total_candidates,
            rounds_used,
        }
    }
}
