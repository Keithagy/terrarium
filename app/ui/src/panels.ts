// DOM panels: header chips, shelf (search, sub-builds, legend; traces and
// endpoints live in traces.ts), specimen card, overlays and toasts.

import { api, log } from "./tauri";
import { store, subscribe, emit, select, setTab, setStep, stepCount, buildingForNode, buildingAt } from "./store";
import { LANG_LABEL, type Lang, type NodeDetail } from "./types";
import { traceFrom } from "./traces";

const $ = <T extends HTMLElement>(sel: string): T => document.querySelector(sel) as T;

export interface Actions {
  openRepo(path?: string): Promise<void>;
  /** Point the camera at a building (or a district's buildings). */
  focusBuilding(index: number): void;
  focusDistrict(index: number | null): void;
  showTraceOnModel(): void;
}

let actions: Actions;

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
  initHeader();
  initEmpty();
  initHelp();
  subscribe("build", () => { renderChips(); renderSubBuilds(); renderLegend(); });
  subscribe("selection", () => { void renderCard(); renderSubBuilds(); });
  subscribe("tab", () => { void renderCard(); });
  subscribe("trace", renderLegend);
  subscribe("repo", () => { renderChips(); renderOverlays(); });
  subscribe("ui", () => { renderChips(); renderOverlays(); renderBridge(); });
  renderOverlays();
}

// ---- header -------------------------------------------------------------------

function initHeader(): void {
  $("#repo-name").addEventListener("click", () => void actions.openRepo());
}

function renderChips(): void {
  const repo = $("#repo-name");
  const b = store.build;
  repo.textContent = b ? b.design.title : store.repo ? store.repo.split("/").filter(Boolean).pop() ?? store.repo : "No repository";
  repo.title = store.repo ? `${store.repo}\nClick to open another repository` : "Open a repository";
  const el = $("#chips");
  el.innerHTML = "";
  if (!b) return;
  const c = b.check;
  const chip = (text: string, testid: string, title: string, cls = "") => {
    const s = document.createElement("span");
    s.className = `hchip ${cls}`;
    s.dataset.testid = testid;
    s.title = title;
    s.innerHTML = text;
    el.appendChild(s);
  };
  chip(`<b>${c.pieces.toLocaleString()}</b> pieces`, "chip-pieces", "Bricks: one per function, type or constant (large files share bricks)");
  chip(`<b>${c.steps}</b> steps`, "chip-steps", "Manual steps, in an order where every file only rests on files built before it");
  chip(`<b>${c.sub_builds}</b> sub-builds`, "chip-sub-builds", "Packages, built as districts");
  chip(`<b>${b.model.studs[0]}×${b.model.studs[1]}</b> studs`, "chip-studs", "Baseplate size");
  chip(`<b>${c.joints.toLocaleString()}</b> joints`, "chip-joints", "Imports and calls between files: what holds the model together");
  if (c.ok) chip("holds together", "chip-check", "Every file is placed after everything it rests on", "is-ok");
  else chip(`${c.weak.length} weak ${c.weak.length === 1 ? "joint" : "joints"}`, "chip-check", "See the Design tab", "is-weak");
  const who = b.design.source === "engine" ? "engine design" : `designed by ${b.design.model ?? b.design.source}`;
  chip(esc(who), "chip-source", b.design.source === "engine" ? "The engine grouped and captioned the steps. Design with Claude for a manual written by agents that read the code." : `Claude agents wrote the sub-build names, steps and captions; the engine checked every joint${b.stale ? `\n${b.stale}` : ""}`, b.design.source === "engine" ? "is-quiet" : "is-claude");
  const btn = $<HTMLButtonElement>("#design-btn");
  btn.disabled = store.design.running || !store.graphLoaded;
  btn.textContent = store.design.running ? `Designing… ${store.design.done}/${store.design.agents}` : b.design.source === "engine" ? "Design with Claude" : "Redesign with Claude";
}

// ---- shelf --------------------------------------------------------------------

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
    jumpTo(h.id);
  };
  let timer = 0;
  input.addEventListener("input", () => {
    store.search = input.value;
    clearTimeout(timer);
    if (!input.value.trim()) { hits = []; render(); return; }
    timer = window.setTimeout(async () => {
      try {
        // The model has files and symbols; packages are districts on the shelf.
        hits = (await api.search(input.value, 20)).filter((h) => h.kind === "file" || h.kind === "symbol").slice(0, 12);
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
    tab.addEventListener("click", () => setShelfTab(tab.dataset.tab as typeof store.shelfTab));
  });
}

export function setShelfTab(t: typeof store.shelfTab): void {
  document.querySelectorAll(".tab").forEach((x) => x.classList.toggle("is-active", (x as HTMLElement).dataset.tab === t));
  document.querySelectorAll<HTMLElement>(".tab-panel").forEach((p) => p.classList.toggle("is-active", p.dataset.tabPanel === t));
  store.shelfTab = t;
  emit("ui");
}

/** Select a file or symbol and bring its building into view, finishing the model if the step hides it. */
export function jumpTo(id: number): void {
  const bi = buildingForNode(id);
  if (bi === null) {
    toast("That is not part of the model (packages are the districts on the shelf)", "info");
    return;
  }
  if (store.tab !== "model" && store.tab !== "manual") setTab("model");
  const b = buildingAt(bi)!;
  if (b.step >= store.step) setStep(stepCount());
  select(id);
  actions.focusBuilding(bi);
}

const openDistricts = new Set<string>();

function renderSubBuilds(): void {
  const el = $("[data-tab-panel=sub-builds]");
  el.innerHTML = "";
  const b = store.build;
  if (!b) return;
  const selB = store.selection !== null ? buildingForNode(store.selection) : null;
  const frag = document.createDocumentFragment();
  b.model.districts.forEach((d, di) => {
    const sb = b.design.sub_builds.find((s) => s.id === d.sub_build);
    const steps = b.design.steps.map((s, i) => [s, i] as const).filter(([s]) => s.sub_build === d.sub_build).map(([, i]) => i + 1);
    const buildings = b.model.buildings.map((x, i) => [x, i] as const).filter(([x]) => x.district === di);
    const open = openDistricts.has(d.sub_build) || buildings.some(([, i]) => i === selB);
    const head = document.createElement("button");
    head.className = `district-row${open ? " is-open" : ""}`;
    head.dataset.testid = `district-${d.sub_build}`;
    head.innerHTML = `
      <span class="dr-top"><span class="dot" data-lang="${d.lang}"></span><span class="name">${esc(sb?.name ?? d.name)}</span><span class="meta">${buildings.length} ${buildings.length === 1 ? "file" : "files"}</span></span>
      ${sb?.blurb ? `<span class="dr-blurb">${prose(sb.blurb)}</span>` : ""}
      <span class="dr-steps">${steps.length ? `steps ${steps[0]}${steps.length > 1 ? `–${steps[steps.length - 1]}` : ""}` : ""}${sb && sb.name !== sb.package ? ` · ${esc(sb.package)}` : ""}</span>`;
    head.addEventListener("click", () => {
      if (openDistricts.has(d.sub_build)) openDistricts.delete(d.sub_build); else openDistricts.add(d.sub_build);
      actions.focusDistrict(di);
      renderSubBuilds();
    });
    frag.appendChild(head);
    if (!open) return;
    for (const [x, i] of buildings) {
      const r = document.createElement("button");
      r.className = `row is-nested${i === selB ? " is-selected" : ""}${x.step >= store.step ? " is-dim" : ""}`;
      r.dataset.testid = `building-${x.id}`;
      r.title = `${x.path}\nAdded in step ${x.step + 1}`;
      r.innerHTML = `<span class="dot" data-lang="${x.lang}"></span><span class="name">${esc(x.name)}</span>${x.lamp ? `<span class="lamp-dot" title="Starts a trace across a boundary"></span>` : ""}<span class="meta">step ${x.step + 1}</span>`;
      r.addEventListener("click", () => jumpTo(x.id));
      frag.appendChild(r);
    }
  });
  el.appendChild(frag);
}

function renderLegend(): void {
  const el = $("#shelf .legend");
  el.innerHTML = "";
  const b = store.build;
  if (!b) return;
  const counts = new Map<Lang, number>();
  for (const x of b.model.buildings) counts.set(x.lang, (counts.get(x.lang) ?? 0) + 1);
  for (const [lang, n] of [...counts.entries()].sort((a, c) => c[1] - a[1])) {
    const s = document.createElement("span");
    s.className = "legend-item";
    s.dataset.testid = `lang-${lang}`;
    s.innerHTML = `<span class="brick-swatch" data-lang="${lang}"></span>${LANG_LABEL[lang]} <span class="meta">${n}</span>`;
    el.appendChild(s);
  }
  const sep = document.createElement("span");
  sep.className = "sep";
  el.appendChild(sep);
  const key = document.createElement("span");
  key.className = "legend-item";
  key.innerHTML = `<span class="edge-swatch" data-edge="flow"></span>bridge: data crosses languages`;
  el.appendChild(key);
  const lamp = document.createElement("span");
  lamp.className = "legend-item";
  lamp.innerHTML = `<span class="lamp-dot"></span>lamp: starts a trace`;
  el.appendChild(lamp);
  if (store.trace && store.traceOnModel) {
    const t = document.createElement("button");
    t.dataset.testid = "clear-trace";
    t.title = "Stop highlighting this trace";
    t.innerHTML = `<span class="chip is-boundary">trace ${esc(store.trace.name)} ×</span>`;
    t.addEventListener("click", () => { store.traceOnModel = false; emit("trace"); });
    el.appendChild(t);
  }
}

// ---- specimen card ----------------------------------------------------------

async function renderCard(): Promise<void> {
  const card = $("#card");
  const id = store.selection;
  if (id === null || store.tab === "manual" || store.tab === "design" || store.tab === "parts") { card.hidden = true; card.innerHTML = ""; document.body.classList.remove("has-card"); return; }
  let d: NodeDetail;
  try { d = await api.getNode(id); } catch (e) { log("warn", `node detail failed: ${String(e)}`); return; }
  if (store.selection !== id) return;
  const n = d.node;
  const tags = n.tags ?? [];
  const bi = buildingForNode(id);
  const bld = buildingAt(bi);
  const step = bld ? store.build!.design.steps[bld.step] : null;
  const kindLine = n.kind === "symbol" ? `${n.symbol_kind ?? "symbol"} in ${d.file ?? ""}` : n.kind === "file" ? `file in ${d.package ?? ""}` : "package";
  const ins = d.neighbours.filter((x) => x.direction === "in" && x.kind !== "package");
  const outs = d.neighbours.filter((x) => x.direction === "out" && x.kind !== "package");
  const nb = (list: typeof ins, title: string) => list.length ? `<div class="card-section"><h3>${title} (${list.length})</h3>${list.slice(0, 40).map((x) => `<button class="nb${x.edge === "flow" ? " is-flow" : ""}" data-jump="${x.id}" data-testid="nb-${x.id}"><span class="arrow">${x.direction === "in" ? "←" : "→"}</span><span class="nb-name" title="${esc(x.path)}">${esc(x.kind === "symbol" ? x.name : short(x.path))}</span><span class="nb-label">${esc(x.edge === "calls" ? "calls" : x.edge === "imports" ? "imports" : x.label ?? "flow")}</span></button>`).join("")}${list.length > 40 ? `<div class="group-title">and ${list.length - 40} more</div>` : ""}</div>` : "";
  card.innerHTML = `
    <button class="card-close" data-testid="card-close" title="Close (Esc)">×</button>
    <div class="card-kind"><span class="dot" data-lang="${n.lang}"></span>${esc(LANG_LABEL[n.lang])} · ${esc(kindLine)}</div>
    <h2 data-testid="card-title">${esc(n.name)}</h2>
    <div class="card-path" data-testid="card-path">${esc(n.path)}${n.span ? `:${n.span[0]}` : ""}</div>
    ${bld && step ? `<button class="card-step" data-testid="card-step" title="Show the manual at this step"><span class="step-no">${bld.step + 1}</span><span><b>${esc(step.title)}</b><br><span class="meta">${esc(store.build!.design.sub_builds.find((s) => s.id === step.sub_build)?.name ?? step.sub_build)}</span></span></button>` : ""}
    ${tags.length ? `<div class="chips">${tags.filter((t) => t !== "external").map((t) => `<span class="chip${t.includes(":") || ["db", "fs", "queue", "http-server", "http-client", "ipc-server", "ipc-client"].includes(t) ? " is-boundary" : ""}">${esc(t)}</span>`).join("")}</div>` : ""}
    <div class="card-actions">
      <button class="ghost" data-testid="open-file">Open in editor</button>
      ${n.kind !== "package" ? `<button class="ghost" data-testid="trace-from" title="Follow the calls from here across every boundary">Trace from here</button>` : ""}
    </div>
    <div class="card-facts">
      <span>Lines</span><b>${n.loc.toLocaleString()}</b>
      <span>Rests on</span><b>${outs.filter((x) => x.edge !== "flow").length}</b>
      <span>Holds up</span><b>${ins.filter((x) => x.edge !== "flow").length}</b>
      ${d.children.length ? `<span>Bricks</span><b>${d.children.length}</b>` : ""}
    </div>
    ${nb(outs.filter((x) => x.edge !== "flow"), "Rests on")}
    ${nb(outs.filter((x) => x.edge === "flow"), "Bridges to")}
    ${nb(ins.filter((x) => x.edge !== "flow"), "Holds up")}
    ${nb(ins.filter((x) => x.edge === "flow"), "Bridged from")}
  `;
  card.hidden = false;
  document.body.classList.add("has-card");
  card.querySelector("[data-testid=card-close]")!.addEventListener("click", () => select(null));
  card.querySelector("[data-testid=open-file]")?.addEventListener("click", () => void api.openPath(n.path.split("#")[0], n.span?.[0]).catch((e) => toast(`Cannot open: ${String(e)}`, "error")));
  card.querySelector("[data-testid=trace-from]")?.addEventListener("click", () => void traceFrom(n.id));
  card.querySelector("[data-testid=card-step]")?.addEventListener("click", () => { if (bld) { setStep(bld.step + 1); setTab("manual"); } });
  card.querySelectorAll<HTMLButtonElement>("[data-jump]").forEach((b) => b.addEventListener("click", () => jumpTo(Number(b.dataset.jump))));
}

// ---- overlays -----------------------------------------------------------------

function renderBridge(): void {
  const bridge = $("#bridge");
  bridge.textContent = store.bridgePort ? `bridge :${store.bridgePort}` : "bridge starting";
  bridge.title = store.bridgePort ? `Agent bridge at http://127.0.0.1:${store.bridgePort} (token in bridge.json)` : "";
  $("#help-bridge").textContent = store.bridgePort ? `http://127.0.0.1:${store.bridgePort}/state` : "http://127.0.0.1";
}

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
  document.body.classList.toggle("is-empty", !store.graphLoaded);
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

/** Escape, then render `inline code` the way the design agents write it. */
export function prose(s: string): string {
  return esc(s).replace(/`([^`]+)`/g, "<code>$1</code>");
}

export function short(path: string): string {
  const [file, sym] = path.split("#");
  const base = file.split("/").pop() ?? file;
  return sym ? `${base}#${sym}` : base;
}
