import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

declare const d3: any;

// ── Types ──────────────────────────────────────────
interface Paper { id: string; title: string; authors: string[]; year: number; venue: string; doi: string; abstract_text: string; citation_count: number; url: string; }
interface ScoredPaper { paper: Paper; score: number; rationale: string; }
interface SearchResult { summary: string; tiers: { high_relevance: ScoredPaper[]; partial_relevance: ScoredPaper[] }; total_candidates: number; rounds_used: number; }
interface ProgressEvent { phase: string; message: string; percent: number; detail: string; }
interface GraphData { nodes: { index: number; title: string; cluster: number }[]; edges: { source: number; target: number; relation: string }[]; }
interface AppConfig { llm: { provider: string; model: string; api_key: string; base_url: string }; }

// ── State ──────────────────────────────────────────
let currentResult: SearchResult | null = null;
let currentGraphData: GraphData | null = null;
let currentView: "list" | "graph" = "list";
let activitySteps: { icon: string; text: string; detail: string; done: boolean }[] = [];

const PHASES: Record<string, { icon: string; label: string }> = {
  decompose:      { icon: "🧠", label: "解析查询" },
  decompose_done: { icon: "✅", label: "查询分解" },
  search:         { icon: "🔍", label: "广度搜索" },
  search_done:    { icon: "📊", label: "搜索完成" },
  refine:         { icon: "🎯", label: "精细检索" },
  refine_done:    { icon: "📈", label: "检索结果" },
  cite_expand:    { icon: "🔗", label: "引用追踪" },
  cite_done:      { icon: "📎", label: "引用扩展" },
  rank:           { icon: "⭐", label: "AI 评分" },
  rank_done:      { icon: "🏆", label: "评分完成" },
  organize:       { icon: "📝", label: "生成摘要" },
  done:           { icon: "🎉", label: "搜索完成" },
};

// ── Activity Feed ──────────────────────────────────
function initActivityFeed() {
  activitySteps = [];
  renderActivity();
  document.getElementById("activity-feed")!.classList.remove("hidden");
}

function addStep(phase: string, message: string, detail: string) {
  const meta = PHASES[phase] || { icon: "⏳", label: phase };
  activitySteps.push({ icon: meta.icon, text: message, detail, done: phase.endsWith("_done") || phase === "done" });
  if (activitySteps.length > 20) activitySteps.shift();
  renderActivity();
}

function renderActivity() {
  const container = document.getElementById("activity-feed")!;
  let html = `<h3 class="text-xs font-semibold text-slate-400 uppercase tracking-wider mb-3">搜索进度</h3>`;
  for (let i = 0; i < activitySteps.length; i++) {
    const step = activitySteps[i];
    const isLast = i === activitySteps.length - 1;
    html += `
    <div class="flex gap-3">
      <div class="flex flex-col items-center">
        <span class="text-sm ${step.done ? "" : "animate-pulse"}">${step.icon}</span>
        ${!isLast ? '<div class="step-line flex-1 bg-slate-200"></div>' : ""}
      </div>
      <div class="pb-4 flex-1 min-w-0">
        <p class="text-sm ${step.done ? "text-slate-600" : "text-slate-900 font-medium"}">${step.text}</p>
        ${step.detail ? `<p class="text-xs text-slate-400 mt-0.5 truncate">${step.detail}</p>` : ""}
      </div>
    </div>`;
  }
  container.innerHTML = html;
  container.scrollTop = container.scrollHeight;
}

// ── Search ─────────────────────────────────────────
async function doSearch() {
  const input = document.getElementById("query-input") as HTMLInputElement;
  const query = input.value.trim();
  if (!query) return;

  document.getElementById("results-container")!.innerHTML = "";
  document.getElementById("view-toggle")!.classList.add("hidden");
  document.getElementById("graph-container")!.classList.add("hidden");
  initActivityFeed();

  const btn = document.getElementById("search-btn") as HTMLButtonElement;
  btn.disabled = true;
  btn.innerHTML = `<svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"/></svg> 搜索中...`;

  const unlisten = await listen<ProgressEvent>("progress", (event) => {
    const p = event.payload;
    let detail = "";
    try {
      const d = JSON.parse(p.detail);
      if (d.sub_queries) detail = d.sub_queries.join(" · ");
      else if (d.found !== undefined) detail = `发现 ${d.found} 篇论文`;
      else if (d.added !== undefined) detail = `新增 ${d.added} 篇`;
      else if (d.high !== undefined) detail = `高度相关 ${d.high} 篇 · 部分相关 ${d.partial} 篇`;
      else if (d.candidates !== undefined) detail = `${d.candidates} 篇 · 分 ${d.batches} 批`;
      else if (d.top_papers) detail = `正在追踪: ${d.top_papers.slice(0,2).join(" · ")}`;
    } catch (_) {}
    addStep(p.phase, p.message, detail);
  });

  try {
    currentResult = await invoke<SearchResult>("search", { query });
    currentGraphData = buildGraphData(currentResult);
    renderResults(currentResult);
    document.getElementById("view-toggle")!.classList.remove("hidden");
  } catch (err) {
    document.getElementById("results-container")!.innerHTML =
      `<div class="bg-red-50 border border-red-200 rounded-2xl p-5 text-red-600 text-sm">搜索失败: ${err}</div>`;
  } finally {
    unlisten();
    btn.disabled = false;
    btn.innerHTML = `<svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"/></svg> 搜索`;
  }
}

// ── Render Results ─────────────────────────────────
function renderResults(result: SearchResult) {
  const container = document.getElementById("results-container")!;
  const { high_relevance, partial_relevance } = result.tiers;

  let html = `
  <div class="bg-gradient-to-r from-indigo-50 to-purple-50 rounded-2xl border border-indigo-100 p-5 mb-6">
    <div class="flex items-start gap-3">
      <span class="text-2xl">📋</span>
      <div>
        <p class="text-sm text-slate-700 leading-relaxed">${result.summary}</p>
        <div class="flex gap-4 mt-3">
          <span class="text-xs text-slate-500 bg-white/70 rounded-lg px-2.5 py-1">候选 ${result.total_candidates} 篇</span>
          <span class="text-xs text-slate-500 bg-white/70 rounded-lg px-2.5 py-1">搜索 ${result.rounds_used} 轮</span>
        </div>
      </div>
    </div>
  </div>`;

  html += renderTier("高度相关", high_relevance, "emerald");
  html += renderTier("部分相关", partial_relevance, "amber");
  container.innerHTML = html;

  container.querySelectorAll("[data-toggle-abstract]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const id = btn.getAttribute("data-toggle-abstract");
      document.getElementById(`abstract-${id}`)?.classList.toggle("hidden");
    });
  });
}

function renderTier(label: string, papers: ScoredPaper[], color: string): string {
  if (papers.length === 0) return "";
  const colors: Record<string, { badge: string; badgeBg: string; dot: string }> = {
    emerald: { badge: "text-emerald-700", badgeBg: "bg-emerald-50", dot: "bg-emerald-500" },
    amber: { badge: "text-amber-700", badgeBg: "bg-amber-50", dot: "bg-amber-500" },
  };
  const c = colors[color];

  let html = `<div class="mb-6">
    <div class="flex items-center gap-2 mb-3">
      <div class="w-2 h-2 rounded-full ${c.dot}"></div>
      <h2 class="text-base font-semibold text-slate-800">${label}</h2>
      <span class="text-xs text-slate-500 bg-slate-100 rounded-full px-2 py-0.5">${papers.length}</span>
    </div>`;

  for (let i = 0; i < papers.length; i++) {
    const sp = papers[i], p = sp.paper;
    html += `
    <div class="paper-card bg-white border border-slate-200 rounded-xl p-4 mb-2.5 transition cursor-default">
      <div class="flex items-start justify-between gap-4">
        <div class="flex-1 min-w-0">
          <div class="flex items-center gap-2 mb-1">
            <span class="text-xs text-slate-400 font-mono flex-shrink-0">#${i + 1}</span>
            <span class="${c.badge} text-[11px] font-semibold px-1.5 py-0.5 rounded ${c.badgeBg} flex-shrink-0">${sp.score}/10</span>
            <h3 class="text-sm font-semibold text-slate-800 truncate">${esc(p.title)}</h3>
          </div>
          <p class="text-xs text-slate-500 ml-7">${p.authors.slice(0, 3).join(", ")}${p.authors.length > 3 ? " et al." : ""} · ${p.venue} · ${p.year} · 引用 ${p.citation_count}</p>
          <p class="text-xs text-slate-400 mt-1.5 ml-7 leading-relaxed">${esc(sp.rationale)}</p>
          <div class="flex gap-3 mt-2 ml-7">
            <button data-toggle-abstract="${i}" class="text-xs text-indigo-500 hover:text-indigo-700 transition font-medium">展开摘要</button>
            <a href="${p.url}" target="_blank" class="text-xs text-slate-400 hover:text-slate-600 transition">DOI ↗</a>
          </div>
          <div id="abstract-${i}" class="hidden mt-2 ml-7 text-xs text-slate-500 leading-relaxed bg-slate-50 rounded-lg p-3">${esc(p.abstract_text)}</div>
        </div>
      </div>
    </div>`;
  }
  html += "</div>";
  return html;
}

function esc(text: string): string {
  const div = document.createElement("div");
  div.textContent = text;
  return div.innerHTML;
}

// ── Graph ──────────────────────────────────────────
function buildGraphData(result: SearchResult): GraphData {
  const all = [...result.tiers.high_relevance, ...result.tiers.partial_relevance];
  const nodes = all.map((sp, i) => ({ index: i, title: sp.paper.title, cluster: sp.score >= 7 ? 0 : 1 }));
  const edges: { source: number; target: number; relation: string }[] = [];
  for (let i = 0; i < nodes.length && i < 50; i++) {
    for (let j = i + 1; j < nodes.length && j < 50; j++) {
      const wi = nodes[i].title.toLowerCase().split(/\s+/);
      const wj = nodes[j].title.toLowerCase().split(/\s+/);
      if (wi.filter(w => w.length > 3 && wj.includes(w)).length >= 3) {
        edges.push({ source: i, target: j, relation: "topic_similarity" });
      }
    }
  }
  return { nodes, edges };
}

function switchView(view: "list" | "graph") {
  currentView = view;
  const bl = document.getElementById("btn-list")!, bg = document.getElementById("btn-graph")!;
  bl.className = view === "list" ? "px-4 py-1.5 text-sm rounded-lg bg-primary text-white transition font-medium" : "px-4 py-1.5 text-sm rounded-lg bg-slate-200 text-slate-600 hover:bg-slate-300 transition font-medium";
  bg.className = view === "graph" ? "px-4 py-1.5 text-sm rounded-lg bg-primary text-white transition font-medium" : "px-4 py-1.5 text-sm rounded-lg bg-slate-200 text-slate-600 hover:bg-slate-300 transition font-medium";
  document.getElementById("graph-container")!.classList.toggle("hidden", view !== "graph");
  document.getElementById("results-container")!.classList.toggle("hidden", view === "graph");
  if (view === "graph" && currentGraphData) renderGraph(currentGraphData);
}

function renderGraph(data: GraphData) {
  const container = document.getElementById("graph-container")!;
  container.innerHTML = "";
  const w = container.clientWidth, h = 520;
  const svg = d3.select("#graph-container").append("svg").attr("width", w).attr("height", h);
  const sim = d3.forceSimulation(data.nodes as any)
    .force("link", d3.forceLink(data.edges).id((d: any) => d.index).distance(100))
    .force("charge", d3.forceManyBody().strength(-300))
    .force("center", d3.forceCenter(w / 2, h / 2));
  const link = svg.append("g").selectAll("line").data(data.edges).join("line").attr("stroke", "#e2e8f0").attr("stroke-width", 1.5);
  const node = svg.append("g").selectAll("circle").data(data.nodes).join("circle")
    .attr("r", 9).attr("fill", (d: any) => d.cluster === 0 ? "#4F46E5" : "#F59E0B").attr("stroke", "#fff").attr("stroke-width", 2)
    .call(d3.drag().on("start", (e: any, d: any) => { if (!e.active) sim.alphaTarget(0.3).restart(); d.fx = d.x; d.fy = d.y; })
      .on("drag", (e: any, d: any) => { d.fx = e.x; d.fy = e.y; })
      .on("end", (e: any, d: any) => { if (!e.active) sim.alphaTarget(0); d.fx = null; d.fy = null; }))
    .append("title").text((d: any) => d.title);
  sim.on("tick", () => { link.attr("x1", (d: any) => d.source.x).attr("y1", (d: any) => d.source.y).attr("x2", (d: any) => d.target.x).attr("y2", (d: any) => d.target.y); node.attr("cx", (d: any) => d.x).attr("cy", (d: any) => d.y); });
}

// ── Settings ──────────────────────────────────────
async function openSettings() {
  try {
    const cfg = await invoke<AppConfig>("get_config");
    (document.getElementById("cfg-api-key") as HTMLInputElement).value = cfg.llm.api_key;
    (document.getElementById("cfg-base-url") as HTMLInputElement).value = cfg.llm.base_url;
    (document.getElementById("cfg-model") as HTMLInputElement).value = cfg.llm.model;
  } catch (_) {}
  document.getElementById("settings-modal")!.classList.remove("hidden");
}

function closeSettings() { document.getElementById("settings-modal")!.classList.add("hidden"); }

async function saveSettings() {
  const api_key = (document.getElementById("cfg-api-key") as HTMLInputElement).value.trim();
  const base_url = (document.getElementById("cfg-base-url") as HTMLInputElement).value.trim();
  const model = (document.getElementById("cfg-model") as HTMLInputElement).value.trim();
  if (!api_key) { alert("API Key 不能为空"); return; }
  try {
    const cfg = await invoke<AppConfig>("get_config");
    cfg.llm.api_key = api_key;
    cfg.llm.base_url = base_url || "https://api.deepseek.com";
    cfg.llm.model = model || "deepseek-chat";
    await invoke("update_config", { newConfig: cfg });
    closeSettings();
  } catch (err) { alert("保存失败: " + err); }
}

// ── Init ──────────────────────────────────────────
async function initApp() {
  try { const cfg = await invoke<AppConfig>("get_config"); if (!cfg.llm.api_key) openSettings(); } catch (_) {}
}
initApp();

// Exports
(window as any).doSearch = doSearch;
(window as any).switchView = switchView;
(window as any).openSettings = openSettings;
(window as any).closeSettings = closeSettings;
(window as any).saveSettings = saveSettings;
