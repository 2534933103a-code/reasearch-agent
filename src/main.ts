import { invoke } from "@tauri-apps/api/core";

declare const d3: any;

// ── Types ──────────────────────────────────────────
interface Paper { id: string; title: string; authors: string[]; year: number; venue: string; doi: string; abstract_text: string; citation_count: number; url: string; }
interface ScoredPaper { paper: Paper; score: number; rationale: string; }
interface SearchResult { conversation_id: string; summary: string; tiers: { high_relevance: ScoredPaper[]; partial_relevance: ScoredPaper[] }; total_candidates: number; rounds_used: number; needs_clarification?: boolean; clarification_options?: string[]; }
interface ProgressEvent { phase: string; message: string; percent: number; detail: string; tokens: number; }
interface LlmProfile { id: string; name: string; provider: string; model: string; api_key: string; base_url: string; }
interface LlmConfig { profiles: LlmProfile[]; active_profile_id: string; }
interface AppConfig { llm: LlmConfig; search: any; budget: any; }
interface Conversation { id: string; title: string; messages: { role: string; content: string; timestamp: number }[]; search_results: SearchResult[]; created_at: number; }

// ── State ──────────────────────────────────────────
let currentResult: SearchResult | null = null;
let currentGraphData: any = null;
let currentView: "list" | "graph" = "list";
let activeConvId: string | null = null;
// Rich step: phase type + data for rendering
interface ActivityStep {
  phase: string;        // event phase
  message: string;      // display text
  detail: any;          // parsed JSON detail
  tokens: number;       // cumulative tokens
  ts: number;           // timestamp for animation ordering
}
let activitySteps: ActivityStep[] = [];
let appConfig: AppConfig | null = null;

// ── Activity Feed ──────────────────────────────────
function initActivityFeed() {
  activitySteps = [];
  renderActivity();
  document.getElementById("activity-feed")!.classList.remove("hidden");
}

function addStep(phase: string, msg: string, detail: string) {
  let detailObj: any = null;
  try { detailObj = JSON.parse(detail); } catch (_) {}
  const tkn = detailObj?.tokens || 0;
  activitySteps.push({ phase, message: msg, detail: detailObj, tokens: tkn, ts: Date.now() });
  // Keep max 30 steps so feed doesn't grow forever
  if (activitySteps.length > 30) activitySteps = activitySteps.slice(-25);
  renderActivity();
}

function renderActivity() {
  const c = document.getElementById("activity-feed")!;
  let h = `<div class="flex items-center justify-between mb-3">
    <h3 class="text-xs font-semibold text-slate-400 uppercase tracking-wider">搜索进度</h3>
    <span class="text-[10px] text-slate-300">${activitySteps.length} 条记录</span>
  </div>`;

  activitySteps.forEach((s, i) => {
    const isLast = i === activitySteps.length - 1;
    h += renderStep(s, isLast);
  });

  c.innerHTML = h;
  // Auto-scroll to bottom
  requestAnimationFrame(() => { c.scrollTop = c.scrollHeight; });
}

function renderStep(s: ActivityStep, isLast: boolean): string {
  const p = s.phase;

  // ── Agent thinking bubble ──
  if (p === "agent_thought") {
    const fullLen = s.detail?.full_length || s.message.length;
    const truncated = s.detail?.truncated;
    return `
    <div class="flex gap-3 ${isLast ? "pb-2" : "pb-3"} animate-fade-in">
      <div class="flex flex-col items-center flex-shrink-0">
        <span class="text-xs bg-purple-100 text-purple-600 rounded-full w-5 h-5 flex items-center justify-center font-mono">💭</span>
        ${isLast ? "" : '<div class="step-line flex-1 bg-purple-100"></div>'}
      </div>
      <div class="flex-1 min-w-0">
        <div class="bg-purple-50 border border-purple-100 rounded-xl p-3">
          <p class="text-xs text-purple-700 leading-relaxed whitespace-pre-wrap line-clamp-4">${esc(s.message)}</p>
          ${truncated ? `<span class="text-[10px] text-purple-400 mt-1 inline-block">… 原文 ${fullLen} 字符，已截断</span>` : ""}
        </div>
      </div>
    </div>`;
  }

  // ── Tool call start ──
  if (p === "tool_start") {
    const toolName = s.detail?.tool || "?";
    const toolIcon = toolName === "search_papers" ? "🔍" : toolName === "get_cited_papers" ? "📎" : toolName === "get_references" ? "📖" : "🔧";
    return `
    <div class="flex gap-3 ${isLast ? "pb-2" : "pb-3"} animate-fade-in">
      <div class="flex flex-col items-center flex-shrink-0">
        <span class="text-xs bg-blue-100 text-blue-600 rounded-full w-5 h-5 flex items-center justify-center">${toolIcon}</span>
        ${isLast ? "" : '<div class="step-line flex-1 bg-blue-100"></div>'}
      </div>
      <div class="flex-1 min-w-0">
        <div class="bg-blue-50/60 border border-blue-100 rounded-lg px-3 py-2">
          <div class="flex items-center gap-2">
            <span class="text-[10px] font-semibold text-blue-500 uppercase tracking-wider">${esc(toolName)}</span>
            <span class="text-xs text-slate-600 truncate">${esc(s.message.replace(/^🔍\s*/, ""))}</span>
          </div>
        </div>
      </div>
    </div>`;
  }

  // ── Tool call done ──
  if (p === "tool_done") {
    const added = s.detail?.new_papers ?? 0;
    const total = s.detail?.total_papers ?? 0;
    const clr = added > 0 ? (added >= 10 ? "text-emerald-600 bg-emerald-50 border-emerald-100" : added >= 5 ? "text-emerald-600 bg-emerald-50 border-emerald-100" : "text-amber-600 bg-amber-50 border-amber-100") : "text-slate-400 bg-slate-50 border-slate-100";
    return `
    <div class="flex gap-3 ${isLast ? "pb-2" : "pb-3"} animate-fade-in">
      <div class="flex flex-col items-center flex-shrink-0">
        <span class="text-[10px] ${added > 0 ? "text-emerald-500" : "text-slate-300"}">${added > 0 ? "✓" : "—"}</span>
        ${isLast ? "" : '<div class="step-line flex-1 bg-slate-100"></div>'}
      </div>
      <div class="flex-1 min-w-0">
        <div class="${clr} border rounded-lg px-3 py-1.5">
          <p class="text-xs">${esc(s.message)}</p>
        </div>
      </div>
    </div>`;
  }

  // ── Scoring phase start ──
  if (p === "rank") {
    const total = s.detail?.total ?? 0;
    const batches = s.detail?.batches ?? 0;
    return `
    <div class="flex gap-3 ${isLast ? "pb-2" : "pb-3"} animate-fade-in">
      <div class="flex flex-col items-center flex-shrink-0">
        <span class="text-xs bg-amber-100 text-amber-600 rounded-full w-5 h-5 flex items-center justify-center">⭐</span>
        ${isLast ? "" : '<div class="step-line flex-1 bg-amber-100"></div>'}
      </div>
      <div class="flex-1 min-w-0">
        <div class="bg-amber-50 border border-amber-200 rounded-lg px-3 py-2">
          <p class="text-xs text-amber-800 font-medium">${esc(s.message)}</p>
        </div>
      </div>
    </div>`;
  }

  // ── Scoring batch progress ──
  if (p === "rank_batch") {
    return `
    <div class="flex gap-3 ${isLast ? "pb-2" : "pb-3"} animate-fade-in">
      <div class="flex flex-col items-center flex-shrink-0">
        <span class="text-[10px] animate-pulse">📊</span>
        ${isLast ? "" : '<div class="step-line flex-1 bg-slate-100"></div>'}
      </div>
      <div class="flex-1 min-w-0">
        <p class="text-xs text-slate-500">${esc(s.message)}</p>
      </div>
    </div>`;
  }

  // ── Scoring done ──
  if (p === "rank_done") {
    const high = s.detail?.high ?? 0;
    const partial = s.detail?.partial ?? 0;
    return `
    <div class="flex gap-3 ${isLast ? "pb-2" : "pb-3"} animate-fade-in">
      <div class="flex flex-col items-center flex-shrink-0">
        <span class="text-xs bg-emerald-100 text-emerald-600 rounded-full w-5 h-5 flex items-center justify-center">✓</span>
        ${isLast ? "" : '<div class="step-line flex-1 bg-emerald-100"></div>'}
      </div>
      <div class="flex-1 min-w-0">
        <div class="bg-emerald-50 border border-emerald-200 rounded-lg px-3 py-2">
          <p class="text-xs text-emerald-700 font-medium">${esc(s.message)}</p>
          <div class="flex gap-2 mt-1">
            <span class="text-[10px] text-emerald-600">⭐ ${high} 篇高度相关</span>
            <span class="text-[10px] text-amber-600">📄 ${partial} 篇部分相关</span>
          </div>
        </div>
      </div>
    </div>`;
  }

  // ── Summary generation ──
  if (p === "organize") {
    return `
    <div class="flex gap-3 ${isLast ? "pb-2" : "pb-3"} animate-fade-in">
      <div class="flex flex-col items-center flex-shrink-0">
        <span class="text-xs bg-indigo-100 text-indigo-600 rounded-full w-5 h-5 flex items-center justify-center">📝</span>
        ${isLast ? "" : '<div class="step-line flex-1 bg-indigo-100"></div>'}
      </div>
      <div class="flex-1 min-w-0">
        <p class="text-xs text-indigo-600 font-medium">${esc(s.message)}</p>
      </div>
    </div>`;
  }

  // ── Tool call (generic) ──
  if (p === "tool_call") {
    return `
    <div class="flex gap-3 ${isLast ? "pb-2" : "pb-3"} animate-fade-in">
      <div class="flex flex-col items-center flex-shrink-0">
        <span class="text-xs bg-indigo-100 text-indigo-600 rounded-full w-5 h-5 flex items-center justify-center">📝</span>
        ${isLast ? "" : '<div class="step-line flex-1 bg-indigo-100"></div>'}
      </div>
      <div class="flex-1 min-w-0">
        <p class="text-xs text-slate-600">${esc(s.message)}</p>
      </div>
    </div>`;
  }

  // ── Agent thinking start ──
  if (p === "agent_think") {
    const round = s.detail?.round || "?";
    const papers = s.detail?.papers_so_far ?? 0;
    return `
    <div class="flex gap-3 ${isLast ? "pb-2" : "pb-3"} animate-fade-in">
      <div class="flex flex-col items-center flex-shrink-0">
        <span class="text-[10px] animate-pulse">🤔</span>
        ${isLast ? "" : '<div class="step-line flex-1 bg-slate-100"></div>'}
      </div>
      <div class="flex-1 min-w-0">
        <p class="text-xs text-slate-500 font-medium">${esc(s.message)}${papers > 0 ? ` <span class="text-slate-400 font-normal">· 已收集 ${papers} 篇</span>` : ""}</p>
      </div>
    </div>`;
  }

  // ── Agent start ──
  if (p === "agent_start") {
    return `
    <div class="flex gap-3 ${isLast ? "pb-2" : "pb-3"} animate-fade-in">
      <div class="flex flex-col items-center flex-shrink-0">
        <span class="text-xs bg-indigo-500 text-white rounded-full w-5 h-5 flex items-center justify-center font-bold">🚀</span>
        ${isLast ? "" : '<div class="step-line flex-1 bg-indigo-200"></div>'}
      </div>
      <div class="flex-1 min-w-0">
        <p class="text-sm text-slate-800 font-semibold">开始检索</p>
        <p class="text-xs text-slate-500 mt-0.5">${esc(s.message)}</p>
      </div>
    </div>`;
  }

  // ── Done ──
  if (p === "done") {
    return `
    <div class="flex gap-3 pb-2 animate-fade-in">
      <div class="flex flex-col items-center flex-shrink-0">
        <span class="text-sm">🎉</span>
      </div>
      <div class="flex-1 min-w-0">
        <div class="bg-gradient-to-r from-emerald-50 to-teal-50 border border-emerald-200 rounded-xl p-3">
          <p class="text-sm text-emerald-800 font-semibold">检索完成</p>
          <p class="text-xs text-emerald-600 mt-1">${esc(s.message)}</p>
          <div class="flex gap-2 mt-2">
            ${s.detail?.high ? `<span class="text-[10px] bg-white/70 text-emerald-700 rounded-full px-2 py-0.5">⭐ ${s.detail.high} 篇高度相关</span>` : ""}
            ${s.detail?.partial ? `<span class="text-[10px] bg-white/70 text-amber-600 rounded-full px-2 py-0.5">📄 ${s.detail.partial} 篇部分相关</span>` : ""}
            ${s.detail?.tokens ? `<span class="text-[10px] bg-white/70 text-slate-500 rounded-full px-2 py-0.5">🪙 ${s.detail.tokens.toLocaleString()} Token</span>` : ""}
            ${s.detail?.llm_calls ? `<span class="text-[10px] bg-white/70 text-slate-500 rounded-full px-2 py-0.5">🧠 ${s.detail.llm_calls} 次调用</span>` : ""}
          </div>
        </div>
      </div>
    </div>`;
  }

  // ── Error ──
  if (p === "error" || p === "tool_error") {
    return `
    <div class="flex gap-3 ${isLast ? "pb-2" : "pb-3"} animate-fade-in">
      <div class="flex flex-col items-center flex-shrink-0">
        <span class="text-xs bg-red-100 text-red-600 rounded-full w-5 h-5 flex items-center justify-center">❌</span>
        ${isLast ? "" : '<div class="step-line flex-1 bg-red-100"></div>'}
      </div>
      <div class="flex-1 min-w-0">
        <div class="bg-red-50 border border-red-100 rounded-lg px-3 py-2">
          <p class="text-xs text-red-600">${esc(s.message)}</p>
        </div>
      </div>
    </div>`;
  }

  // ── Cancelled ──
  if (p === "cancelled") {
    return `
    <div class="flex gap-3 pb-2 animate-fade-in">
      <div class="flex flex-col items-center flex-shrink-0">
        <span class="text-xs bg-amber-100 text-amber-600 rounded-full w-5 h-5 flex items-center justify-center">⏹</span>
      </div>
      <div class="flex-1 min-w-0">
        <div class="bg-amber-50 border border-amber-200 rounded-lg px-3 py-2">
          <p class="text-xs text-amber-700 font-medium">${esc(s.message)}</p>
        </div>
      </div>
    </div>`;
  }

  // ── Default / legacy ──
  const icon = p === "organize" ? "📝" : p === "budget_warn" ? "⚠️" : p === "force_finish" ? "⚡" : "·";
  return `
  <div class="flex gap-3 ${isLast ? "pb-2" : "pb-3"} animate-fade-in">
    <div class="flex flex-col items-center flex-shrink-0">
      <span class="text-[10px] text-slate-400">${icon}</span>
      ${isLast ? "" : '<div class="step-line flex-1 bg-slate-200"></div>'}
    </div>
    <div class="flex-1 min-w-0">
      <p class="text-xs text-slate-500">${esc(s.message)}</p>
    </div>
  </div>`;
}

// ── Poll progress ──────────────────────────────────
async function pollProgress(pollTimer: number, lastCount: number): Promise<number> {
  try {
    const steps = await invoke<ProgressEvent[]>("get_progress");
    for (let i = lastCount; i < steps.length; i++) {
      const p = steps[i];
      addStep(p.phase, p.message, p.detail);
    }
    return steps.length;
  } catch (_) { return lastCount; }
}

// ── Stop ──────────────────────────────────────────
let searchActive = false;

async function doStop() {
  try { await invoke("cancel_search"); } catch (_) {}
  // Immediately reset UI — the finally block in doSearch/doRefine will also run
  setSearching(false);
}

function setSearching(active: boolean) {
  searchActive = active;
  const searchBtn = document.getElementById("search-btn") as HTMLButtonElement;
  const stopBtn = document.getElementById("stop-btn") as HTMLButtonElement;
  if (active) {
    searchBtn.disabled = true; searchBtn.textContent = "搜索中...";
    searchBtn.classList.add("opacity-50");
    stopBtn.classList.remove("hidden");
  } else {
    searchBtn.disabled = false; searchBtn.textContent = "搜索";
    searchBtn.classList.remove("opacity-50");
    stopBtn.classList.add("hidden");
  }
}

// ── Clarification ──────────────────────────────────
function renderClarification(r: SearchResult) {
  const c = document.getElementById("results-container")!;
  let h = `<div class="bg-gradient-to-r from-amber-50 to-orange-50 rounded-2xl border border-amber-100 p-5 mb-6">
    <p class="text-sm text-amber-800 font-medium mb-1">🔍 查询方向不明确</p>
    <p class="text-sm text-slate-700">${esc(r.summary)}</p>
  </div>
  <div class="mb-6">
    <h3 class="text-xs font-semibold text-slate-400 uppercase tracking-wider mb-3">请选择一个具体方向</h3>
    <div class="grid gap-2">`;
  (r.clarification_options || []).forEach((opt, i) => {
    h += `<button onclick="clarifySearch('${esc(opt)}')"
      class="text-left w-full px-4 py-3 bg-white border border-slate-200 rounded-xl hover:border-indigo-300 hover:bg-indigo-50/30 transition group">
      <span class="text-[10px] font-bold text-indigo-400 bg-indigo-50 rounded-full w-5 h-5 inline-flex items-center justify-center mr-2">${i+1}</span>
      <span class="text-sm text-slate-700 group-hover:text-indigo-700">${esc(opt)}</span>
    </button>`;
  });
  h += `</div></div>`;
  c.innerHTML = h;
  document.getElementById("refine-box")!.classList.remove("hidden");
  (document.getElementById("refine-input") as HTMLInputElement).placeholder = "或输入你自己的细化方向…";
}

(window as any).clarifySearch = function(option: string) {
  // Keep original query context — refine the existing conversation with the selected option
  (document.getElementById("refine-input") as HTMLInputElement).value = option;
  doRefine();
};
async function showCurrentConversation() {
  if (!activeConvId) return;
  try {
    const cs = await invoke<Conversation[]>("get_conversations");
    const conv = cs.find(c => c.id === activeConvId);
    if (conv && conv.search_results.length > 0) {
      currentResult = conv.search_results[conv.search_results.length - 1];
      currentGraphData = buildGraphData(currentResult);
      renderConversation(conv);
      document.getElementById("refine-box")!.classList.remove("hidden");
      document.getElementById("view-toggle")!.classList.remove("hidden");
    }
  } catch (_) {}
}

// ── Search ─────────────────────────────────────────
async function doSearch() {
  const q = (document.getElementById("query-input") as HTMLInputElement).value.trim();
  if (!q) return;
  resetUI();
  initActivityFeed();
  setSearching(true);

  let last = 0;
  const timer = setInterval(() => { pollProgress(timer, last).then(n => last = n); }, 300);

  try {
    currentResult = await invoke<SearchResult>("search", { query: q, conversationId: null });
    clearInterval(timer);
    await pollProgress(timer, last);
    activeConvId = currentResult.conversation_id;
    refreshConversations();
    if (currentResult.needs_clarification) {
      renderClarification(currentResult);
    } else {
      await showCurrentConversation();
    }
  } catch (err) {
    clearInterval(timer);
    const errStr = String(err);
    if (!errStr.includes("搜索被取消")) {
      addStep("error", `搜索失败: ${errStr}`, "");
      document.getElementById("results-container")!.innerHTML = `<div class="bg-red-50 border border-red-200 rounded-2xl p-5 text-red-600 text-sm">${esc(errStr)}</div>`;
    } else {
      addStep("cancelled", "搜索被取消", "");
    }
  } finally {
    setSearching(false);
  }
}

// ── Refine ─────────────────────────────────────────
async function doRefine() {
  const refinement = getRefinementText();
  if (!refinement || !activeConvId) return;
  (document.getElementById("refine-input") as HTMLInputElement).value = "";
  (document.getElementById("year-from") as HTMLInputElement).value = "";
  (document.getElementById("year-to") as HTMLInputElement).value = "";
  resetUI();
  initActivityFeed();
  setSearching(true);

  let last = 0;
  const timer = setInterval(() => { pollProgress(timer, last).then(n => last = n); }, 300);

  try {
    currentResult = await invoke<SearchResult>("refine_search", { conversationId: activeConvId, refinement });
    clearInterval(timer);
    await pollProgress(timer, last);
    refreshConversations();
    await showCurrentConversation();
  } catch (err) {
    clearInterval(timer);
    const errStr = String(err);
    if (!errStr.includes("取消")) {
      addStep("error", `细化失败: ${errStr}`, "");
    } else {
      addStep("cancelled", "细化被取消", "");
    }
  } finally {
    setSearching(false);
  }
}

// ── UI Helpers ─────────────────────────────────────
function resetUI() {
  document.getElementById("results-container")!.innerHTML = "";
  document.getElementById("view-toggle")!.classList.add("hidden");
  document.getElementById("graph-container")!.classList.add("hidden");
  document.getElementById("refine-box")!.classList.add("hidden");
}

function esc(t: string): string { const d = document.createElement("div"); d.textContent = t; return d.innerHTML; }

// ── Render Results ─────────────────────────────────
function renderResults(r: SearchResult) {
  const c = document.getElementById("results-container")!;
  const allPapers = [...r.tiers.high_relevance, ...r.tiers.partial_relevance]
    .sort((a, b) => b.score - a.score);
  const total = allPapers.length;
  const truncated = total > 50;

  let h = `<div class="bg-gradient-to-r from-indigo-50 to-purple-50 rounded-2xl border border-indigo-100 p-5 mb-6">
    <p class="text-sm text-slate-700 mb-2">${r.summary}</p>
    <div class="flex gap-3 flex-wrap">
      <span class="text-xs text-slate-500 bg-white/80 rounded-lg px-2.5 py-1">候选 ${r.total_candidates} 篇</span>
      <span class="text-xs text-slate-500 bg-white/80 rounded-lg px-2.5 py-1">${r.rounds_used} 轮搜索</span>
      <span class="text-xs text-slate-500 bg-white/80 rounded-lg px-2.5 py-1">⭐ ${r.tiers.high_relevance.length} 高度相关</span>
      ${truncated ? `<span class="text-xs text-amber-600 bg-amber-50 rounded-lg px-2.5 py-1">显示前 50 / 共 ${total} 篇</span>` : ""}
    </div>
  </div>
  <div class="mb-6">
    <div class="flex items-center gap-2 mb-3">
      <div class="w-2 h-2 rounded-full bg-indigo-500"></div>
      <h2 class="text-base font-semibold text-slate-800">搜索结果</h2>
      <span class="text-xs text-slate-500 bg-slate-100 rounded-full px-2 py-0.5">${total} 篇</span>
      <span class="text-[10px] text-slate-400 ml-auto">按相关度降序</span>
    </div>`;
  h += renderResultsFlat(r);
  h += `</div>`;
  c.innerHTML = h;
  bindAbstractToggles(c);
  showExportBar();
}

// ── Graph ──────────────────────────────────────────
function buildGraphData(r: SearchResult): any {
  const all = [...r.tiers.high_relevance, ...r.tiers.partial_relevance];
  const nodes = all.map((sp, i) => ({ index: i, title: sp.paper.title, cluster: sp.score >= 7 ? 0 : 1 }));
  const edges: any[] = [];
  for (let i = 0; i < nodes.length && i < 50; i++) for (let j = i + 1; j < nodes.length && j < 50; j++) {
    const wi = nodes[i].title.toLowerCase().split(/\s+/), wj = nodes[j].title.toLowerCase().split(/\s+/);
    if (wi.filter((w: string) => w.length > 3 && wj.includes(w)).length >= 3) edges.push({ source: i, target: j, relation: "topic" });
  }
  return { nodes, edges };
}

function switchView(v: "list" | "graph") {
  currentView = v;
  const bl = document.getElementById("btn-list")!, bg = document.getElementById("btn-graph")!;
  bl.className = v === "list" ? "px-4 py-1.5 text-sm rounded-lg bg-primary text-white transition" : "px-4 py-1.5 text-sm rounded-lg bg-slate-200 text-slate-600 hover:bg-slate-300 transition";
  bg.className = v === "graph" ? "px-4 py-1.5 text-sm rounded-lg bg-primary text-white transition" : "px-4 py-1.5 text-sm rounded-lg bg-slate-200 text-slate-600 hover:bg-slate-300 transition";
  document.getElementById("graph-container")!.classList.toggle("hidden", v !== "graph");
  document.getElementById("results-container")!.classList.toggle("hidden", v === "graph");
  if (v === "graph" && currentGraphData) renderGraph(currentGraphData);
}

function renderGraph(data: any) {
  const c = document.getElementById("graph-container")!; c.innerHTML = "";
  const w = Math.max(c.clientWidth, 400), h = 520;
  const svg = d3.select("#graph-container").append("svg").attr("width", w).attr("height", h);
  const sim = d3.forceSimulation(data.nodes).force("link", d3.forceLink(data.edges).id((d: any) => d.index).distance(100)).force("charge", d3.forceManyBody().strength(-300)).force("center", d3.forceCenter(w / 2, h / 2));
  const link = svg.append("g").selectAll("line").data(data.edges).join("line").attr("stroke", "#e2e8f0").attr("stroke-width", 1.5);
  const node = svg.append("g").selectAll("circle").data(data.nodes).join("circle")
    .attr("r", 9)
    .attr("fill", (d: any) => d.cluster === 0 ? "#4F46E5" : "#F59E0B")
    .attr("stroke", "#fff")
    .attr("stroke-width", 2)
    .call(d3.drag()
      .on("start", (e: any, d: any) => { if (!e.active) sim.alphaTarget(0.3).restart(); d.fx = d.x; d.fy = d.y; })
      .on("drag", (e: any, d: any) => { d.fx = e.x; d.fy = e.y; })
      .on("end", (e: any, d: any) => { if (!e.active) sim.alphaTarget(0); d.fx = null; d.fy = null; }));
  // Append titles SEPARATELY so `node` still refers to <circle> elements
  node.append("title").text((d: any) => d.title);
  sim.on("tick", () => {
    link.attr("x1", (d: any) => d.source.x).attr("y1", (d: any) => d.source.y)
        .attr("x2", (d: any) => d.target.x).attr("y2", (d: any) => d.target.y);
    node.attr("cx", (d: any) => d.x).attr("cy", (d: any) => d.y);
  });
}

// ── Conversations ──────────────────────────────────
function toggleSidebar() {
  document.getElementById("sidebar")!.classList.toggle("hidden");
  refreshConversations();
}

async function refreshConversations() {
  try {
    const cs = await invoke<Conversation[]>("get_conversations");
    const list = document.getElementById("conv-list")!;
    list.innerHTML = cs.map(c => {
      const rounds = c.search_results?.length || 0;
      return `<div class="flex items-center gap-2 p-2 rounded-lg hover:bg-slate-50 cursor-pointer ${c.id === activeConvId ? 'bg-indigo-50 border border-indigo-100' : ''}" onclick="loadConversation('${c.id}')">
        <div class="flex-1 min-w-0">
          <div class="text-xs truncate">${esc(c.title)}</div>
          ${rounds > 1 ? `<div class="text-[10px] text-slate-400">${rounds} 轮对话</div>` : ""}
        </div>
        <button class="text-slate-300 hover:text-red-500 text-xs flex-shrink-0" onclick="event.stopPropagation();deleteConv('${c.id}')">✕</button>
      </div>`;
    }).join("");
  } catch (_) {}
}

async function loadConversation(id: string) {
  try {
    const cs = await invoke<Conversation[]>("get_conversations");
    const conv = cs.find(c => c.id === id);
    if (!conv || !conv.search_results.length) return;
    activeConvId = id;
    // Show the latest search result
    currentResult = conv.search_results[conv.search_results.length - 1];
    currentGraphData = buildGraphData(currentResult);
    renderConversation(conv);
    document.getElementById("refine-box")!.classList.remove("hidden");
    document.getElementById("view-toggle")!.classList.remove("hidden");
    document.getElementById("activity-feed")!.classList.add("hidden");
    (document.getElementById("query-input") as HTMLInputElement).value = conv.messages[0]?.content || "";
    refreshConversations();
  } catch (_) {}
}

function renderConversation(conv: Conversation) {
  const c = document.getElementById("results-container")!;
  const totalRounds = conv.search_results.length;
  const latest = conv.search_results[totalRounds - 1];

  let h = "";

  // Show previous rounds as compact cards (if more than 1)
  if (totalRounds > 1) {
    h += `<div class="mb-6">
      <h3 class="text-xs font-semibold text-slate-400 uppercase tracking-wider mb-2">对话历史 (${totalRounds} 轮)</h3>`;
    for (let r = 0; r < totalRounds - 1; r++) {
      const sr = conv.search_results[r];
      const msg = conv.messages[r * 2]; // user message for this round
      const queryText = msg?.content || `第 ${r + 1} 轮`;
      h += `<div class="bg-slate-50 border border-slate-200 rounded-xl p-3 mb-2 cursor-pointer hover:border-indigo-300 transition"
        onclick="event.stopPropagation();showRound('${conv.id}', ${r})">
        <div class="flex items-center gap-2">
          <span class="text-[10px] bg-slate-300 text-white rounded-full w-4 h-4 flex items-center justify-center font-bold">${r + 1}</span>
          <span class="text-xs text-slate-600 truncate">${esc(queryText.slice(0, 60))}</span>
          <span class="text-[10px] text-slate-400 ml-auto">⭐ ${sr.tiers.high_relevance.length} · 📄 ${sr.tiers.partial_relevance.length}</span>
        </div>
      </div>`;
    }
    h += `</div>`;
  }

  // Latest round (current result) — detailed display
  h += `<div class="mb-4">
    <div class="flex items-center gap-2 mb-3">
      <span class="text-[10px] bg-indigo-500 text-white rounded-full w-4 h-4 flex items-center justify-center font-bold">${totalRounds}</span>
      <span class="text-xs font-semibold text-indigo-600 uppercase">当前结果</span>
    </div>`;
  h += renderResultsFlat(latest);
  h += `</div>`;

  c.innerHTML = h;
  bindAbstractToggles(c);
  showExportBar();
}

// Show a specific round from history
(window as any).showRound = function(convId: string, roundIdx: number) {
  invoke<Conversation[]>("get_conversations").then(cs => {
    const conv = cs.find(c => c.id === convId);
    if (conv && conv.search_results[roundIdx]) {
      currentResult = conv.search_results[roundIdx];
      currentGraphData = buildGraphData(currentResult);
      const c = document.getElementById("results-container")!;
      let h = `<div class="mb-3">
        <button onclick="loadConversation('${convId}')" class="text-xs text-indigo-500 hover:text-indigo-700 font-medium">← 返回完整对话</button>
        <span class="text-xs text-slate-400 ml-2">第 ${roundIdx + 1}/${conv.search_results.length} 轮</span>
      </div>`;
      h += renderResultsFlat(currentResult);
      c.innerHTML = h;
      bindAbstractToggles(c);
    }
  });
};

// Flat result rendering (without conversation header)
function renderResultsFlat(r: SearchResult): string {
  const allPapers = [...r.tiers.high_relevance, ...r.tiers.partial_relevance]
    .sort((a, b) => b.score - a.score);
  const total = allPapers.length;
  const shown = allPapers.slice(0, 50);

  let h = `<div class="flex items-center gap-2 mb-2">
    <span class="text-xs text-slate-500">${total} 篇</span>
    <span class="text-[10px] text-slate-400">按相关度降序</span>
  </div>`;

  shown.forEach((sp, i) => {
    const p = sp.paper;
    const scoreClr = sp.score >= 9 ? { b: "text-green-700", bg: "bg-green-50", d: "bg-green-500" }
      : sp.score >= 7 ? { b: "text-emerald-700", bg: "bg-emerald-50", d: "bg-emerald-500" }
      : sp.score >= 5 ? { b: "text-amber-700", bg: "bg-amber-50", d: "bg-amber-500" }
      : { b: "text-slate-500", bg: "bg-slate-100", d: "bg-slate-400" };
    const uid = `p-round-${i}`;
    h += `<div class="paper-card bg-white border border-slate-200 rounded-xl p-4 mb-2.5 transition">
      <div class="flex items-start justify-between gap-4"><div class="flex-1 min-w-0">
        <div class="flex items-center gap-2 mb-1">
          <span class="text-xs text-slate-400 font-mono flex-shrink-0">#${i+1}</span>
          <span class="${scoreClr.b} text-[11px] font-semibold px-1.5 py-0.5 rounded ${scoreClr.bg}">${sp.score}/10</span>
          <h3 class="text-sm font-semibold text-slate-800 truncate">${esc(p.title)}</h3>
        </div>
        <p class="text-xs text-slate-500 ml-7">${p.authors.slice(0,3).join(", ")}${p.authors.length>3?" et al.":""} · ${p.venue} · ${p.year} · 引用 ${p.citation_count}</p>
        <p class="text-xs text-slate-400 mt-1.5 ml-7">${esc(sp.rationale)}</p>
        <div class="flex gap-3 mt-2 ml-7">
          <button onclick="document.getElementById('abstract-${uid}').classList.toggle('hidden')" class="text-xs text-indigo-500 hover:text-indigo-700 font-medium">摘要</button>
          <button onclick="window.open('${esc(p.url)}','_blank')" class="text-xs text-slate-400 hover:text-slate-600">DOI ↗</button>
          <button onclick="findSimilar('${esc(p.title.replace(/'/g, "\\'"))}')" class="text-xs text-slate-400 hover:text-indigo-600" title="用这篇论文的标题作为新查询">🔗 相似</button>
        </div>
        <div id="abstract-${uid}" class="hidden mt-2 ml-7 text-xs text-slate-500 bg-slate-50 rounded-lg p-3">${esc(p.abstract_text)}</div>
      </div></div>
    </div>`;
  });
  return h;
}

function renderResultsFlatInto(container: HTMLElement, r: SearchResult) {
  container.innerHTML = renderResultsFlat(r);
  showExportBar();
}

function bindAbstractToggles(container: HTMLElement) {
  container.querySelectorAll("[data-toggle-abstract]").forEach(b => {
    b.addEventListener("click", () => {
      document.getElementById(`abstract-${b.getAttribute("data-toggle-abstract")}`)?.classList.toggle("hidden");
    });
  });
}

async function deleteConv(id: string) {
  await invoke("delete_conversation", { id });
  if (id === activeConvId) activeConvId = null;
  refreshConversations();
}

// ── Settings ──────────────────────────────────────
let editingProfileId: string | null = null;
let localProfiles: LlmProfile[] = [];

async function openSettings() {
  try {
    appConfig = await invoke<AppConfig>("get_config");
    localProfiles = [...appConfig.llm.profiles];
    editingProfileId = appConfig.llm.active_profile_id;
    renderProfiles();
    selectProfile(appConfig.llm.active_profile_id);
    (document.getElementById("edit-max-llm") as HTMLInputElement).value = String(appConfig.budget?.max_llm_calls ?? 10);
    (document.getElementById("edit-max-search") as HTMLInputElement).value = String(appConfig.budget?.max_search_calls ?? 30);
  } catch (_) {}
  document.getElementById("settings-modal")!.classList.remove("hidden");
}

function closeSettings() { document.getElementById("settings-modal")!.classList.add("hidden"); }

function renderProfiles() {
  const list = document.getElementById("profile-list")!;
  list.innerHTML = localProfiles.map(p => `
    <div class="flex items-center gap-2 p-3 rounded-xl border cursor-pointer transition ${p.id === editingProfileId ? 'border-primary bg-indigo-50' : 'border-slate-200 hover:border-slate-300'}" onclick="selectProfile('${p.id}')">
      <div class="flex-1 min-w-0">
        <p class="text-sm font-medium text-slate-700 truncate">${esc(p.name)}</p>
        <p class="text-xs text-slate-400 truncate">${esc(p.model)} @ ${esc(p.base_url)}</p>
      </div>
      ${localProfiles.length > 1 ? `<button class="text-slate-300 hover:text-red-500 text-xs flex-shrink-0" onclick="event.stopPropagation();deleteProfile('${p.id}')">删除</button>` : ""}
    </div>
  `).join("");
}

function selectProfile(id: string) {
  editingProfileId = id;
  const p = localProfiles.find(x => x.id === id);
  if (!p) return;
  (document.getElementById("edit-name") as HTMLInputElement).value = p.name;
  (document.getElementById("edit-base-url") as HTMLInputElement).value = p.base_url;
  (document.getElementById("edit-model") as HTMLInputElement).value = p.model;
  (document.getElementById("edit-api-key") as HTMLInputElement).value = p.api_key;
  renderProfiles();
}

function addProfile() {
  const id = "p_" + Date.now().toString(36);
  const p: LlmProfile = { id, name: "新配置", provider: "openai", model: "deepseek-chat", api_key: "", base_url: "https://api.deepseek.com" };
  localProfiles.push(p);
  editingProfileId = id;
  renderProfiles();
  selectProfile(id);
}

function deleteProfile(id: string) {
  if (localProfiles.length <= 1) return;
  localProfiles = localProfiles.filter(p => p.id !== id);
  if (editingProfileId === id) editingProfileId = localProfiles[0]?.id || null;
  renderProfiles();
  if (editingProfileId) selectProfile(editingProfileId);
}

async function saveSettings() {
  // Sync editor fields to selected profile
  const p = localProfiles.find(x => x.id === editingProfileId);
  if (!p) return;
  p.name = (document.getElementById("edit-name") as HTMLInputElement).value.trim() || p.name;
  p.base_url = (document.getElementById("edit-base-url") as HTMLInputElement).value.trim() || p.base_url;
  p.model = (document.getElementById("edit-model") as HTMLInputElement).value.trim() || p.model;
  p.api_key = (document.getElementById("edit-api-key") as HTMLInputElement).value.trim();

  if (!appConfig) return;
  appConfig.llm.profiles = localProfiles;
  appConfig.llm.active_profile_id = editingProfileId!;
  if (!appConfig.budget) appConfig.budget = {};
  appConfig.budget.max_llm_calls = parseInt((document.getElementById("edit-max-llm") as HTMLInputElement).value) || 10;
  appConfig.budget.max_search_calls = parseInt((document.getElementById("edit-max-search") as HTMLInputElement).value) || 30;
  try {
    await invoke("update_config", { newConfig: appConfig });
    updateProfileBadge();
    closeSettings();
  } catch (err) { alert("保存失败: " + err); }
}

async function updateProfileBadge() {
  try {
    appConfig = await invoke<AppConfig>("get_config");
    const active = appConfig.llm.profiles.find(p => p.id === appConfig!.llm.active_profile_id);
    const badge = document.getElementById("active-profile-badge")!;
    badge.textContent = active ? active.name : "未配置";
    badge.className = active?.api_key ? "text-xs text-emerald-600 bg-emerald-50 px-2 py-1 rounded-full" : "text-xs text-amber-600 bg-amber-50 px-2 py-1 rounded-full";
  } catch (_) {}
}

// ── Init ──────────────────────────────────────────
async function initApp() {
  await updateProfileBadge();
  try {
    const cfg = await invoke<AppConfig>("get_config");
    appConfig = cfg;
    const hasKey = cfg.llm.profiles.find(p => p.id === cfg.llm.active_profile_id)?.api_key;
    if (!hasKey) openSettings();
  } catch (_) { openSettings(); }
  // Auto-show sidebar if there are saved conversations
  try {
    const cs = await invoke<Conversation[]>("get_conversations");
    if (cs.length > 0) document.getElementById("sidebar")!.classList.remove("hidden");
    refreshConversations();
  } catch (_) {}
}
initApp();

// ── Dark Mode ─────────────────────────────────────
function toggleDarkMode() {
  document.documentElement.classList.toggle('dark');
  localStorage.setItem('dark', document.documentElement.classList.contains('dark') ? '1' : '0');
}
// Apply saved preference
(function() { if (localStorage.getItem('dark') === '1') document.documentElement.classList.add('dark'); })();

// ── Export ────────────────────────────────────────
async function doExport(format: string) {
  if (!currentResult) return;
  const all = [...currentResult.tiers.high_relevance, ...currentResult.tiers.partial_relevance]
    .sort((a, b) => b.score - a.score);
  try {
    const text = await invoke<string>("export_papers", { papers: all, format });
    await navigator.clipboard.writeText(text);
    flashExportBtn(format);
  } catch (e) {
    alert('导出失败: ' + e);
  }
}
function flashExportBtn(format: string) {
  const btn = document.getElementById(`export-${format}`);
  if (!btn) return;
  const orig = btn.textContent;
  btn.textContent = '✅ 已复制!';
  btn.classList.add('bg-emerald-500', 'text-white');
  setTimeout(() => { btn.textContent = orig; btn.classList.remove('bg-emerald-500', 'text-white'); }, 1500);
}
function showExportBar() {
  const bar = document.getElementById('export-bar')!;
  bar.classList.remove('hidden');
  bar.innerHTML = `<span class="text-xs text-slate-400 dark:text-slate-500 mr-2">导出:</span>
    <button id="export-bibtex" onclick="doExport('bibtex')" class="text-xs px-3 py-1.5 bg-slate-100 dark:bg-slate-700 text-slate-600 dark:text-slate-300 rounded-lg hover:bg-slate-200 dark:hover:bg-slate-600 transition">BibTeX</button>
    <button id="export-markdown" onclick="doExport('markdown')" class="text-xs px-3 py-1.5 bg-slate-100 dark:bg-slate-700 text-slate-600 dark:text-slate-300 rounded-lg hover:bg-slate-200 dark:hover:bg-slate-600 transition">Markdown</button>
    <span class="text-[10px] text-slate-400 dark:text-slate-500 ml-auto">复制到剪贴板</span>`;
}

// ── Find Similar ──────────────────────────────────
(window as any).findSimilar = function(title: string) {
  (document.getElementById("query-input") as HTMLInputElement).value = title;
  document.getElementById("activity-feed")!.classList.add("hidden");
  doSearch();
};

// ── Year filter in refine ─────────────────────────
function getRefinementText(): string {
  const input = (document.getElementById("refine-input") as HTMLInputElement).value.trim();
  const yf = (document.getElementById("year-from") as HTMLInputElement).value.trim();
  const yt = (document.getElementById("year-to") as HTMLInputElement).value.trim();
  let text = input;
  if (yf || yt) {
    const from = yf || '不限';
    const to = yt || '不限';
    text = text ? `${text} (年份: ${from}-${to})` : `只看 ${from}-${to} 年的论文`;
  }
  return text;
}

// Exports
(window as any).doSearch = doSearch;
(window as any).doStop = doStop;
(window as any).toggleDarkMode = toggleDarkMode;
(window as any).doExport = doExport;
(window as any).doStop = doStop;
(window as any).doRefine = doRefine;
(window as any).switchView = switchView;
(window as any).newConversation = function() {
  activeConvId = null;
  currentResult = null;
  currentGraphData = null;
  (document.getElementById("query-input") as HTMLInputElement).value = "";
  (document.getElementById("refine-input") as HTMLInputElement).value = "";
  resetUI();
  document.getElementById("activity-feed")!.classList.add("hidden");
  document.getElementById("export-bar")!.classList.add("hidden");
};
(window as any).toggleSidebar = toggleSidebar;
(window as any).loadConversation = loadConversation;
(window as any).deleteConv = deleteConv;
(window as any).openSettings = openSettings;
(window as any).closeSettings = closeSettings;
(window as any).saveSettings = saveSettings;
(window as any).addProfile = addProfile;
(window as any).selectProfile = selectProfile;
(window as any).deleteProfile = deleteProfile;
