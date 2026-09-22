// Agent hooks. The backend forwards `/select`, `/step`, `/view`, `/tab`, `/ui`, `/eval`… as `bridge:request`
// events; we answer with `bridge_reply`. Everything is also reachable from the
// devtools console via `window.__terrarium`.

import { api, log, on } from "./tauri";
import { store, snapshot, select, emit, setStep, setTab, stepCount, type Tab } from "./store";
import type { BrickScene, View } from "./bricks";
import type { Actions } from "./panels";
import { jumpTo } from "./panels";

interface BridgeRequest {
  id: number;
  op: string;
  payload: Record<string, unknown>;
}

export function initBridge(scene: BrickScene, actions: Actions): void {
  const handlers: Record<string, (p: Record<string, unknown>) => Promise<unknown> | unknown> = {
    state: () => ({ ...snapshot(), frame: scene.stats() }),
    select: (p) => {
      const id = Number(p.id);
      jumpTo(id);
      return { ...snapshot(), selected: store.selection === id };
    },
    step: async (p) => {
      const n = p.step == null ? stepCount() : Number(p.step);
      if (!Number.isFinite(n) || n < 0 || n > stepCount()) return { error: `step must be 0..${stepCount()}` };
      setStep(n);
      await settled(450);
      return snapshot();
    },
    view: async (p) => {
      if (typeof p.view === "string") {
        if (!["iso", "front", "top"].includes(p.view)) return { error: "view must be iso, front or top" };
        store.view = p.view as View;
        scene.setView(store.view);
      }
      if (typeof p.spin === "boolean") { store.spin = p.spin; scene.setSpin(p.spin); }
      if (p.fit) scene.fit();
      emit("view");
      await settled(700);
      return snapshot();
    },
    tab: async (p) => {
      const t = String(p.tab);
      if (!["model", "manual", "parts", "traces", "design"].includes(t)) return { error: "tab must be model, manual, parts, traces or design" };
      setTab(t as Tab);
      await settled(250);
      return snapshot();
    },
    search: async (p) => {
      const input = document.querySelector<HTMLInputElement>("#search")!;
      input.value = String(p.q ?? "");
      input.focus();
      input.dispatchEvent(new Event("input", { bubbles: true }));
      await sleep(250);
      const results = [...document.querySelectorAll<HTMLElement>("#search-results li")].map((li) => li.textContent?.trim() ?? "");
      return { query: input.value, results, ...snapshot() };
    },
    reset: () => {
      select(null);
      store.traceOnModel = false;
      emit("trace");
      setStep(stepCount());
      setTab("model");
      scene.fit();
      return snapshot();
    },
    ui: () => uiSnapshot(),
    click: async (p) => {
      const el = byTestId(String(p.testid));
      if (!el) return { error: `no element with data-testid="${p.testid}"` };
      el.click();
      await settled(450);
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
      const fn = new Function("terrarium", "store", "scene", `return (async () => { ${String(p.js).includes("return") ? String(p.js) : `return (${String(p.js)});`} })();`);
      const result = await fn(window.__terrarium, store, scene);
      return { result: safeJson(result) };
    },
    screenshot: () => ({ dataUrl: scene.snapshotDataUrl(), partial: true }),
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
    scene,
    actions,
    snapshot,
    select: (id: number | null) => (id === null ? select(null) : jumpTo(id)),
    ui: uiSnapshot,
    version: "0.2.0",
  };
}

function byTestId(id: string): HTMLElement | null {
  return document.querySelector<HTMLElement>(`[data-testid="${CSS.escape(id)}"]`);
}

function visible(el: HTMLElement): boolean {
  if (el.hidden) return false;
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
  const elements = [...document.querySelectorAll<HTMLElement>("[data-testid]")]
    .filter((el) => visible(el) && !el.matches("[data-panel], .overlay, canvas"))
    .slice(0, 400)
    .map((el) => ({
      testid: el.dataset.testid,
      tag: el.tagName.toLowerCase(),
      text: (el instanceof HTMLInputElement ? el.value : el.textContent ?? "").trim().replace(/\s+/g, " ").slice(0, 120),
      active: el.classList.contains("is-active") || el.classList.contains("is-selected") || undefined,
      disabled: (el as HTMLButtonElement).disabled || undefined,
    }));
  const toasts = [...document.querySelectorAll<HTMLElement>(".toast")].map((t) => t.textContent ?? "");
  const card = document.querySelector<HTMLElement>("#card");
  return {
    ...snapshot(),
    panels,
    overlays,
    card: card && !card.hidden ? { title: card.querySelector("[data-testid=card-title]")?.textContent, path: card.querySelector("[data-testid=card-path]")?.textContent, chips: [...card.querySelectorAll(".chip")].map((c) => c.textContent) } : null,
    toasts,
    elements,
    window: { width: window.innerWidth, height: window.innerHeight, dpr: window.devicePixelRatio },
  };
}

function safeJson(v: unknown): unknown {
  try { return JSON.parse(JSON.stringify(v ?? null)); } catch { return String(v); }
}

/** Two frames plus a short wait: long enough for a render and a camera ease or drop-in. */
async function settled(ms: number): Promise<void> {
  await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  await sleep(ms);
}

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

declare global {
  interface Window {
    __terrarium: {
      store: typeof store;
      scene: BrickScene;
      actions: Actions;
      snapshot: typeof snapshot;
      select: (id: number | null) => unknown;
      ui: typeof uiSnapshot;
      version: string;
    };
  }
}
