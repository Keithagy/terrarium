// DOM panels: shelf (search, packages, boundaries, legend; traces live in traces.ts), specimen
// card (selected node), bench (status strip), overlays and toasts.

import { api, log } from "./tauri";
import { store, subscribe, emit, select, nodeVisible, setTrace } from "./store";
import { LANG_LABEL, type Boundary, type Lang, type NodeDetail, type ViewNode } from "./types";
import { setStage, traceFrom } from "./traces";

const $ = <T extends HTMLElement>(sel: string): T => document.querySelector(sel) as T;

export interface Actions {
  openRepo(path?: string): Promise<void>;
  setLevel(level: "package" | "file" | "symbol", focus?: number | null): Promise<void>;
  focusNode(id: number): Promise<void>;
  centerOn(id: number): void;
  relayout(): void;
  fit(): void;
}

let actions: Actions;
const BOUNDARY_TAGS = ["http-server", "http-client", "ipc-server", "ipc-client", "db", "queue", "fs", "env", "process"];

export function toast(message: string, kind: "ok" | "error" | "info" = "info", ms = 3200): void {
  const el = document.createElement("div");
  el.className = `toast is-${kind}`;
  el.textContent = message;
  el.dataset.testid = "toast";
  $("#toasts").appendChild(el);
  setTimeout(() => el.remove(), ms);
}

export function initPanels(a: Actions): void {
  actions = a;
  initShelf();
  initBench();
  initEmpty();
  initHelp();
  subscribe("graph", () => { renderPackages(); renderLegend(); renderBench(); void renderBoundaries(); });
  subscribe("trace", renderLegend);
  subscribe("filters", () => { renderLegend(); renderPackages(); });
  subscribe("selection", () => { void renderCard(); renderPackages(); });
  subscribe("layout", renderBench);
  subscribe("repo", () => { renderBench(); renderOverlays(); });
  subscribe("ui", () => { renderBench(); renderOverlays(); });
  renderOverlays();
}

// ---- shelf ------------------------------------------------------------------

function initShelf(): void {
  const input = $<HTMLInputElement>("#search");
  const results = $<HTMLUListElement>("#search-results");
  let active = -1;
  let hits: { id: number; name: string; path: string; kind: string; lang: string }[] = [];
  const render = () => {
    results.innerHTML = "";
    results.hidden = hits.length === 0;
    hits.forEach((h, i) => {
      const li = document.createElement("li");
      li.className = i === active ? "is-active" : "";
      li.dataset.testid = `search-result-${i}`;
      li.innerHTML = `<span class="dot" data-lang="${h.lang}"></span><span class="name">${esc(h.name)}</span><span class="path">${esc(h.path)}</span>`;
      li.addEventListener("mousedown", (e) => { e.preventDefault(); choose(i); });
      results.appendChild(li);
    });
  };
  const choose = (i: number) => {
    const h = hits[i];
    if (!h) return;
    hits = [];
    active = -1;
    render();
    input.value = "";
    store.search = "";
    void jumpTo(h.id);
  };
  let timer = 0;
  input.addEventListener("input", () => {
    store.search = input.value;
    clearTimeout(timer);
    if (!input.value.trim()) { hits = []; render(); return; }
    timer = window.setTimeout(async () => {
      try {
        hits = await api.search(input.value, 12);
        active = hits.length ? 0 : -1;
        render();
      } catch (e) { log("warn", `search failed: ${String(e)}`); }
    }, 90);
  });
  input.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown") { active = Math.min(active + 1, hits.length - 1); render(); e.preventDefault(); }
    else if (e.key === "ArrowUp") { active = Math.max(active - 1, 0); render(); e.preventDefault(); }
    else if (e.key === "Enter") { choose(active); e.preventDefault(); }
    else if (e.key === "Escape") { hits = []; render(); input.value = ""; store.search = ""; input.blur(); }
  });
  input.addEventListener("blur", () => { setTimeout(() => { hits = []; render(); }, 120); });
  document.querySelectorAll<HTMLButtonElement>(".tab").forEach((tab) => {
    tab.addEventListener("click", () => {
      document.querySelectorAll(".tab").forEach((t) => t.classList.toggle("is-active", t === tab));
      document.querySelectorAll<HTMLElement>(".tab-panel").forEach((p) => p.classList.toggle("is-active", p.dataset.tabPanel === tab.dataset.tab));
      store.shelfTab = tab.dataset.tab as typeof store.shelfTab;
      // Traces and endpoints read on the trace stage; packages and boundaries on the map.
      setStage(store.shelfTab === "traces" || store.shelfTab === "endpoints" ? "traces" : "map");
      emit("ui");
    });
  });
}

/** Select a node that may live at another level: switch level/focus as needed, then centre. */
export async function jumpTo(id: number): Promise<void> {
  setStage("map");
  if (store.index.has(id)) {
    select(id);
    actions.centerOn(id);
    return;
  }
  let detail: NodeDetail;
  try { detail = await api.getNode(id); } catch (e) { toast(`Cannot open node ${id}: ${String(e)}`, "error"); return; }
  const kind = detail.node.kind;
  if (kind === "package" || kind === "file" || kind === "symbol") {
    if (kind === "symbol" && store.level === "file" && detail.node.parent !== null) {
      // expand the parent file in place rather than switching the whole level
      await actions.setLevel("file", detail.node.parent);
    } else {
      await actions.setLevel(kind);
    }
  }
  if (store.index.has(id)) {
    select(id);
    actions.centerOn(id);
  } else {
    toast(`${detail.node.name} is hidden by the current filters`, "info");
  }
}

function renderPackages(): void {
  const el = $("[data-tab-panel=packages]");
  el.innerHTML = "";
  if (!store.graphLoaded) return;
  // Group visible nodes by package; at the package level list the packages themselves.
  const groups = new Map<number, ViewNode[]>();
  for (const n of store.nodes) {
    // dependencies sit under their importing package but read better as their own group
    const key = n.external && store.level !== "package" ? -1 : n.group;
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key)!.push(n);
  }
  const frag = document.createDocumentFragment();
  const entries = [...groups.entries()].sort((a, b) => b[1].length - a[1].length);
  for (const [gid, members] of entries) {
    const head = members.find((m) => m.id === gid);
    const title = document.createElement("div");
    title.className = "group-title";
    const external = gid === -1;
    const label = external ? "dependencies" : head ? head.name : members[0].group_name || `group ${gid}`;
    title.textContent = label;
    if (store.level !== "package" || !head) frag.appendChild(title);
    const list = store.level === "package" ? members : members.filter((m) => m.id !== gid).slice(0, 400);
    for (const n of list) {
      const b = document.createElement("button");
      b.className = `row${n.id === store.selection ? " is-selected" : ""}${nodeVisible(n) ? "" : " is-dim"}`;
      b.dataset.testid = `node-${n.id}`;
      b.dataset.nodeId = String(n.id);
      b.innerHTML = `<span class="dot${n.external ? " is-external" : ""}" data-lang="${n.lang}"></span><span class="name" title="${esc(n.path)}">${esc(n.name)}</span><span class="meta">${n.external ? "" : n.kind === "symbol" ? n.degree : n.loc}</span>`;
      b.addEventListener("click", () => { select(n.id); actions.centerOn(n.id); });
      b.addEventListener("dblclick", () => void actions.focusNode(n.id));
      frag.appendChild(b);
    }
    if (members.length > 401) {
      const more = document.createElement("div");
      more.className = "group-title";
      more.textContent = `and ${members.length - 401} more`;
      frag.appendChild(more);
    }
  }
  el.appendChild(frag);
}

async function renderBoundaries(): Promise<void> {
  const el = $("[data-tab-panel=boundaries]");
  el.innerHTML = "";
  if (!store.graphLoaded) return;
  let rows: Boundary[] = [];
  try { rows = await api.boundaries(); } catch { return; }
  if (rows.length === 0) {
    el.innerHTML = `<div class="group-title">Nothing here touches the outside world yet.</div>`;
    return;
  }
  const frag = document.createDocumentFragment();
  for (const tag of BOUNDARY_TAGS) {
    const members = rows.filter((r) => r.tags.includes(tag));
    if (!members.length) continue;
    const title = document.createElement("button");
    title.className = "group-title row";
    title.dataset.testid = `boundary-${tag}`;
    title.textContent = `${tagLabel(tag)} (${members.length})`;
    title.title = "Click to filter the graph to this boundary";
    title.addEventListener("click", () => { store.filters.tag = store.filters.tag === tag ? "" : tag; emit("filters"); });
    frag.appendChild(title);
    for (const r of members.slice(0, 80)) {
      const b = document.createElement("button");
      b.className = "row";
      b.dataset.testid = `boundary-node-${r.id}`;
      const keyed = r.tags.filter((t) => t.includes(":")).slice(0, 2).join(" ");
      b.innerHTML = `<span class="dot" data-lang="${r.lang}"></span><span class="name" title="${esc(r.path)}">${esc(r.name)}</span><span class="meta mono">${esc(keyed)}</span>`;
      b.addEventListener("click", () => void jumpTo(r.id));
      frag.appendChild(b);
    }
  }
  el.appendChild(frag);
}

function tagLabel(t: string): string {
  return ({ "http-server": "HTTP routes", "http-client": "HTTP calls", "ipc-server": "IPC handlers", "ipc-client": "IPC calls", db: "Database", queue: "Queues", fs: "Files on disk", env: "Environment", process: "Processes" } as Record<string, string>)[t] ?? t;
}

function renderLegend(): void {
  const el = $("#shelf .legend");
  el.innerHTML = "";
  if (!store.graphLoaded) return;
  const counts = new Map<Lang, number>();
  for (const n of store.nodes) counts.set(n.lang, (counts.get(n.lang) ?? 0) + 1);
  const langs = [...counts.entries()].sort((a, b) => b[1] - a[1]);
  for (const [lang, count] of langs) {
    const b = document.createElement("button");
    b.dataset.testid = `lang-${lang}`;
    b.className = store.filters.langs.has(lang) ? "" : "is-off";
    b.title = `${count} ${LANG_LABEL[lang]} nodes. Click to hide or show.`;
    b.innerHTML = `<span class="dot" data-lang="${lang}"></span>${LANG_LABEL[lang]} <span class="meta">${count}</span>`;
    b.addEventListener("click", () => {
      if (store.filters.langs.has(lang)) store.filters.langs.delete(lang); else store.filters.langs.add(lang);
      emit("filters");
    });
    el.appendChild(b);
  }
  const sep = document.createElement("span");
  sep.className = "sep";
  el.appendChild(sep);
  for (const kind of ["imports", "calls", "flow"] as const) {
    const b = document.createElement("button");
    b.dataset.testid = `edge-${kind}`;
    b.className = store.filters.edges.has(kind) ? "" : "is-off";
    b.innerHTML = `<span class="edge-swatch" data-edge="${kind}"></span>${kind === "flow" ? "data flows" : kind}`;
    b.addEventListener("click", () => {
      if (store.filters.edges.has(kind)) store.filters.edges.delete(kind); else store.filters.edges.add(kind);
      emit("filters");
    });
    el.appendChild(b);
  }
  const ext = document.createElement("button");
  ext.dataset.testid = "toggle-externals";
  ext.className = store.filters.externals ? "" : "is-off";
  ext.innerHTML = `<span class="dot is-external" style="color:var(--fern)"></span>dependencies`;
  ext.addEventListener("click", () => { store.filters.externals = !store.filters.externals; emit("filters"); });
  el.appendChild(ext);
  if (store.trace && store.traceOnMap) {
    const t = document.createElement("button");
    t.dataset.testid = "clear-trace";
    t.title = "Stop highlighting this trace";
    t.innerHTML = `<span class="chip is-boundary">trace ${esc(store.trace.name)} ×</span>`;
    t.addEventListener("click", () => { store.traceOnMap = false; setTrace(store.trace); });
    el.appendChild(t);
  }
  if (store.filters.tag) {
    const t = document.createElement("button");
    t.dataset.testid = "clear-tag";
    t.innerHTML = `<span class="chip is-boundary">${esc(tagLabel(store.filters.tag))} ×</span>`;
    t.addEventListener("click", () => { store.filters.tag = ""; emit("filters"); });
    el.appendChild(t);
  }
}

// ---- specimen card ---------------------------------------------------------

async function renderCard(): Promise<void> {
  const card = $("#card");
  const id = store.selection;
  if (id === null) { card.hidden = true; card.innerHTML = ""; return; }
  let d: NodeDetail;
  try { d = await api.getNode(id); } catch (e) { log("warn", `node detail failed: ${String(e)}`); return; }
  if (store.selection !== id) return;
  const n = d.node;
  const tags = n.tags ?? [];
  const kindLine = n.kind === "symbol" ? `${n.symbol_kind ?? "symbol"} in ${d.file ?? ""}` : n.kind === "file" ? `file in ${d.package ?? ""}` : n.external ? "dependency" : "package";
  const ins = d.neighbours.filter((x) => x.direction === "in");
  const outs = d.neighbours.filter((x) => x.direction === "out");
  const nb = (list: typeof ins, title: string) => list.length ? `<div class="card-section"><h3>${title} (${list.length})</h3>${list.slice(0, 40).map((x) => `<button class="nb${x.edge === "flow" ? " is-flow" : ""}" data-jump="${x.id}" data-testid="nb-${x.id}"><span class="arrow">${x.direction === "in" ? "←" : "→"}</span><span class="nb-name" title="${esc(x.path)}">${esc(x.kind === "symbol" ? x.name : short(x.path))}</span><span class="nb-label">${esc(x.edge === "calls" ? "calls" : x.edge === "imports" ? "imports" : x.label ?? "flow")}</span></button>`).join("")}${list.length > 40 ? `<div class="group-title">and ${list.length - 40} more</div>` : ""}</div>` : "";
  card.innerHTML = `
    <button class="card-close" data-testid="card-close" title="Close (Esc)">×</button>
    <div class="card-kind"><span class="dot" data-lang="${n.lang}"></span>${esc(LANG_LABEL[n.lang])} · ${esc(kindLine)}</div>
    <h2 data-testid="card-title">${esc(n.name)}</h2>
    <div class="card-path" data-testid="card-path">${esc(n.path)}${n.span ? `:${n.span[0]}` : ""}</div>
    ${tags.length ? `<div class="chips">${tags.map((t) => `<span class="chip${BOUNDARY_TAGS.includes(t) || t.includes(":") ? " is-boundary" : ""}">${esc(t)}</span>`).join("")}</div>` : ""}
    <div class="card-actions">
      ${n.external ? "" : `<button class="ghost" data-testid="open-file">Open in editor</button>`}
      ${d.children.length && n.kind !== "symbol" ? `<button class="ghost" data-testid="expand">Show ${d.children.length} inside</button>` : ""}
      ${n.external || n.kind === "package" ? "" : `<button class="ghost" data-testid="trace-from" title="Follow the calls from here across every boundary">Trace from here</button>`}
      ${store.stage === "traces" ? `<button class="ghost" data-testid="card-show-map">Show on map</button>` : ""}
    </div>
    <div class="card-facts">
      <span>Lines</span><b>${n.loc.toLocaleString()}</b>
      <span>Incoming</span><b>${ins.reduce((s, x) => s + x.weight, 0)}</b>
      <span>Outgoing</span><b>${outs.reduce((s, x) => s + x.weight, 0)}</b>
      ${d.children.length ? `<span>Contains</span><b>${d.children.length}</b>` : ""}
    </div>
    ${nb(ins, "Used by")}
    ${nb(outs, "Uses")}
  `;
  card.hidden = false;
  card.querySelector("[data-testid=card-close]")!.addEventListener("click", () => select(null));
  card.querySelector("[data-testid=open-file]")?.addEventListener("click", () => void api.openPath(n.path, n.span?.[0]).catch((e) => toast(`Cannot open: ${String(e)}`, "error")));
  card.querySelector("[data-testid=expand]")?.addEventListener("click", () => void actions.focusNode(n.id));
  card.querySelector("[data-testid=trace-from]")?.addEventListener("click", () => void traceFrom(n.id));
  card.querySelector("[data-testid=card-show-map]")?.addEventListener("click", () => void jumpTo(n.id));
  // On the trace stage, neighbours open in the card so the diagram stays put.
  card.querySelectorAll<HTMLButtonElement>("[data-jump]").forEach((b) => b.addEventListener("click", () => {
    const id = Number(b.dataset.jump);
    if (store.stage === "traces") select(id); else void jumpTo(id);
  }));
}

// ---- bench ------------------------------------------------------------------

function initBench(): void {
  $("#repo-name").addEventListener("click", () => void actions.openRepo());
  document.querySelectorAll<HTMLButtonElement>("[data-level]").forEach((b) => b.addEventListener("click", () => void actions.setLevel(b.dataset.level as "package" | "file" | "symbol")));
  $("#layout-status").addEventListener("click", () => actions.relayout());
}

function renderBench(): void {
  const repo = $("#repo-name");
  repo.textContent = store.repo ? store.repo.split("/").filter(Boolean).pop() ?? store.repo : "No repository";
  repo.title = store.repo ? `${store.repo}\nClick to open another repository` : "Open a repository";
  document.querySelectorAll<HTMLButtonElement>("[data-level]").forEach((b) => b.classList.toggle("is-active", b.dataset.level === store.level));
  const ls = $("#layout-status");
  ls.classList.toggle("is-running", store.layout.running);
  ls.innerHTML = `<span class="pulse"></span>${store.layout.running ? `settling ${store.layout.backend}` : store.layout.backend ? `settled on ${store.layout.backend}` : "layout idle"}`;
  ls.title = store.layout.running ? `iteration ${store.layout.iteration}, energy ${store.layout.energy.toFixed(3)}` : "Click to run the layout again";
  const s = store.stats;
  $("#counts").textContent = s ? `${store.nodes.length.toLocaleString()} shown · ${s.files.toLocaleString()} files · ${s.symbols.toLocaleString()} symbols · ${s.flows} flows` : "";
  const bridge = $("#bridge");
  bridge.textContent = store.bridgePort ? `bridge :${store.bridgePort}` : "bridge starting";
  bridge.title = store.bridgePort ? `Agent bridge at http://127.0.0.1:${store.bridgePort} (token in bridge.json)` : "";
  $("#help-bridge").textContent = store.bridgePort ? `http://127.0.0.1:${store.bridgePort}/state` : "http://127.0.0.1";
}

export function renderFps(fps: number, ms: number): void {
  $("#fps").textContent = fps > 0 ? `${fps} fps · ${ms.toFixed(1)} ms` : "idle";
}

// ---- overlays ---------------------------------------------------------------

function initEmpty(): void {
  $("#open-btn").addEventListener("click", () => void actions.openRepo());
  void refreshRecent();
}

export async function refreshRecent(): Promise<void> {
  const ul = $("#recent");
  ul.innerHTML = "";
  let entries: Awaited<ReturnType<typeof api.recent>> = [];
  try { entries = await api.recent(); } catch { return; }
  for (const e of entries.slice(0, 6)) {
    const li = document.createElement("li");
    const b = document.createElement("button");
    b.dataset.testid = "recent-repo";
    b.innerHTML = `<span class="path">${esc(e.root)}</span><span class="meta">${e.files} files · ${e.flows} flows</span>`;
    b.addEventListener("click", () => void actions.openRepo(e.root));
    li.appendChild(b);
    ul.appendChild(li);
  }
}

function renderOverlays(): void {
  $("#empty").hidden = store.graphLoaded || store.scanning !== null;
  $("#scanning").hidden = store.scanning === null;
  $("#scanning-path").textContent = store.scanning ?? "";
  $("#shelf").classList.toggle("is-collapsed", !store.graphLoaded || !store.shelfOpen);
}

function initHelp(): void {
  const help = $("#help");
  $("#help-btn").addEventListener("click", () => { help.hidden = !help.hidden; });
  help.querySelector("[data-testid=help-close]")!.addEventListener("click", () => { help.hidden = true; });
  help.addEventListener("click", (e) => { if (e.target === help) help.hidden = true; });
}

export function toggleHelp(force?: boolean): void {
  const help = $("#help");
  help.hidden = force === undefined ? !help.hidden : !force;
}

// ---- utils ------------------------------------------------------------------

export function esc(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
}

function short(path: string): string {
  const [file, sym] = path.split("#");
  const base = file.split("/").pop() ?? file;
  return sym ? `${base}#${sym}` : base;
}
