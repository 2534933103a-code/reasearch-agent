use crate::backends::llm::LlmBackend;
use crate::backends::search::SearchBackend;
use crate::modules::ranker::Ranker;
use crate::modules::result_organizer::ResultOrganizer;
use crate::modules::tools;
use crate::types::{LlmMessage, Paper, ProgressEvent, ScoredPaper, SearchResult, TieredResults};
use std::collections::HashSet;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};

fn ev(phase: &str, message: &str, percent: u8, detail: &str, tokens: u32) -> ProgressEvent {
    ProgressEvent { phase: phase.into(), message: message.into(), percent, detail: detail.into(), tokens }
}

fn push(progress: &Arc<Mutex<Vec<ProgressEvent>>>, e: ProgressEvent) {
    if let Ok(mut v) = progress.lock() { v.push(e); }
}

/// Format a tool call's arguments into a human-readable Chinese description
fn fmt_tool_args(name: &str, args_json: &str) -> String {
    let args: serde_json::Value = serde_json::from_str(args_json).unwrap_or_default();
    match name {
        "search_papers" => {
            let query = args["query"].as_str().unwrap_or("?");
            let max = args["max_results"].as_u64().unwrap_or(15);
            format!("搜索关键词: 「{}」 (最多 {} 篇)", query, max)
        }
        "get_cited_papers" => {
            let idx = args["paper_index"].as_u64().unwrap_or(0);
            format!("查找引用了 [{}] 的论文", idx)
        }
        "get_references" => {
            let idx = args["paper_index"].as_u64().unwrap_or(0);
            format!("查找 [{}] 引用的论文", idx)
        }
        "finish_search" => "检索阶段完成，开始评分…".into(),
        _ => format!("{}", args_json),
    }
}

/// Count new papers added (from tool result string, format: "returned N papers:")
fn count_papers_in_result(result: &str) -> usize {
    result.split("returned ").nth(1)
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

const SYSTEM_PROMPT: &str = r#"You are an expert academic research assistant. Your task is to find research papers relevant to the user's query.

## Workflow

1. **Search strategically**: Start with 2-3 different keyword combinations covering different aspects (methodology, application, theory). Use English academic keywords.
2. **Review results**: Papers are shown with index numbers in brackets, like [0], [5], [12]. Use these indices to reference papers.
3. **Expand via citations**: For the most promising papers, use get_cited_papers(index) or get_references(index) with their index number.
4. **Iterate if needed**: If coverage is insufficient, search with alternative keywords.
5. **Finalize**: When you have gathered a good set of relevant papers (usually 20-50), call `finish_search` with a brief summary of what you found.

## Important

- **When to clarify**: If a query could refer to multiple distinct research areas, call `clarify_query`. This applies at ANY stage:
  - BEFORE searching: vague/ambiguous queries (e.g., "鲜花", "无人机", "区块链", "电池", "AI", "机器人")
  - DURING searching: if first search results span unrelated fields (e.g., "鲜花的分子生物学" vs "鲜花的保鲜技术" vs "鲜花的文化史")
  - AFTER searching: when papers cluster into distinct groups — present each group as an option
  A good pattern: do ONE quick search to gauge breadth. If results are diverse, stop and clarify with 3-5 specific directions based on what you actually found. For example, searching "鲜花" might reveal: ①鲜花保鲜与采后生理 ②花卉基因育种 ③鲜切花供应链 ④鲜花文化与社会学 ⑤花艺设计. Present these to the user. Better to clarify early than waste budget on unfocused search.
- You have a limited budget. Usually 3-5 rounds is optimal. Be strategic!
- **CRITICAL**: ALWAYS include a brief reasoning in your response text BEFORE calling tools. Even if just 1 sentence, explain what you are doing and why. Do NOT leave the content field empty — the user needs to follow your thinking. Example: "Round 2: The MoE papers so far focus on architecture. I'll now search for load balancing methods specifically."
- If a search returns no results, try different keywords.
- Prioritize papers with higher citation counts when assessing quality.
- The system will handle detailed scoring and ranking after you finish searching. Your job is to cast a wide, relevant net.
- **When the user message says REFINEMENT**: the user is adding constraints to their previous query. Interpret the refinement as a FILTER, not as a new search topic. Combine the original topic keywords with the refinement (e.g., year, method, venue) — never search the refinement text literally.
- **When you have existing papers AND a refinement**: use the `drop_indices` in finish_search to remove papers that no longer match the refinement criteria. For example, if the user says \"只看2025年\", drop all pre-2025 papers. This keeps the paper pool clean and focused."#;

pub struct Orchestrator;

impl Orchestrator {
    pub async fn run(
        llm: &LlmBackend,
        search_backend: &dyn SearchBackend,
        query: String,
        config: &crate::config::AppConfig,
        progress: &Arc<Mutex<Vec<ProgressEvent>>>,
        refinement: Option<&str>,
        existing_papers: &[Paper],
        cancelled: &AtomicBool,
    ) -> Result<SearchResult, anyhow::Error> {
        let max_llm_calls = config.budget.max_llm_calls;
        let max_search_calls = config.budget.max_search_calls;

        let mut llm_calls: u32 = 0;
        let mut search_calls: u32 = 0;
        let mut total_tokens: u32 = 0;

        // Ordered paper list + dedup set — pre-populate with existing papers
        let mut paper_list: Vec<Paper> = Vec::new();
        let mut seen_keys: HashSet<String> = HashSet::new();
        for p in existing_papers {
            let key = if !p.doi.is_empty() && p.doi != "N/A" { p.doi.clone() } else { p.id.clone() };
            if seen_keys.insert(key) {
                paper_list.push(p.clone());
            }
        }
        let preload_count = paper_list.len();

        // Build initial messages with optional refinement context
        let user_msg = if let Some(ref r) = refinement {
            if preload_count > 0 {
                // Build a quick summary of the existing pool
                let mut years: Vec<u32> = paper_list.iter().map(|p| p.year).filter(|&y| y > 0).collect();
                years.sort_unstable();
                let year_range = if years.len() > 1 {
                    format!("{} - {}", years.first().unwrap(), years.last().unwrap())
                } else if let Some(y) = years.first() {
                    format!("{}", y)
                } else { "未知".into() };

                let mut venue_counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
                for p in &paper_list {
                    *venue_counts.entry(p.venue.as_str()).or_default() += 1;
                }
                let mut top_venues: Vec<(&str, usize)> = venue_counts.into_iter().collect();
                top_venues.sort_by(|a, b| b.1.cmp(&a.1));
                let venue_summary: String = top_venues.iter().take(5)
                    .map(|(v, c)| format!("{} ({})", v, c))
                    .collect::<Vec<_>>().join(", ");

                format!(
                    "ORIGINAL QUERY: {}\n\nREFINEMENT: {}\n\nEXISTING POOL: {} papers\n  Years: {}\n  Top venues: {}\n\n⛔ MANDATORY — BEFORE calling any search tools:\n1. SCAN the existing pool against the refinement. Which papers match? Which don't?\n2. CALL finish_search with `drop_indices` to REMOVE papers that don't match. You MUST list at least some indices to drop if the refinement is a filter.\n3. Only AFTER dropping, if the remaining pool is too small, search for more.\n\nFor example, if refinement is \"只看2025年\" and the pool has pre-2025 papers, you MUST drop them BY INDEX NUMBER. Look at each paper's year shown in previous results. Be aggressive — a clean small pool is better than a noisy large one.",
                    query, r, preload_count, year_range, venue_summary
                )
            } else {
                format!(
                    "ORIGINAL QUERY: {}\n\nREFINEMENT: {}\n\nInterpret the refinement as a FILTER or CONSTRAINT. Search for papers about the ORIGINAL TOPIC, applying the refinement. For example:\n- \"只关注2025年\" → add year filter, use relevant keywords\n- \"只看并行策略\" → focus on the method mentioned\n- \"要顶会论文\" → prioritize top venues\n\nDo NOT search the refinement text literally. Generate English keywords that COMBINE the original topic with the refinement constraint.",
                    query, r
                )
            }
        } else {
            format!(
                "Research query: {}\n\nIf this query is ambiguous, do ONE quick search first to gauge the research landscape. Then, if results span distinct fields, CALL `clarify_query` with focused directions based on what you found. If the query is clearly specific, proceed with normal search. Before calling tools, briefly explain your strategy.",
                query
            )
        };

        let mut messages: Vec<LlmMessage> = vec![
            LlmMessage::system(SYSTEM_PROMPT),
            LlmMessage::user(&user_msg),
        ];

        let tools = tools::agent_tools();

        let start_msg = if preload_count > 0 {
            format!("细化搜索 — 复用 {} 篇已有论文 + 补充搜索", preload_count)
        } else {
            format!("开始分析查询: 「{}」", query)
        };
        push(progress, ev("agent_start", &start_msg, 5,
            &serde_json::json!({"query": query, "preloaded": preload_count}).to_string(),
            0));

        // ═══════════════════════════════════════════════
        // PHASE 1: Agentic Search Loop
        // ═══════════════════════════════════════════════
        let mut finish_data: Option<(String, Vec<usize>)> = None;
        loop {
            // Paper pool limit
            if paper_list.len() >= 80 {
                push(progress, ev("budget_warn",
                    &format!("论文池已达 {} 篇上限，要求 Agent 输出结果…", paper_list.len()),
                    48, "", total_tokens));
                messages.push(LlmMessage::user(
                    "Paper pool is full (80 papers). Please call finish_search NOW. Use drop_indices if some are irrelevant."
                ));
            }

            // Budget check
            if llm_calls >= max_llm_calls {
                push(progress, ev("budget_warn",
                    &format!("LLM 调用次数已达上限 ({})，结束搜索阶段…", max_llm_calls),
                    45, "", total_tokens));
                messages.push(LlmMessage::user(
                    "You have reached the maximum number of operations. Please call finish_search NOW."
                ));
            }
            if search_calls >= max_search_calls {
                push(progress, ev("budget_warn",
                    &format!("搜索调用次数已达上限 ({})，结束搜索阶段…", max_search_calls),
                    45, "", total_tokens));
                messages.push(LlmMessage::user(
                    "Search budget exhausted. Please call finish_search NOW."
                ));
            }

            // Check for cancellation
            if cancelled.load(Ordering::SeqCst) {
                push(progress, ev("cancelled", "搜索被用户取消", 50, "", total_tokens));
                return Self::build_partial_result(paper_list, &query, llm_calls, search_calls, total_tokens, progress, false);
            }

            let round_pct = 8 + (llm_calls as u64 * 37 / max_llm_calls as u64).min(37) as u8;
            push(progress, ev("agent_think",
                &format!("🤔 Agent 思考中 (第 {} 轮)…", llm_calls + 1),
                round_pct,
                &serde_json::json!({
                    "round": llm_calls + 1,
                    "papers_so_far": paper_list.len(),
                    "llm_calls": llm_calls,
                    "search_calls": search_calls,
                }).to_string(),
                total_tokens));

            let resp = llm.chat_with_tools(&messages, &tools).await?;
            llm_calls += 1;
            total_tokens += resp.tokens;

            // ── Emit LLM thinking content ──
            let thinking = resp.content.clone().unwrap_or_default();
            if thinking.len() > 10 {
                let snippet: String = thinking.chars().take(400).collect();
                let truncated = if thinking.len() > 400 { format!("{}…", snippet) } else { snippet };
                push(progress, ev("agent_thought", &truncated, round_pct,
                    &serde_json::json!({"full_length": thinking.len(), "truncated": thinking.len() > 400}).to_string(),
                    total_tokens));
            } else if let Some(ref tcs) = resp.tool_calls {
                // Fallback: LLM didn't output text reasoning, but is calling tools.
                // Generate a synthetic thinking message from the tool calls.
                let descriptions: Vec<String> = tcs.iter()
                    .map(|tc| fmt_tool_args(&tc.function.name, &tc.function.arguments))
                    .collect();
                let synthetic = format!("Agent 决定: {}", descriptions.join("；"));
                push(progress, ev("agent_thought", &synthetic, round_pct,
                    &serde_json::json!({"synthetic": true}).to_string(),
                    total_tokens));
            }

            let assistant_msg = if let Some(ref tcs) = resp.tool_calls {
                LlmMessage::assistant_with_tools(tcs.clone())
            } else {
                LlmMessage::assistant(&thinking)
            };
            messages.push(assistant_msg);

            // Process tool calls
            if let Some(tool_calls) = resp.tool_calls {
                if tool_calls.is_empty() {
                    if thinking.len() < 30 {
                        messages.push(LlmMessage::user("Please use search_papers to find papers, then call finish_search when ready."));
                    } else {
                        messages.push(LlmMessage::user("Please call finish_search to proceed to the scoring phase."));
                    }
                    continue;
                }

                let mut should_finish = false;

                for tc in &tool_calls {
                    let fn_name = tc.function.name.as_str();
                    let args_desc = fmt_tool_args(fn_name, &tc.function.arguments);

                    if fn_name == "clarify_query" {
                        push(progress, ev("tool_call",
                            "🔍 查询过于宽泛，需要用户选择方向…",
                            round_pct + 5,
                            &serde_json::json!({"tool": "clarify_query"}).to_string(),
                            total_tokens));

                        if let Ok((msg, options)) = tools::parse_clarify(&tc.function.arguments) {
                            push(progress, ev("done",
                                &format!("请选择研究方向 ({} 个选项)", options.len()),
                                100, "", total_tokens));
                            return Ok(SearchResult {
                                conversation_id: String::new(),
                                summary: msg,
                                tiers: TieredResults { high_relevance: vec![], partial_relevance: vec![] },
                                total_candidates: 0, rounds_used: llm_calls,
                                needs_clarification: true,
                                clarification_options: options,
                            });
                        }
                        // Fall through — if parsing fails, continue loop
                        messages.push(LlmMessage::tool_result(tc.id.clone(),
                            "Error parsing clarify options. Please try again with valid JSON.".into()));
                        continue;
                    }

                    if fn_name == "finish_search" {
                        push(progress, ev("tool_call",
                            "📝 搜索阶段完成，准备评分…",
                            round_pct + 5,
                            &serde_json::json!({"tool": "finish_search"}).to_string(),
                            total_tokens));

                        finish_data = tools::parse_finish_message(&tc.function.arguments).ok();
                        should_finish = true;
                        break;
                    }

                    push(progress, ev("tool_start",
                        &format!("🔍 {}", args_desc),
                        round_pct,
                        &serde_json::json!({"tool": fn_name, "args_desc": args_desc}).to_string(),
                        total_tokens));

                    match tools::execute_tool(
                        fn_name, &tc.function.arguments,
                        search_backend, &mut paper_list, &mut seen_keys, &mut search_calls,
                    ).await {
                        Ok(result) => {
                            let added = count_papers_in_result(&result);
                            push(progress, ev("tool_done",
                                &format!("{} → 找到 {} 篇新论文 (共 {} 篇)",
                                    if added >= 10 { "📚 收获丰富" }
                                    else if added >= 5 { "📄 有所发现" }
                                    else if added > 0 { "📎 少量补充" }
                                    else { "😕 未找到新结果" },
                                    added, paper_list.len()),
                                round_pct + 2,
                                &serde_json::json!({"new_papers": added, "total_papers": paper_list.len()}).to_string(),
                                total_tokens));
                            messages.push(LlmMessage::tool_result(tc.id.clone(), result));
                        }
                        Err(e) => {
                            push(progress, ev("tool_error",
                                &format!("工具调用失败: {}", e), round_pct, "", total_tokens));
                            messages.push(LlmMessage::tool_result(tc.id.clone(),
                                format!("Error: {}. Please try a different approach.", e)));
                        }
                    }
                }

                if should_finish {
                    // Drop irrelevant papers first (if agent requested)
                    let mut pool_emptied = false;
                    if let Some((_, ref drops)) = finish_data {
                        if !drops.is_empty() {
                            let mut sorted: Vec<usize> = drops.clone();
                            sorted.sort_unstable_by(|a, b| b.cmp(a));
                            sorted.dedup();
                            let before = paper_list.len();
                            for idx in sorted {
                                if idx < paper_list.len() { paper_list.remove(idx); }
                            }
                            let dropped = before - paper_list.len();
                            if dropped > 0 {
                                push(progress, ev("agent_thought",
                                    &format!("丢弃了 {} 篇不相关论文，保留 {} 篇", dropped, paper_list.len()),
                                    48, "", total_tokens));
                            }
                            // If pool emptied but budget remains, push agent to search more
                            if paper_list.is_empty() && llm_calls < max_llm_calls && search_calls < max_search_calls {
                                pool_emptied = true;
                                push(progress, ev("agent_thought",
                                    "论文池已空，要求 Agent 重新搜索…", 49, "", total_tokens));
                                messages.push(LlmMessage::user(
                                    "All papers were dropped — none matched the refinement. Search for NEW papers using keywords that combine the original topic with the refinement constraints."
                                ));
                                finish_data = None;
                            }
                        }
                    }
                    if !pool_emptied {
                        if let Some((ref msg, _)) = finish_data {
                            if !msg.is_empty() {
                                push(progress, ev("agent_thought", msg, 48, "", total_tokens));
                            }
                        }
                        break;
                    }
                }

                // Force termination if budget fully exhausted
                if llm_calls >= max_llm_calls && search_calls >= max_search_calls {
                    push(progress, ev("force_finish", "预算耗尽，强制结束搜索阶段…", 48, "", total_tokens));
                    messages.push(LlmMessage::user("Call finish_search NOW."));
                    let resp = llm.chat_with_tools(&messages, &tools).await?;
                    llm_calls += 1;
                    total_tokens += resp.tokens;
                    if let Some(tcs) = resp.tool_calls {
                        if tcs.iter().any(|tc| tc.function.name == "finish_search") {
                            break;
                        }
                    }
                    break; // Break anyway — don't loop forever
                }
            } else {
                // No tool calls — prompt the LLM
                if thinking.len() < 30 {
                    messages.push(LlmMessage::user("Please use search_papers to find relevant papers, then call finish_search."));
                } else {
                    messages.push(LlmMessage::user("Please call finish_search to proceed to the scoring phase."));
                }
            }
        }

        // ═══════════════════════════════════════════════
        // PHASE 2: Batch Scoring
        // ═══════════════════════════════════════════════
        let paper_count = paper_list.len();
        if paper_count == 0 {
            // Pool still empty after retry — budget may be exhausted
            push(progress, ev("done",
                &format!("预算耗尽且未找到论文。请尝试放宽约束。"),
                100, &serde_json::json!({"high": 0, "partial": 0}).to_string(), total_tokens));
            return Ok(SearchResult {
                conversation_id: String::new(),
                summary: "多次搜索未找到匹配的论文。建议放宽约束或换个研究方向。".into(),
                tiers: TieredResults { high_relevance: vec![], partial_relevance: vec![] },
                total_candidates: 0, rounds_used: llm_calls,
                needs_clarification: false, clarification_options: vec![],
            });
        }

        let batch_count = (paper_count as f64 / 20.0).ceil() as usize;
        push(progress, ev("rank",
            &format!("⭐ AI 逐批评分为 {} 篇论文打分 ({} 批，每批 20 篇)…", paper_count, batch_count),
            55,
            &serde_json::json!({"total": paper_count, "batches": batch_count}).to_string(),
            total_tokens));

        // Score all batches concurrently
        let batch_size = 20;
        let mut all_scored = Vec::new();

        // Spawn all batch scoring tasks concurrently
        let mut handles = Vec::new();
        for (batch_idx, chunk) in paper_list.chunks(batch_size).enumerate() {
            let llm_clone = llm.clone();
            let chunk_owned: Vec<Paper> = chunk.to_vec();
            let query_owned = query.clone();
            push(progress, ev("rank_batch",
                &format!("启动评分第 {}/{} 批 ({} 篇)…", batch_idx + 1, batch_count, chunk.len()),
                58, "", total_tokens));

            handles.push(tokio::spawn(async move {
                (batch_idx, Self::score_batch(&llm_clone, &chunk_owned, &query_owned).await)
            }));
        }

        // Collect results as they complete
        let mut batch_results = Vec::new();
        for handle in handles {
            match handle.await {
                Ok((idx, Ok((scored_chunk, t)))) => {
                    batch_results.push((idx, scored_chunk, t));
                }
                Ok((_, Err(e))) => {
                    anyhow::bail!("Batch scoring failed: {}", e);
                }
                Err(e) => {
                    anyhow::bail!("Batch task panicked: {}", e);
                }
            }
        }

        // Sort by batch order and accumulate
        batch_results.sort_by_key(|(idx, _, _)| *idx);
        for (_, scored_chunk, t) in batch_results {
            total_tokens += t;
            all_scored.extend(scored_chunk);
        }

        // Partition into tiers
        let tiers = Ranker::partition(all_scored);
        let high_n = tiers.high_relevance.len();
        let partial_n = tiers.partial_relevance.len();

        push(progress, ev("rank_done",
            &format!("评分完成 — 高度相关 {} 篇 · 部分相关 {} 篇", high_n, partial_n),
            88,
            &serde_json::json!({"high": high_n, "partial": partial_n}).to_string(),
            total_tokens));

        // ═══════════════════════════════════════════════
        // PHASE 3: Generate Summary
        // ═══════════════════════════════════════════════
        push(progress, ev("organize", "📝 AI 生成搜索摘要…", 92, "", total_tokens));
        let (summary, t) = ResultOrganizer::generate_summary(llm, &tiers, &query).await?;
        total_tokens += t;

        let total = high_n + partial_n;
        push(progress, ev("done",
            &format!("✅ 完成 — {} 篇相关论文 · {} 轮搜索 · {} Token",
                total, llm_calls, total_tokens),
            100,
            &serde_json::json!({
                "high": high_n, "partial": partial_n,
                "llm_calls": llm_calls, "search_calls": search_calls, "tokens": total_tokens,
            }).to_string(),
            total_tokens));

        Ok(ResultOrganizer::assemble_result(summary, tiers, total, llm_calls))
    }

    /// Score a single batch of papers for relevance to the query.
    /// Uses the legacy JSON-mode chat (not tool calling) for reliable structured output.
    /// Build partial results when cancelled — rough ranking by citation count
    fn build_partial_result(
        paper_list: Vec<Paper>,
        _query: &str,
        llm_calls: u32,
        _search_calls: u32,
        total_tokens: u32,
        progress: &Arc<Mutex<Vec<ProgressEvent>>>,
        _completed: bool,
    ) -> Result<SearchResult, anyhow::Error> {
        if paper_list.is_empty() {
            anyhow::bail!("搜索被取消，未找到任何论文。");
        }
        let mut sorted = paper_list;
        sorted.sort_by(|a, b| b.citation_count.cmp(&a.citation_count));
        let tops: Vec<ScoredPaper> = sorted.iter().take(30).map(|p| {
            ScoredPaper {
                paper: p.clone(),
                score: if p.citation_count > 100 { 7 } else if p.citation_count > 10 { 5 } else { 3 },
                rationale: "(搜索被取消，按引用数粗略排序)".into(),
            }
        }).collect();
        let mut high = Vec::new();
        let mut partial = Vec::new();
        for sp in tops {
            if sp.score >= 7 { high.push(sp); } else { partial.push(sp); }
        }
        let tiers = TieredResults { high_relevance: high, partial_relevance: partial };
        let total = tiers.high_relevance.len() + tiers.partial_relevance.len();
        push(progress, ev("done",
            &format!("⚠️ 搜索被取消 — {} 篇论文 · {} Token", total, total_tokens),
            100, &serde_json::json!({"high": tiers.high_relevance.len(), "partial": tiers.partial_relevance.len(), "cancelled": true}).to_string(), total_tokens));
        Ok(SearchResult {
            conversation_id: String::new(),
            summary: format!("搜索被取消。基于已收集的 {} 篇论文按引用数排序。", total),
            tiers, total_candidates: total, rounds_used: llm_calls,
            needs_clarification: false, clarification_options: vec![],
        })
    }

    async fn score_batch(
        llm: &LlmBackend,
        papers: &[Paper],
        original_query: &str,
    ) -> Result<(Vec<crate::types::ScoredPaper>, u32), anyhow::Error> {
        let papers_text: Vec<String> = papers
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let abstract_snippet: String = p.abstract_text.chars().take(300).collect();
                format!(
                    "[{}] Title: {}\nAuthors: {}\nYear: {} | Venue: {} | Citations: {}\nAbstract: {}",
                    i,
                    p.title,
                    p.authors.first().map(|s| s.as_str()).unwrap_or("Unknown"),
                    p.year,
                    p.venue,
                    p.citation_count,
                    abstract_snippet
                )
            })
            .collect();

        let system = r#"你是一个学术论文评审专家。请对每篇论文与用户查询的相关性打分（1-10分）。

评分标准：
- 9-10: 直接相关，核心论文
- 7-8: 高度相关，重要参考
- 5-6: 部分相关
- 3-4: 略有关系
- 1-2: 基本无关

请输出严格 JSON 格式：{"scores": [{"index": 0, "score": 8, "rationale": "中文理由"}, ...]}"#;

        let user_prompt = format!(
            "用户查询: {}\n\n论文列表:\n{}",
            original_query,
            papers_text.join("\n\n")
        );

        let resp = llm.chat(system, &user_prompt).await?;
        let tokens = resp.tokens;
        let content = resp.content.unwrap_or_default();
        let json: serde_json::Value = serde_json::from_str(&content)?;

        let mut scored = Vec::new();
        if let Some(scores) = json["scores"].as_array() {
            for entry in scores {
                let idx = entry["index"].as_u64().unwrap_or(0) as usize;
                if idx < papers.len() {
                    scored.push(crate::types::ScoredPaper {
                        paper: papers[idx].clone(),
                        score: entry["score"].as_u64().unwrap_or(5).min(10).max(1) as u8,
                        rationale: entry["rationale"].as_str().unwrap_or("").to_string(),
                    });
                }
            }
        }

        Ok((scored, tokens))
    }
}
