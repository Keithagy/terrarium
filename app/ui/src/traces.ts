// Traces: the shelf lists of traces and endpoints, and the Traces tab's lane
// diagram that follows one request from its entry point across every boundary
// to its sinks.
// One lane per package, one row per step in call order; calls are quiet tree
// connectors, boundary crossings are amber lines into another lane.

import { api, log } from "./tauri";
import { store, subscribe, select, setTab, setTrace } from "./store";
import { LANG_LABEL, type Endpoint, type Lang, type Trace } from "./types";
import { esc, toast } from "./panels";

const ROW = 42;
const PILL_W = 192;
const PILL_H = 30;
const INDENT = 18;
const LANE_GAP = 56;
const HEAD = 58;
const PAD = 28;

const $ = <T extends HTMLElement>(sel: string): T => document.querySelector(sel) as T;

export interface TraceActions {
  showOnModel(): void;
}

let actions: TraceActions;
/** Step to scroll to and ring once, e.g. where an endpoint's call lands. */
let focusStep: number | null = null;

export function initTraces(a: TraceActions): void {
  actions = a;
  subscribe("trace", () => { renderTraceList(); renderStage(); });
  subscribe("tab", renderStage);
  subscribe("selection", markSelected);
}

/** Fetch traces and endpoints for the loaded repo; called after every scan. */
export async function loadTraces(firstForRepo: boolean): Promise<void> {
  try {
    [store.traces, store.endpoints] = await Promise.all([api.traces(), api.endpoints()]);
  } catch (e) {
    log("warn", `traces failed: ${String(e)}`);
    store.traces = [];
    store.endpoints = [];
  }
  const keep = !firstForRepo && store.trace && store.traces.find((t) => t.entry === store.trace!.entry);
  store.traceOnModel = false;
  setTrace(keep || store.traces[0] || null);
  renderEndpoints();
  log("info", "traces loaded", { traces: store.traces.length, endpoints: store.endpoints.length, gaps: store.endpoints.filter((e) => e.status !== "ok").length });
}

export function showTrace(t: Trace, stepPath?: string): void {
  store.traceOnModel = false;
  focusStep = stepPath ? t.steps.findIndex((s) => s.path === stepPath) : null;
  select(null);
  setTrace(t);
  setTab("traces");
}

/** Trace from any node, not only entry points: "what happens from here on". */
export async function traceFrom(id: number): Promise<void> {
  const existing = store.traces.find((t) => t.entry === id);
  if (existing) { showTrace(existing); return; }
  try {
    showTrace(await api.trace(id));
  } catch (e) {
    toast(String(e).replace(/^Error: /, ""), "info");
  }
}

export function stepTrace(delta: number): void {
  if (!store.traces.length) return;
  const i = store.trace ? store.traces.findIndex((t) => t.entry === store.trace!.entry) : -1;
  const next = store.traces[(i + delta + store.traces.length) % store.traces.length];
  showTrace(next);
  document.querySelector(`[data-testid="trace-${next.entry}"]`)?.scrollIntoView({ block: "nearest" });
}

// ---- shelf -------------------------------------------------------------------

function langChain(langs: Lang[]): string {
  return langs.map((l) => `<span class="lang"><span class="dot" data-lang="${l}"></span>${esc(LANG_LABEL[l])}</span>`).join(`<span class="arrow">→</span>`);
}

function sinkChips(sinks: string[]): string {
  return sinks.map((s) => s.startsWith("unmatched")
    ? `<span class="chip is-gap" title="${esc(s)}">leaves repo</span>`
    : `<span class="chip is-sink">${esc(s)}</span>`).join("");
}

function renderTraceList(): void {
  const el = $("[data-tab-panel=traces]");
  el.innerHTML = "";
  if (!store.graphLoaded) return;
  if (!store.traces.length) {
    el.innerHTML = `<p class="empty-note">No request crosses a boundary yet. A trace appears when an HTTP call, a Tauri <code>invoke</code> or a queue producer pairs with its handler. The Endpoints tab lists the calls that did not pair up.</p>`;
    return;
  }
  const frag = document.createDocumentFragment();
  for (const t of store.traces) {
    const b = document.createElement("button");
    b.className = `trace-row${store.trace?.entry === t.entry ? " is-selected" : ""}`;
    b.dataset.testid = `trace-${t.entry}`;
    b.title = t.entry_path;
    b.innerHTML = `
      <span class="tr-top"><span class="name">${esc(t.name)}</span><span class="meta">${t.hops} ${t.hops === 1 ? "hop" : "hops"}</span></span>
      <span class="tr-path">${esc(t.entry_path.split("#")[0])}</span>
      <span class="tr-langs">${langChain(t.langs)}</span>
      ${t.sinks.length ? `<span class="tr-sinks">${sinkChips(t.sinks)}</span>` : ""}`;
    b.addEventListener("click", () => showTrace(t));
    frag.appendChild(b);
  }
  el.appendChild(frag);
}

const STATUS_TITLE: Record<Endpoint["status"], string> = {
  "no-handler": "Called, but nothing here serves it",
  "no-callers": "Served, but nothing here calls it",
  ok: "Paired",
};

function renderEndpoints(): void {
  const el = $("[data-tab-panel=endpoints]");
  el.innerHTML = "";
  if (!store.graphLoaded) return;
  if (!store.endpoints.length) {
    el.innerHTML = `<p class="empty-note">No HTTP routes, IPC commands or queue topics found.</p>`;
    return;
  }
  const frag = document.createDocumentFragment();
  for (const status of ["no-handler", "no-callers", "ok"] as const) {
    const members = store.endpoints.filter((e) => e.status === status);
    if (!members.length) continue;
    const title = document.createElement("div");
    title.className = `group-title${status === "ok" ? "" : " is-gap"}`;
    title.textContent = `${STATUS_TITLE[status]} (${members.length})`;
    frag.appendChild(title);
    for (const e of members) {
      const b = document.createElement("button");
      b.className = `endpoint-row is-${status}`;
      b.dataset.testid = `endpoint-${e.key}`;
      const side = (refs: typeof e.callers, none: string) =>
        refs.length
          ? refs.slice(0, 3).map((r) => `<span class="ep-ref" title="${esc(r.path)}"><span class="dot" data-lang="${r.lang}"></span>${esc(r.name)}</span>`).join("") + (refs.length > 3 ? `<span class="meta">+${refs.length - 3}</span>` : "")
          : `<span class="ep-none">${none}</span>`;
      b.innerHTML = `
        <span class="ep-key">${esc(e.key)}</span>
        <span class="ep-sides">${side(e.callers, "no caller")}<span class="arrow">→</span>${side(e.handlers, "no handler")}</span>`;
      b.addEventListener("click", () => openEndpoint(e));
      frag.appendChild(b);
    }
  }
  el.appendChild(frag);
}

function openEndpoint(e: Endpoint): void {
  // Show the longest trace that crosses this endpoint, ringing the step where it lands.
  const hit = store.traces.find((t) => t.steps.some((s) => s.via === "flow" && s.label === e.key));
  if (hit) {
    const landing = hit.steps.find((s) => s.via === "flow" && s.label === e.key)!;
    showTrace(hit, landing.path);
    return;
  }
  // A gap has nothing to trace across; open the side that exists.
  const side = e.callers[0] ?? e.handlers[0];
  if (side) select(side.id);
}

// ---- stage -------------------------------------------------------------------

function listLangs(langs: Lang[]): string {
  const names = langs.map((l) => LANG_LABEL[l]);
  return names.length <= 1 ? names.join("") : `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
}

function summary(t: Trace): string {
  const rest = t.sinks.filter((s) => !s.startsWith("unmatched"));
  const gaps = t.sinks.length - rest.length;
  let out = `Crosses ${t.hops} ${t.hops === 1 ? "boundary" : "boundaries"} through ${listLangs(t.langs)}.`;
  out += rest.length ? ` Data comes to rest in ${rest.join(", ")}.` : " Nothing on this path touches a database, file or queue.";
  if (gaps) out += ` ${gaps} ${gaps === 1 ? "call leaves" : "calls leave"} the repository.`;
  if (t.truncated) out += " Cut short at 300 steps.";
  return out;
}

function renderStage(): void {
  const el = $("#trace-view");
  el.hidden = store.tab !== "traces" || !store.graphLoaded;
  if (el.hidden) return;
  const t = store.trace;
  if (!t) {
    el.innerHTML = `<div class="trace-empty"><h2>Nothing to trace yet</h2><p>${store.traces.length ? "Pick a trace from the shelf." : "No request in this repository crosses a language or process boundary that Terrarium can pair up. The model and its manual still show how files rest on each other."}</p>${store.traces.length ? "" : `<button class="ghost" data-testid="trace-empty-model">Open the model</button>`}</div>`;
    el.querySelector("[data-testid=trace-empty-model]")?.addEventListener("click", () => setTab("model"));
    return;
  }
  const steps = t.steps;
  const laneIdx = new Map(t.lanes.map((l, i) => [l, i]));
  // Indent only within a lane: entering another lane starts at its left edge.
  const local = new Array<number>(steps.length).fill(0);
  const maxLocal = new Array<number>(t.lanes.length).fill(0);
  const laneLangs = t.lanes.map(() => new Map<Lang, number>());
  steps.forEach((s, i) => {
    const p = s.parent;
    local[i] = p !== undefined && steps[p].lane === s.lane ? local[p] + 1 : 0;
    const li = laneIdx.get(s.lane)!;
    maxLocal[li] = Math.max(maxLocal[li], local[i]);
    laneLangs[li].set(s.lang, (laneLangs[li].get(s.lang) ?? 0) + 1);
  });
  const laneW = maxLocal.map((m) => PILL_W + m * INDENT);
  const laneX: number[] = [];
  let acc = PAD;
  for (const w of laneW) { laneX.push(acc); acc += w + LANE_GAP; }
  const width = acc - LANE_GAP + PAD;
  const height = HEAD + steps.length * ROW + PAD;
  const xs = steps.map((s, i) => laneX[laneIdx.get(s.lane)!] + local[i] * INDENT);
  const ys = steps.map((_, i) => HEAD + i * ROW);

  const lanes = t.lanes.map((name, li) => {
    const lang = [...laneLangs[li].entries()].sort((a, b) => b[1] - a[1])[0]?.[0] ?? "other";
    return `<div class="lane" data-lang="${lang}" style="left:${laneX[li] - 14}px;width:${laneW[li] + 28}px;height:${height}px"></div>
      <div class="lane-head" data-testid="lane-${esc(name || "root")}" style="left:${laneX[li]}px;width:${laneW[li]}px"><span class="dot" data-lang="${lang}"></span><span class="lane-name">${esc(name || "root")}</span><span class="lane-lang">${esc(LANG_LABEL[lang])}</span></div>`;
  }).join("");

  let wires = "";
  let labels = "";
  steps.forEach((s, i) => {
    const p = s.parent;
    if (p === undefined) return;
    const px = xs[p] + 11;
    const py = ys[p] + PILL_H;
    const cy = ys[i] + PILL_H / 2;
    if (s.via === "flow") {
      const rightward = xs[i] >= px;
      const toX = rightward ? xs[i] : xs[i] + PILL_W;
      wires += `<path class="w-flow" d="M${px} ${py} V${cy} H${toX}"/>`;
      labels += `<span class="flow-label" style="${rightward ? `right:${width - toX + 10}px` : `left:${toX + 10}px`};top:${cy - 20}px">${esc(s.label ?? "flow")}</span>`;
    } else {
      wires += `<path class="w-call" d="M${px} ${py} V${cy} H${xs[i]}"/>`;
    }
  });

  const pills = steps.map((s, i) => {
    const cls = ["step", s.via === "flow" ? "is-landing" : "", s.repeat ? "is-repeat" : "", i === 0 ? "is-entry" : ""].filter(Boolean).join(" ");
    const title = `${s.path}${s.line ? `:${s.line}` : ""}${s.repeat ? "\nCalled again here; its calls are shown the first time" : ""}`;
    return `<button class="${cls}" data-testid="step-${i}" data-step="${i}" data-node="${s.id}" style="left:${xs[i]}px;top:${ys[i]}px;width:${PILL_W}px" title="${esc(title)}"><span class="dot" data-lang="${s.lang}"></span><span class="step-name">${esc(s.name)}</span>${s.repeat ? `<span class="step-note">again</span>` : ""}${s.sinks?.length ? sinkChips(s.sinks) : ""}</button>`;
  }).join("");

  el.innerHTML = `
    <header class="trace-head">
      <div class="trace-title"><h2 data-testid="trace-title">${esc(t.name)}</h2><span class="trace-path">${esc(t.entry_path.split("#")[0])}</span></div>
      <p class="trace-summary" data-testid="trace-summary">${esc(summary(t))}</p>
      <div class="trace-actions">
        <button class="ghost" data-testid="trace-show-model" title="Light up the buildings this trace passes through">Show on model</button>
        <button class="ghost" data-testid="trace-open" title="Open the entry point in your editor">Open in editor</button>
      </div>
    </header>
    <div class="trace-scroll" data-testid="trace-scroll">
      <div class="trace-canvas" style="width:${width}px;height:${height}px">
        ${lanes}
        <svg class="trace-wires" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" aria-hidden="true">${wires}</svg>
        ${labels}
        ${pills}
      </div>
    </div>`;
  el.querySelector("[data-testid=trace-show-model]")!.addEventListener("click", () => actions.showOnModel());
  el.querySelector("[data-testid=trace-open]")!.addEventListener("click", () => {
    const s = steps[0];
    void api.openPath(s.path.split("#")[0], s.line).catch((e) => toast(`Cannot open: ${String(e)}`, "error"));
  });
  el.querySelectorAll<HTMLButtonElement>("[data-step]").forEach((b) =>
    b.addEventListener("click", () => select(Number(b.dataset.node))),
  );
  markSelected();
  if (focusStep !== null && focusStep >= 0) {
    const b = el.querySelector<HTMLElement>(`[data-step="${focusStep}"]`);
    b?.classList.add("is-focus");
    b?.scrollIntoView({ block: "center", inline: "center" });
  }
  focusStep = null;
}

function markSelected(): void {
  const el = $("#trace-view");
  el.classList.toggle("has-card", store.selection !== null);
  el.querySelectorAll<HTMLElement>("[data-node]").forEach((b) => b.classList.toggle("is-selected", Number(b.dataset.node) === store.selection));
}
