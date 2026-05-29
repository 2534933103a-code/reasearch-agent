use crate::backends::llm::LlmBackend;
use crate::backends::search::SearchBackend;
use crate::modules::query_decomposer::QueryDecomposer;
use crate::modules::ranker::Ranker;
use crate::modules::result_organizer::ResultOrganizer;
use crate::modules::search_engine::SearchEngine;
use crate::types::{Paper, ProgressEvent, SearchResult};
use tauri::Emitter;

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
        window.emit("progress", ProgressEvent {
            phase: "decompose".into(),
            message: "正在解析查询意图...".into(),
            percent: 5,
        })?;

        let plan = QueryDecomposer::decompose(llm, &query).await?;

        // Step 2: Phase 1 — Broad Search
        window.emit("progress", ProgressEvent {
            phase: "search".into(),
            message: format!("正在搜索 (共{}个子查询)...", plan.sub_queries.len()),
            percent: 15,
        })?;

        let mut candidate_pool = SearchEngine::phase1_broad_search(
            search_backend,
            &plan,
            config.search.max_results_per_query,
        )
        .await?;

        window.emit("progress", ProgressEvent {
            phase: "search".into(),
            message: format!("首轮搜索完成，找到 {} 篇候选论文", candidate_pool.len()),
            percent: 30,
        })?;

        // Step 3: Phase 2 — Iterative Refinement
        let max_rounds = config.search.max_rounds.min(3);
        for round in 1..=max_rounds {
            if candidate_pool.len() >= 100 {
                break;
            }

            window.emit("progress", ProgressEvent {
                phase: "refine".into(),
                message: format!("正在精细化搜索 (第{}/{}轮)...", round, max_rounds),
                percent: (35 + (round * 10) as u32) as u8,
            })?;

            let new_papers = SearchEngine::phase2_iterate(
                llm,
                search_backend,
                &candidate_pool,
                &plan,
                round,
                config.search.max_results_per_query,
            )
            .await?;

            let before = candidate_pool.len();
            SearchEngine::merge_dedup(&mut candidate_pool, new_papers);
            let after = candidate_pool.len();

            if after - before < 3 {
                break;
            }
        }

        // Step 4: Citation Expansion
        window.emit("progress", ProgressEvent {
            phase: "cite_expand".into(),
            message: "正在追踪引用关系...".into(),
            percent: 65,
        })?;

        let top_refs: Vec<&Paper> = candidate_pool.iter().take(10).collect();
        let cited = SearchEngine::citation_expansion(search_backend, &top_refs).await?;
        SearchEngine::merge_dedup(&mut candidate_pool, cited);

        // Cap at 100
        candidate_pool.truncate(100);

        window.emit("progress", ProgressEvent {
            phase: "rank".into(),
            message: format!("候选池共 {} 篇，正在排序...", candidate_pool.len()),
            percent: 70,
        })?;

        // Step 5: Ranking
        let filtered = Ranker::fast_filter(candidate_pool, &plan);
        let scored = Ranker::llm_score(llm, &filtered, &query).await?;
        let tiers = Ranker::partition(scored);

        // Step 6: Results
        window.emit("progress", ProgressEvent {
            phase: "organize".into(),
            message: "正在生成搜索报告...".into(),
            percent: 90,
        })?;

        let summary = ResultOrganizer::generate_summary(llm, &tiers, &query).await?;
        let total = tiers.high_relevance.len() + tiers.partial_relevance.len();

        window.emit("progress", ProgressEvent {
            phase: "done".into(),
            message: format!("搜索完成，共找到 {} 篇相关论文", total),
            percent: 100,
        })?;

        Ok(ResultOrganizer::assemble_result(summary, tiers, total, max_rounds))
    }
}
