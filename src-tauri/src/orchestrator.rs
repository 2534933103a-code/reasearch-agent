use crate::backends::llm::LlmBackend;
use crate::backends::search::SearchBackend;
use crate::modules::query_decomposer::QueryDecomposer;
use crate::modules::ranker::Ranker;
use crate::modules::result_organizer::ResultOrganizer;
use crate::modules::search_engine::SearchEngine;
use crate::types::{Paper, ProgressEvent, SearchResult};
use std::sync::{Arc, Mutex};

fn ev(phase: &str, message: &str, percent: u8, detail: &str, tokens: u32) -> ProgressEvent {
    ProgressEvent { phase: phase.into(), message: message.into(), percent, detail: detail.into(), tokens }
}

fn push(progress: &Arc<Mutex<Vec<ProgressEvent>>>, e: ProgressEvent) {
    if let Ok(mut v) = progress.lock() { v.push(e); }
}

pub struct Orchestrator;

impl Orchestrator {
    pub async fn run(
        llm: &LlmBackend,
        search_backend: &dyn SearchBackend,
        query: String,
        config: &crate::config::AppConfig,
        progress: &Arc<Mutex<Vec<ProgressEvent>>>,
    ) -> Result<SearchResult, anyhow::Error> {
        let mut tokens: u32 = 0;

        // Step 1: Query Decomposition
        push(progress, ev("decompose", "正在解析查询意图 (调用 LLM)...", 5, "", tokens));
        let (plan, t) = QueryDecomposer::decompose(llm, &query).await?;
        tokens += t;

        let sq_list: Vec<String> = plan.sub_queries.iter()
            .map(|sq| format!("{} [{}]", sq.query, sq.dimension)).collect();
        push(progress, ev("decompose_done",
            &format!("查询分解完成 — {} 个子句", plan.sub_queries.len()),
            12, &serde_json::json!({"sub_queries": sq_list}).to_string(), tokens));

        // Step 2: Phase 1 — Broad Search
        push(progress, ev("search",
            &format!("并行搜索 {} 个子查询...", plan.sub_queries.len()), 15, "", tokens));
        push(progress, ev("search_detail",
            &format!("搜索词: {}", plan.sub_queries.iter().map(|s| s.query.as_str()).collect::<Vec<_>>().join(", ")),
            16, "", tokens));

        let mut candidate_pool = SearchEngine::phase1_broad_search(
            search_backend, &plan, config.search.max_results_per_query).await?;

        push(progress, ev("search_done",
            &format!("首轮搜索完成 — {} 篇候选", candidate_pool.len()),
            30, &serde_json::json!({"found": candidate_pool.len()}).to_string(), tokens));

        // Step 3: Phase 2 — Iterative Refinement
        let max_rounds = config.search.max_rounds.min(3);
        let mut actual_rounds = 0;
        for round in 1..=max_rounds {
            if candidate_pool.len() >= 100 { break; }
            push(progress, ev("refine",
                &format!("精细化检索第 {}/{} 轮 — LLM 分析缺失方向...", round, max_rounds),
                (35 + (round * 10) as u32) as u8, "", tokens));

            let (new_papers, t) = SearchEngine::phase2_iterate(
                llm, search_backend, &candidate_pool, &plan, round, config.search.max_results_per_query).await?;
            tokens += t;

            let before = candidate_pool.len();
            SearchEngine::merge_dedup(&mut candidate_pool, new_papers);
            let after = candidate_pool.len();
            actual_rounds = round;

            push(progress, ev("refine_done",
                &format!("第 {} 轮完成: {} → {} 篇", round, before, after),
                (40 + (round * 12) as u32) as u8,
                &serde_json::json!({"before": before, "after": after, "added": after - before}).to_string(), tokens));
            if after - before < 3 { break; }
        }

        // Step 4: Citation Expansion
        push(progress, ev("cite_expand", "追踪引用关系...", 65,
            &serde_json::json!({"top": candidate_pool.iter().take(3).map(|p| &p.title).collect::<Vec<_>>()}).to_string(), tokens));
        let top_refs: Vec<&Paper> = candidate_pool.iter().take(10).collect();
        let cited = SearchEngine::citation_expansion(search_backend, &top_refs).await?;
        let before_cite = candidate_pool.len();
        SearchEngine::merge_dedup(&mut candidate_pool, cited);
        candidate_pool.truncate(100);
        push(progress, ev("cite_done",
            &format!("引用扩展: {} → {} 篇", before_cite, candidate_pool.len()), 68, "", tokens));

        // Step 5: Ranking
        let batch_count = (candidate_pool.len() as f64 / 20.0).ceil() as usize;
        push(progress, ev("rank",
            &format!("AI 为 {} 篇论文打分 ({} 批)...", candidate_pool.len(), batch_count), 70, "", tokens));

        let filtered = Ranker::fast_filter(candidate_pool, &plan);
        let (scored, t) = Ranker::llm_score(llm, &filtered, &query).await?;
        tokens += t;
        let tiers = Ranker::partition(scored);

        let high_n = tiers.high_relevance.len();
        let partial_n = tiers.partial_relevance.len();
        push(progress, ev("rank_done",
            &format!("评分完成 — 高度相关 {} · 部分相关 {}", high_n, partial_n), 85,
            &serde_json::json!({"high": high_n, "partial": partial_n}).to_string(), tokens));

        // Step 6: Generate summary
        push(progress, ev("organize", "AI 生成搜索摘要...", 90, "", tokens));
        let (summary, t) = ResultOrganizer::generate_summary(llm, &tiers, &query).await?;
        tokens += t;

        let total = high_n + partial_n;
        push(progress, ev("done",
            &format!("完成 — {} 篇相关论文 · 累计 Token: {}", total, tokens),
            100,
            &serde_json::json!({"high": high_n, "partial": partial_n, "rounds": actual_rounds, "tokens": tokens}).to_string(), tokens));

        Ok(ResultOrganizer::assemble_result(summary, tiers, total, actual_rounds))
    }
}
