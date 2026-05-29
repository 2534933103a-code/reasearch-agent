import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

declare const d3: any; // loaded via CDN in index.html

// ── Types ──────────────────────────────────────────
interface Paper {
  id: string; title: string; authors: string[]; year: number;
  venue: string; doi: string; abstract_text: string;
  citation_count: number; url: string;
}

interface ScoredPaper { paper: Paper; score: number; rationale: string; }

interface SearchResult {
  summary: string;
  tiers: { high_relevance: ScoredPaper[]; partial_relevance: ScoredPaper[] };
  total_candidates: number; rounds_used: number;
}

interface ProgressEvent { phase: string; message: string; percent: number; }

interface GraphNode { index: number; title: string; cluster: number; }
interface GraphEdge { source: number; target: number; relation: string; }
interface GraphData { nodes: GraphNode[]; edges: GraphEdge[]; }

// ── State ──────────────────────────────────────────
let currentResult: SearchResult | null = null;
let currentGraphData: GraphData | null = null;
let currentView: "list" | "graph" = "list";

// ── Search ─────────────────────────────────────────
async function doSearch() {
  const input = document.getElementById("query-input") as HTMLInputElement;
  const query = input.value.trim();
  if (!query) return;

  document.getElementById("progress-container")!.classList.remove("hidden");
  document.getElementById("results-container")!.innerHTML = "";
  document.getElementById("view-toggle")!.classList.add("hidden");
  document.getElementById("graph-container")!.classList.add("hidden");

  const btn = document.getElementById("search-btn") as HTMLButtonElement;
  btn.disabled = true;
  btn.textContent = "搜索中...";

  const unlisten = await listen<ProgressEvent>("progress", (event) => {
    const p = event.payload;
    (document.getElementById("progress-bar") as HTMLElement).style.width = p.percent + "%";
    document.getElementById("progress-text")!.textContent = p.message;
  });

  try {
    currentResult = await invoke<SearchResult>("search", { query });
    currentGraphData = buildGraphData(currentResult);
    renderResults(currentResult);
    document.getElementById("view-toggle")!.classList.remove("hidden");
  } catch (err) {
    document.getElementById("results-container")!.innerHTML =
      `<div class="bg-red-50 border border-red-200 rounded-lg p-4 text-red-700">搜索失败: ${err}</div>`;
  } finally {
    unlisten();
    btn.disabled = false;
    btn.textContent = "搜索";
    document.getElementById("progress-container")!.classList.add("hidden");
  }
}

// ── Render ─────────────────────────────────────────
function renderResults(result: SearchResult) {
  const container = document.getElementById("results-container")!;
  const { high_relevance, partial_relevance } = result.tiers;

  let html = `<div class="bg-white rounded-xl shadow-sm border p-6 mb-4">
    <p class="text-gray-700">${result.summary}</p>
    <p class="text-xs text-gray-400 mt-2">候选 ${result.total_candidates} 篇 · 搜索 ${result.rounds_used} 轮</p>
  </div>`;

  html += renderTier("高度相关", high_relevance, "text-green-700", "bg-green-50");
  html += renderTier("部分相关", partial_relevance, "text-yellow-700", "bg-yellow-50");
  container.innerHTML = html;

  container.querySelectorAll("[data-toggle-abstract]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const id = btn.getAttribute("data-toggle-abstract");
      const el = document.getElementById(`abstract-${id}`);
      if (el) el.classList.toggle("hidden");
    });
  });
}

function renderTier(label: string, papers: ScoredPaper[], badgeColor: string, bgColor: string): string {
  if (papers.length === 0) return "";
  let html = `<h2 class="text-lg font-semibold mt-6 mb-3">${label} <span class="text-sm font-normal text-gray-400">(${papers.length})</span></h2>`;
  for (let i = 0; i < papers.length; i++) {
    const sp = papers[i], p = sp.paper;
    html += `<div class="bg-white border rounded-lg p-4 mb-3 hover:shadow-sm transition">
      <div class="flex items-start justify-between">
        <div class="flex-1">
          <h3 class="font-medium text-gray-900">[${i + 1}] ${esc(p.title)}</h3>
          <p class="text-sm text-gray-500 mt-1">${p.authors.slice(0, 3).join(", ")}${p.authors.length > 3 ? " et al." : ""} · ${p.venue} · ${p.year} · 引用 ${p.citation_count}</p>
        </div>
        <span class="${badgeColor} text-xs font-medium px-2 py-1 rounded ${bgColor} ml-3 whitespace-nowrap">⭐${sp.score}/10</span>
      </div>
      <p class="text-xs text-gray-400 mt-1">${esc(sp.rationale)}</p>
      <div class="mt-2">
        <button data-toggle-abstract="${i}" class="text-sm text-blue-600 hover:underline">展开摘要</button>
        <span class="text-xs text-gray-300 mx-2">|</span>
        <a href="${p.url}" target="_blank" class="text-sm text-blue-600 hover:underline">DOI</a>
      </div>
      <div id="abstract-${i}" class="hidden mt-2 text-sm text-gray-600 leading-relaxed">${esc(p.abstract_text)}</div>
    </div>`;
  }
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
  const nodes: GraphNode[] = all.map((sp, i) => ({
    index: i, title: sp.paper.title, cluster: sp.score >= 7 ? 0 : 1,
  }));
  const edges: GraphEdge[] = [];
  for (let i = 0; i < nodes.length && i < 50; i++) {
    for (let j = i + 1; j < nodes.length && j < 50; j++) {
      const wi = nodes[i].title.toLowerCase().split(/\s+/);
      const wj = nodes[j].title.toLowerCase().split(/\s+/);
      const common = wi.filter(w => w.length > 3 && wj.includes(w)).length;
      if (common >= 3) edges.push({ source: i, target: j, relation: "topic_similarity" });
    }
  }
  return { nodes, edges };
}

function switchView(view: "list" | "graph") {
  currentView = view;
  const btnList = document.getElementById("btn-list")!;
  const btnGraph = document.getElementById("btn-graph")!;
  btnList.className = view === "list" ? "px-3 py-1 text-sm rounded bg-blue-600 text-white" : "px-3 py-1 text-sm rounded bg-gray-200 text-gray-600";
  btnGraph.className = view === "graph" ? "px-3 py-1 text-sm rounded bg-blue-600 text-white" : "px-3 py-1 text-sm rounded bg-gray-200 text-gray-600";
  document.getElementById("graph-container")!.classList.toggle("hidden", view !== "graph");
  document.getElementById("results-container")!.classList.toggle("hidden", view === "graph");
  if (view === "graph" && currentGraphData) renderGraph(currentGraphData);
}

function renderGraph(data: GraphData) {
  const container = document.getElementById("graph-container")!;
  container.innerHTML = "";
  const width = container.clientWidth, height = 500;
  const svg = d3.select("#graph-container").append("svg").attr("width", width).attr("height", height);
  const sim = d3.forceSimulation(data.nodes as any)
    .force("link", d3.forceLink(data.edges).id((d: any) => d.index).distance(100))
    .force("charge", d3.forceManyBody().strength(-300))
    .force("center", d3.forceCenter(width / 2, height / 2));
  const link = svg.append("g").selectAll("line").data(data.edges).join("line").attr("stroke", "#ddd").attr("stroke-width", 1);
  const node = svg.append("g").selectAll("circle").data(data.nodes).join("circle")
    .attr("r", 8).attr("fill", (d: any) => d.cluster === 0 ? "#16a34a" : "#ca8a04")
    .call(d3.drag().on("start", (e: any, d: any) => { if (!e.active) sim.alphaTarget(0.3).restart(); d.fx = d.x; d.fy = d.y; })
      .on("drag", (e: any, d: any) => { d.fx = e.x; d.fy = e.y; })
      .on("end", (e: any, d: any) => { if (!e.active) sim.alphaTarget(0); d.fx = null; d.fy = null; }) as any)
    .append("title").text((d: any) => d.title);
  sim.on("tick", () => { link.attr("x1", (d: any) => d.source.x).attr("y1", (d: any) => d.source.y).attr("x2", (d: any) => d.target.x).attr("y2", (d: any) => d.target.y); node.attr("cx", (d: any) => d.x).attr("cy", (d: any) => d.y); });
}

// Expose to HTML onclick handlers
(window as any).doSearch = doSearch;
(window as any).switchView = switchView;
