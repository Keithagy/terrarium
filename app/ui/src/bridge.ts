// Agent hooks. The backend forwards `/select`, `/level`, `/journey`, `/ui`, `/eval`…
// as `bridge:request` events; we answer with `bridge_reply`. Everything is also
// reachable from the devtools console via `window.__terrarium`.

import { api, log, on } from "./tauri";
import { store, snapshot, select, setLevel, setJourney, setJourneyStep, setView, elementById, shownContainers, currentProjection, type Level } from "./store";
import type { Actions } from "./panels";
import { jumpTo, playJourney } from "./panels";
import { exportSvg, fit } from "./atlas";
import { sequenceSnapshot } from "./sequence";

interface BridgeRequest {
  id: number;
  op: string;
  payload: Record<string, unknown>;
}

/** An element by id, name, package or file path. */
function resolveElement(s: string): string | null {
  const a = store.atlas;
  if (!a) return null;
  if (elementById(s)) return s;
  const q = s.trim().toLowerCase();
  if (q === a.system.name.toLowerCase() || q === "system") return "s";
  for (const c of a.containers) {
    if (c.name.toLowerCase() === q || c.package.toLowerCase() === q) return c.id;
    for (const k of c.components) if (k.name.toLowerCase() === q || `${c.name}/${k.name}`.toLowerCase() === q) return k.id;
  }
  for (const p of a.people) if (p.name.toLowerCase() === q) return p.id;
  for (const x of a.externals) if (x.name.toLowerCase() === q) return x.id;
  for (const c of a.containers) for (const k of c.components) if (k.files.includes(s)) return `path:${s}`;
  return null;
}

export function initBridge(actions: Actions): void {
  const handlers: Record<string, (p: Record<string, unknown>) => Promise<unknown> | unknown> = {
    state: () => snapshot(),
    select: async (p) => {
      const q = String(p.element ?? p.node ?? "");
      const id = resolveElement(q);
      if (!id) return { error: `no element matches \`${q}\`` };
      jumpTo(id);
      await settled(350);
      return { ...snapshot(), selected: store.selection };
    },
    level: async (p) => {
      const level = String(p.level ?? "");
      if (!["context", "containers", "components", "code"].includes(level)) return { error: "level must be context, containers, components or code" };
      let focus: string | null = null;
      if (level === "components" || level === "code") {
        const q = p.focus ? String(p.focus) : "";
        const id = q ? resolveElement(q) : store.selection ?? store.focus;
        if (!id) return { error: `${level} needs a focus: a ${level === "code" ? "component" : "container"} id or name` };
        const e = elementById(id.startsWith("path:") ? "" : id);
        if (level === "components") focus = e?.kind === "container" ? e.container.id : e?.kind === "component" ? e.container.id : null;
        else focus = e?.kind === "component" ? e.component.id : null;
        if (!focus) return { error: `\`${q || id}\` is not a ${level === "code" ? "component" : "container"}` };
      }
      setLevel(level as Level, focus);
      await settled(350);
      return snapshot();
    },
    journey: async (p) => {
      if (p.journey == null) { setJourney(null); await settled(100); return snapshot(); }
      const q = String(p.journey).toLowerCase();
      const j = store.atlas?.journeys.find((x) => x.id === q || x.name.toLowerCase() === q || x.entry.toLowerCase() === q);
      if (!j) return { error: `no journey matches \`${p.journey}\`` };
      const view = p.view === "sequence" || p.view === "map" ? p.view : undefined;
      playJourney(j, p.step == null ? 0 : Number(p.step), view);
      if (view) setView(view);
      if (p.step != null) setJourneyStep(Number(p.step));
      await settled(350);
      const proj = currentProjection();
      return { ...snapshot(), steps: (proj?.messages ?? []).map((m, i) => ({ n: i + 1, from: m.from, to: m.to, kind: m.kind, label: m.label, source: m.source, caption: m.caption })), messages: j.messages.length };
    },
    search: async (p) => {
      const input = document.querySelector<HTMLInputElement>("#search")!;
      input.value = String(p.q ?? "");
      input.focus();
      input.dispatchEvent(new Event("input", { bubbles: true }));
      await sleep(300);
      const results = [...document.querySelectorAll<HTMLElement>("#search-results li")].map((li) => li.textContent?.trim() ?? "");
      return { query: input.value, results, ...snapshot() };
    },
    reset: async () => {
      select(null);
      setJourney(null);
      store.notesOpen = false;
      setLevel("containers");
      fit(false);
      await settled(200);
      return snapshot();
    },
    ui: () => uiSnapshot(),
    click: async (p) => {
      const el = byTestId(String(p.testid));
      if (!el) return { error: `no element with data-testid="${p.testid}"` };
      el.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
      await settled(400);
      return { clicked: p.testid, ...snapshot() };
    },
    type: (p) => {
      const el = byTestId(String(p.testid));
      if (!el) return { error: `no element with data-testid="${p.testid}"` };
      if (!(el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement)) return { error: `${p.testid} is not an input` };
      el.focus();
      el.value = String(p.text ?? "");
      el.dispatchEvent(new Event("input", { bubbles: true }));
      return { typed: el.value, testid: p.testid };
    },
    eval: async (p) => {
      // eslint-disable-next-line no-new-func
      const fn = new Function("terrarium", "store", `return (async () => { ${String(p.js).includes("return") ? String(p.js) : `return (${String(p.js)});`} })();`);
      const result = await fn(window.__terrarium, store);
      return { result: safeJson(result) };
    },
    screenshot: () => ({ error: "no canvas fallback: the native snapshot is the screenshot" }),
    svg: () => ({ svg: exportSvg() }),
  };

  void on<BridgeRequest>("bridge:request", async (req) => {
    const t0 = performance.now();
    let result: unknown;
    try {
      const h = handlers[req.op];
      result = h ? await h(req.payload ?? {}) : { error: `unknown op ${req.op}` };
    } catch (e) {
      result = { error: String(e instanceof Error ? e.message : e) };
      log("warn", `bridge op ${req.op} failed`, { error: String(e) });
    }
    log("debug", `bridge ${req.op}`, { ms: Math.round(performance.now() - t0) });
    try { await api.bridgeReply(req.id, result); } catch (e) { log("error", `bridge reply failed: ${String(e)}`); }
  });

  window.__terrarium = {
    store,
    actions,
    snapshot,
    select: (id: string | null) => (id === null ? select(null) : jumpTo(id)),
    ui: uiSnapshot,
    svg: exportSvg,
    containers: () => shownContainers().map((c) => c.id),
    version: "0.3.0",
  };
}

function byTestId(id: string): HTMLElement | null {
  return document.querySelector<HTMLElement>(`[data-testid="${CSS.escape(id)}"]`);
}

function visible(el: Element): boolean {
  if ((el as HTMLElement).hidden) return false;
  const r = el.getBoundingClientRect();
  if (r.width === 0 && r.height === 0) return false;
  const cs = getComputedStyle(el);
  return cs.display !== "none" && cs.visibility !== "hidden" && cs.opacity !== "0";
}

/** What an agent needs to "see" the UI without a screenshot. */
export function uiSnapshot(): Record<string, unknown> {
  const panels: Record<string, unknown> = {};
  for (const p of document.querySelectorAll<HTMLElement>("[data-panel]")) {
    panels[p.dataset.panel!] = { visible: visible(p) && !p.classList.contains("is-collapsed"), testid: p.dataset.testid };
  }
  const overlays: Record<string, boolean> = {};
  for (const o of document.querySelectorAll<HTMLElement>(".overlay")) overlays[o.dataset.testid ?? o.id] = visible(o);
  const elements = [...document.querySelectorAll<Element>("[data-testid]")]
    .filter((el) => visible(el) && !el.matches("[data-panel], .overlay"))
    .slice(0, 500)
    .map((el) => ({
      testid: (el as HTMLElement).dataset?.testid ?? el.getAttribute("data-testid"),
      tag: el.tagName.toLowerCase(),
      text: (el instanceof HTMLInputElement ? el.value : el.textContent ?? "").trim().replace(/\s+/g, " ").slice(0, 120),
      active: el.classList.contains("is-active") || el.classList.contains("is-selected") || el.classList.contains("is-current") || undefined,
      state: (el as HTMLElement).dataset?.state || undefined,
      disabled: (el as HTMLButtonElement).disabled || undefined,
    }));
  const toasts = [...document.querySelectorAll<HTMLElement>(".toast")].map((t) => t.textContent ?? "");
  const card = document.querySelector<HTMLElement>("#card");
  const diagram = document.getElementById("diagram");
  return {
    ...snapshot(),
    panels,
    overlays,
    card: card && !card.hidden ? { title: card.querySelector("[data-testid=card-title]")?.textContent?.trim(), chips: [...card.querySelectorAll(".chip")].map((c) => c.textContent) } : null,
    diagram: diagram ? { nodes: [...diagram.querySelectorAll<SVGGElement>("g.node")].map((g) => ({ id: g.dataset.id, title: g.querySelector(".c4-title")?.textContent, state: g.dataset.state || undefined, dim: g.classList.contains("is-dim") || undefined })), edges: [...diagram.querySelectorAll<SVGGElement>("g.edge")].map((g) => ({ from: g.dataset.from, to: g.dataset.to, source: [...g.classList].find((c) => c.startsWith("is-") && ["is-code", "is-survey", "is-claimed"].includes(c))?.slice(3), journey: g.classList.contains("is-journey") || undefined })) } : null,
    sequence: sequenceSnapshot() ?? undefined,
    notes: store.notesOpen ? store.discovery.notes.slice(-30).map((n) => n.text) : undefined,
    toasts,
    elements,
    window: { width: window.innerWidth, height: window.innerHeight, dpr: window.devicePixelRatio },
  };
}

function safeJson(v: unknown): unknown {
  try { return JSON.parse(JSON.stringify(v ?? null)); } catch { return String(v); }
}

/** Two frames plus a short wait: long enough for a render and a level ease. An occluded
 * window gets no animation frames, so a timer stands in for them rather than hang the caller. */
async function settled(ms: number): Promise<void> {
  await Promise.race([new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))), sleep(150)]);
  await sleep(ms);
}

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

declare global {
  interface Window {
    __terrarium: {
      store: typeof store;
      actions: Actions;
      snapshot: typeof snapshot;
      select: (id: string | null) => unknown;
      ui: typeof uiSnapshot;
      svg: () => string;
      containers: () => string[];
      version: string;
    };
  }
}
