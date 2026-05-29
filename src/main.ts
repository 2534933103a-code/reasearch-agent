import { invoke } from "@tauri-apps/api/core";

declare const d3: any;

// ── Types ──────────────────────────────────────────
interface Paper { id: string; title: string; authors: string[]; year: number; venue: string; doi: string; abstract_text: string; citation_count: number; url: string; }
interface ScoredPaper { paper: Paper; score: number; rationale: string; }
interface SearchResult { summary: string; tiers: { high_relevance: ScoredPaper[]; partial_relevance: ScoredPaper[] }; total_candidates: number; rounds_used: number; }
interface ProgressEvent { phase: string; message: string; percent: number; detail: string; }
interface LlmProfile { id: string; name: string; provider: string; model: string; api_key: string; base_url: string; }
interface LlmConfig { profiles: LlmProfile[]; active_profile_id: string; }
interface AppConfig { llm: LlmConfig; search: any; budget: any; }
interface Conversation { id: string; title: string; messages: { role: string; content: string; timestamp: number }[]; search_result: SearchResult | null; created_at: number; }

// ── State ──────────────────────────────────────────
let currentResult: SearchResult | null = null;
let currentGraphData: any = null;
let currentView: "list" | "graph" = "list";
let activeConvId: string | null = null;
let activitySteps: { icon: string; text: string; detail: string; done: boolean }[] = [];
let appConfig: AppConfig | null = null;

const PHASES: Record<string, string> = {
  start:"🔌", decompose:"🧠", decompose_done:"✅", search:"🔍", search_detail:"🔎", search_done:"📊",
  refine:"🎯", refine_done:"📈", cite_expand:"🔗", cite_done:"📎", rank:"⭐", rank_done:"🏆", organize:"📝", done:"🎉", error:"❌",
};

// ── Activity Feed ──────────────────────────────────
function initActivityFeed() {
  activitySteps = [{ icon: "⏳", text: "引擎启动，等待连接...", detail: "正在向 LLM 发送请求", done: false }];
  renderActivity();
  document.getElementById("activity-feed")!.classList.remove("hidden");
}

function addStep(phase: string, msg: string, detail: string) {
  const icon = PHASES[phase] || "⏳";
  const done = phase.endsWith("_done") || phase === "done" || phase === "error";
  if (activitySteps.length === 1 && activitySteps[0].icon === "⏳" && !done) {
    activitySteps[0] = { icon, text: msg, detail, done };
  } else {
    activitySteps.push({ icon: phase === "error" ? "❌" : icon, text: msg, detail, done });
  }
  if (activitySteps.length > 20) activitySteps.shift();
  renderActivity();
}

function renderActivity() {
  const c = document.getElementById("activity-feed")!;
  let h = `<h3 class="text-xs font-semibold text-slate-400 uppercase tracking-wider mb-3">搜索进度</h3>`;
  activitySteps.forEach((s, i) => {
    const last = i === activitySteps.length - 1;
    h += `<div class="flex gap-3"><div class="flex flex-col items-center"><span class="text-sm ${s.done ? "" : "animate-pulse"}">${s.icon}</span>${last ? "" : '<div class="step-line flex-1 bg-slate-200"></div>'}</div><div class="pb-4"><p class="text-sm ${s.done ? "text-slate-600" : "text-slate-900 font-medium"}">${s.text}</p>${s.detail ? `<p class="text-xs text-slate-400 mt-0.5 truncate">${s.detail}</p>` : ""}</div></div>`;
  });
  c.innerHTML = h;
  c.scrollTop = c.scrollHeight;
}

// ── Poll progress ──────────────────────────────────
async function pollProgress(pollTimer: number, lastCount: number): Promise<number> {
  try {
    const steps = await invoke<ProgressEvent[]>("get_progress");
    for (let i = lastCount; i < steps.length; i++) {
      const p = steps[i];
      let detail = "";
      try { const d = JSON.parse(p.detail); if (d.sub_queries) detail = d.sub_queries.join(" · "); else if (d.found !== undefined) detail = `发现 ${d.found} 篇`; else if (d.added !== undefined) detail = `+${d.added} 篇`; else if (d.high !== undefined) detail = `高 ${d.high} · 部分 ${d.partial}`; else if (d.top) detail = "追踪引用中"; } catch (_) {}
      addStep(p.phase, p.message, detail);
    }
    return steps.length;
  } catch (_) { return lastCount; }
}

// ── Search ─────────────────────────────────────────
async function doSearch() {
  const q = (document.getElementById("query-input") as HTMLInputElement).value.trim();
  if (!q) return;
  resetUI();
  initActivityFeed();
  const btn = document.getElementById("search-btn") as HTMLButtonElement;
  btn.disabled = true; btn.textContent = "搜索中...";

  let last = 0;
  const timer = setInterval(() => { pollProgress(timer, last).then(n => last = n); }, 500);

  try {
    currentResult = await invoke<SearchResult>("search", { query: q, conversationId: activeConvId });
    clearInterval(timer);
    await pollProgress(timer, last); // final flush
    activeConvId = currentResult.summary ? q.slice(0, 50) : activeConvId;
    currentGraphData = buildGraphData(currentResult);
    renderResults(currentResult);
    document.getElementById("refine-box")!.classList.remove("hidden");
    document.getElementById("view-toggle")!.classList.remove("hidden");
    refreshConversations();
  } catch (err) {
    clearInterval(timer);
    addStep("error", `搜索失败: ${err}`, "");
    document.getElementById("results-container")!.innerHTML = `<div class="bg-red-50 border border-red-200 rounded-2xl p-5 text-red-600 text-sm">${esc(String(err))}</div>`;
  } finally {
    btn.disabled = false; btn.textContent = "搜索";
    document.getElementById("activity-feed")!.classList.add("hidden");
  }
}

// ── Refine ─────────────────────────────────────────
async function doRefine() {
  const input = document.getElementById("refine-input") as HTMLInputElement;
  const refinement = input.value.trim();
  if (!refinement || !activeConvId) return;
  input.value = "";
  resetUI();
  initActivityFeed();
  const btn = document.getElementById("refine-btn") as HTMLButtonElement;
  btn.disabled = true; btn.textContent = "细化中...";

  let last = 0;
  const timer = setInterval(() => { pollProgress(timer, last).then(n => last = n); }, 500);

  try {
    currentResult = await invoke<SearchResult>("refine_search", { conversationId: activeConvId, refinement });
    clearInterval(timer);
    await pollProgress(timer, last);
    currentGraphData = buildGraphData(currentResult);
    renderResults(currentResult);
    document.getElementById("view-toggle")!.classList.remove("hidden");
    refreshConversations();
  } catch (err) {
    clearInterval(timer);
    addStep("error", `细化失败: ${err}`, "");
  } finally {
    btn.disabled = false; btn.textContent = "细化";
    document.getElementById("activity-feed")!.classList.add("hidden");
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
  let h = `<div class="bg-gradient-to-r from-indigo-50 to-purple-50 rounded-2xl border border-indigo-100 p-5 mb-6">
    <p class="text-sm text-slate-700 mb-2">${r.summary}</p>
    <div class="flex gap-3"><span class="text-xs text-slate-500 bg-white/80 rounded-lg px-2.5 py-1">候选 ${r.total_candidates} 篇</span><span class="text-xs text-slate-500 bg-white/80 rounded-lg px-2.5 py-1">${r.rounds_used} 轮搜索</span></div>
  </div>`;
  h += renderTier("高度相关", r.tiers.high_relevance, "emerald");
  h += renderTier("部分相关", r.tiers.partial_relevance, "amber");
  c.innerHTML = h;
  c.querySelectorAll("[data-toggle-abstract]").forEach(b => b.addEventListener("click", () => {
    document.getElementById(`abstract-${b.getAttribute("data-toggle-abstract")}`)?.classList.toggle("hidden");
  }));
}

function renderTier(label: string, papers: ScoredPaper[], color: string): string {
  if (!papers.length) return "";
  const clr = color === "emerald" ? { b: "text-emerald-700", bg: "bg-emerald-50", d: "bg-emerald-500" } : { b: "text-amber-700", bg: "bg-amber-50", d: "bg-amber-500" };
  let h = `<div class="mb-6"><div class="flex items-center gap-2 mb-3"><div class="w-2 h-2 rounded-full ${clr.d}"></div><h2 class="text-base font-semibold text-slate-800">${label}</h2><span class="text-xs text-slate-500 bg-slate-100 rounded-full px-2 py-0.5">${papers.length}</span></div>`;
  papers.forEach((sp, i) => {
    const p = sp.paper;
    h += `<div class="paper-card bg-white border border-slate-200 rounded-xl p-4 mb-2.5 transition">
      <div class="flex items-start justify-between gap-4"><div class="flex-1 min-w-0">
        <div class="flex items-center gap-2 mb-1"><span class="text-xs text-slate-400 font-mono">#${i+1}</span><span class="${clr.b} text-[11px] font-semibold px-1.5 py-0.5 rounded ${clr.bg}">${sp.score}/10</span><h3 class="text-sm font-semibold text-slate-800 truncate">${esc(p.title)}</h3></div>
        <p class="text-xs text-slate-500 ml-7">${p.authors.slice(0,3).join(", ")}${p.authors.length>3?" et al.":""} · ${p.venue} · ${p.year} · 引用 ${p.citation_count}</p>
        <p class="text-xs text-slate-400 mt-1.5 ml-7">${esc(sp.rationale)}</p>
        <div class="flex gap-3 mt-2 ml-7"><button data-toggle-abstract="${i}" class="text-xs text-indigo-500 hover:text-indigo-700 font-medium">摘要</button><a href="${p.url}" target="_blank" class="text-xs text-slate-400 hover:text-slate-600">DOI ↗</a></div>
        <div id="abstract-${i}" class="hidden mt-2 ml-7 text-xs text-slate-500 bg-slate-50 rounded-lg p-3">${esc(p.abstract_text)}</div>
      </div></div>
    </div>`;
  });
  return h + "</div>";
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
  const w = c.clientWidth, h = 520;
  const svg = d3.select("#graph-container").append("svg").attr("width", w).attr("height", h);
  const sim = d3.forceSimulation(data.nodes).force("link", d3.forceLink(data.edges).id((d: any) => d.index).distance(100)).force("charge", d3.forceManyBody().strength(-300)).force("center", d3.forceCenter(w / 2, h / 2));
  const link = svg.append("g").selectAll("line").data(data.edges).join("line").attr("stroke", "#e2e8f0").attr("stroke-width", 1.5);
  const node = svg.append("g").selectAll("circle").data(data.nodes).join("circle").attr("r", 9).attr("fill", (d: any) => d.cluster === 0 ? "#4F46E5" : "#F59E0B").attr("stroke", "#fff").attr("stroke-width", 2)
    .call(d3.drag().on("start", (e: any, d: any) => { if (!e.active) sim.alphaTarget(0.3).restart(); d.fx = d.x; d.fy = d.y; }).on("drag", (e: any, d: any) => { d.fx = e.x; d.fy = e.y; }).on("end", (e: any, d: any) => { if (!e.active) sim.alphaTarget(0); d.fx = null; d.fy = null; }))
    .append("title").text((d: any) => d.title);
  sim.on("tick", () => { link.attr("x1", (d: any) => d.source.x).attr("y1", (d: any) => d.source.y).attr("x2", (d: any) => d.target.x).attr("y2", (d: any) => d.target.y); node.attr("cx", (d: any) => d.x).attr("cy", (d: any) => d.y); });
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
    list.innerHTML = cs.map(c => `<div class="flex items-center gap-2 p-2 rounded-lg hover:bg-slate-50 cursor-pointer ${c.id === activeConvId ? 'bg-indigo-50 border border-indigo-100' : ''}" onclick="loadConversation('${c.id}')">
      <span class="text-xs truncate flex-1">${esc(c.title)}</span>
      <button class="text-slate-300 hover:text-red-500 text-xs flex-shrink-0" onclick="event.stopPropagation();deleteConv('${c.id}')">✕</button>
    </div>`).join("");
  } catch (_) {}
}

async function loadConversation(id: string) {
  try {
    const cs = await invoke<Conversation[]>("get_conversations");
    const conv = cs.find(c => c.id === id);
    if (!conv || !conv.search_result) return;
    activeConvId = id;
    currentResult = conv.search_result;
    currentGraphData = buildGraphData(currentResult);
    renderResults(currentResult);
    document.getElementById("refine-box")!.classList.remove("hidden");
    document.getElementById("view-toggle")!.classList.remove("hidden");
    document.getElementById("activity-feed")!.classList.add("hidden");
    (document.getElementById("query-input") as HTMLInputElement).value = conv.messages[0]?.content || "";
    refreshConversations();
  } catch (_) {}
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
}
initApp();

// Exports
(window as any).doSearch = doSearch;
(window as any).doRefine = doRefine;
(window as any).switchView = switchView;
(window as any).toggleSidebar = toggleSidebar;
(window as any).loadConversation = loadConversation;
(window as any).deleteConv = deleteConv;
(window as any).openSettings = openSettings;
(window as any).closeSettings = closeSettings;
(window as any).saveSettings = saveSettings;
(window as any).addProfile = addProfile;
(window as any).selectProfile = selectProfile;
(window as any).deleteProfile = deleteProfile;
