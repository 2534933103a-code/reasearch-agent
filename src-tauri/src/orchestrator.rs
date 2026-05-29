use crate::backends::llm::LlmBackend;
use crate::backends::search::SearchBackend;
use crate::modules::query_decomposer::QueryDecomposer;
use crate::modules::ranker::Ranker;
use crate::modules::result_organizer::ResultOrganizer;
use crate::modules::search_engine::SearchEngine;
use crate::types::{Paper, ProgressEvent, SearchResult};
use tauri::Emitter;

fn event(phase: &str, message: &str, percent: u8, detail: &str) -> ProgressEvent {
    ProgressEvent { phase: phase.into(), message: message.into(), percent, detail: detail.into() }
}

pub struct Orchestrator;

impl Orchestrator {
    pub async fn run<R: tauri::Runtime, W: Emitter<R>>(
        window: &W,
        llm: &LlmBackend,
        search_backend: &dyn SearchBackend,
        query: String,
        config: &crate::config::AppConfig,
    ) -> Result<SearchResult, anyhow::Error> {
        // Step 1: Query Decomposition
        window.emit("progress", event("decompose", "正在解析查询意图...", 5, ""))?;

        let plan = QueryDecomposer::decompose(llm, &query).await?;
        let sq_list: Vec<String> = plan.sub_queries.iter()
            .map(|sq| format!("{} [{}]", sq.query, sq.dimension))
            .collect();
        let detail_json = serde_json::json!({
            "sub_queries": sq_list,
            "constraints": format!("{:?}", plan.constraints.year_range)
        });
        window.emit("progress", event("decompose_done",
            &format!("查询已分解为 {} 个子句", plan.sub_queries.len()),
            12,
            &detail_json.to_string()
        ))?;

        // Step 2: Phase 1 — Broad Search
        window.emit("progress", event("search",
            &format!("并行搜索 {} 个子查询...", plan.sub_queries.len()),
            15,
            &serde_json::json!({"parallel_queries": sq_list}).to_string()
        ))?;

        let mut candidate_pool = SearchEngine::phase1_broad_search(
            search_backend, &plan, config.search.max_results_per_query,
        ).await?;

        window.emit("progress", event("search_done",
            &format!("首轮搜索完成，收集 {} 篇候选", candidate_pool.len()),
            30,
            &serde_json::json!({"found": candidate_pool.len()}).to_string()
        ))?;

        // Step 3: Phase 2 — Iterative Refinement
        let max_rounds = config.search.max_rounds.min(3);
        let mut actual_rounds = 0;
        for round in 1..=max_rounds {
            if candidate_pool.len() >= 100 { break; }

            window.emit("progress", event("refine",
                &format!("精细化检索第 {}/{} 轮 — AI 正在分析缺失方向...", round, max_rounds),
                (35 + (round * 10) as u32) as u8,
                ""
            ))?;

            let new_papers = SearchEngine::phase2_iterate(
                llm, search_backend, &candidate_pool, &plan, round,
                config.search.max_results_per_query,
            ).await?;

            let before = candidate_pool.len();
            SearchEngine::merge_dedup(&mut candidate_pool, new_papers);
            let after = candidate_pool.len();
            actual_rounds = round;

            window.emit("progress", event("refine_done",
                &format!("第 {} 轮完成: {} → {} 篇", round, before, after),
                (40 + (round * 12) as u32) as u8,
                &serde_json::json!({"before": before, "after": after, "added": after - before}).to_string()
            ))?;

            if after - before < 3 { break; }
        }

        // Step 4: Citation Expansion
        window.emit("progress", event("cite_expand",
            "追踪引用关系 — 补充被引论文...", 65,
            &serde_json::json!({"top_papers": candidate_pool.iter().take(5).map(|p| &p.title).collect::<Vec<_>>()}).to_string()
        ))?;

        let top_refs: Vec<&Paper> = candidate_pool.iter().take(10).collect();
        let cited = SearchEngine::citation_expansion(search_backend, &top_refs).await?;
        let before_cite = candidate_pool.len();
        SearchEngine::merge_dedup(&mut candidate_pool, cited);
        candidate_pool.truncate(100);

        window.emit("progress", event("cite_done",
            &format!("引用扩展完成: {} → {} 篇", before_cite, candidate_pool.len()),
            68,
            &serde_json::json!({"before": before_cite, "after": candidate_pool.len()}).to_string()
        ))?;

        // Step 5: Ranking
        let batch_count = (candidate_pool.len() as f64 / 20.0).ceil() as usize;
        window.emit("progress", event("rank",
            &format!("AI 正在为 {} 篇论文打分 (分 {} 批)...", candidate_pool.len(), batch_count),
            70,
            &serde_json::json!({"candidates": candidate_pool.len(), "batches": batch_count}).to_string()
        ))?;

        let filtered = Ranker::fast_filter(candidate_pool, &plan);
        let scored = Ranker::llm_score(llm, &filtered, &query).await?;
        let tiers = Ranker::partition(scored);

        let high_n = tiers.high_relevance.len();
        let partial_n = tiers.partial_relevance.len();

        window.emit("progress", event("rank_done",
            &format!("打分完成 — 高度相关 {} 篇，部分相关 {} 篇", high_n, partial_n),
            85,
            &serde_json::json!({"high": high_n, "partial": partial_n}).to_string()
        ))?;

        // Step 6: Results
        window.emit("progress", event("organize",
            "AI 正在生成搜索摘要...", 90, ""))?;

        let summary = ResultOrganizer::generate_summary(llm, &tiers, &query).await?;
        let total = high_n + partial_n;

        window.emit("progress", event("done",
            &format!("搜索完成，共找到 {} 篇相关论文", total),
            100,
            &serde_json::json!({"high": high_n, "partial": partial_n, "rounds": actual_rounds}).to_string()
        ))?;

        Ok(ResultOrganizer::assemble_result(summary, tiers, total, actual_rounds))
    }
}
