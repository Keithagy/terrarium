// Single source of truth for the UI. Panels and the brick scene subscribe to it;
// the bridge reads it to answer `/state`.

import type { Build, Building, Endpoint, Stats, Trace } from "./types";

export type Tab = "model" | "manual" | "parts" | "traces" | "design";
export type ShelfTab = "sub-builds" | "traces" | "endpoints";
export type View = "iso" | "front" | "top";

export interface DesignRunState {
  running: boolean;
  model: string;
  agents: number;
  done: number;
  cost_usd: number;
  log: { role: string; ok: boolean; cost_usd: number }[];
  error: string | null;
}

export interface Store {
  repo: string | null;
  stats: Stats | null;
  build: Build | null;
  /** Manual steps shown: 0 is the empty baseplate, `steps` is the finished model. */
  step: number;
  playing: boolean;
  speed: number;
  tab: Tab;
  shelfTab: ShelfTab;
  view: View;
  spin: boolean;
  /** Selected graph node (a file or a symbol). */
  selection: number | null;
  hover: number | null;
  /** Building index by file node id, and brick index by symbol node id. */
  buildingOf: Map<number, number>;
  brickOf: Map<number, number>;
  traces: Trace[];
  endpoints: Endpoint[];
  /** The trace on the Traces tab. */
  trace: Trace | null;
  /** Light the trace up on the model, dimming everything else. */
  traceOnModel: boolean;
  design: DesignRunState;
  search: string;
  shelfOpen: boolean;
  bridgePort: number | null;
  scanning: string | null;
  graphLoaded: boolean;
}

export const store: Store = {
  repo: null,
  stats: null,
  build: null,
  step: 0,
  playing: false,
  speed: 1,
  tab: "model",
  shelfTab: "sub-builds",
  view: "iso",
  spin: false,
  selection: null,
  hover: null,
  buildingOf: new Map(),
  brickOf: new Map(),
  traces: [],
  endpoints: [],
  trace: null,
  traceOnModel: false,
  design: { running: false, model: "", agents: 0, done: 0, cost_usd: 0, log: [], error: null },
  search: "",
  shelfOpen: true,
  bridgePort: null,
  scanning: null,
  graphLoaded: false,
};

type Topic = "build" | "step" | "selection" | "hover" | "tab" | "view" | "trace" | "design" | "repo" | "ui";
const listeners = new Map<Topic, Set<() => void>>();

export function subscribe(topic: Topic, fn: () => void): () => void {
  if (!listeners.has(topic)) listeners.set(topic, new Set());
  listeners.get(topic)!.add(fn);
  return () => listeners.get(topic)!.delete(fn);
}

export function emit(topic: Topic): void {
  listeners.get(topic)?.forEach((fn) => fn());
}

export function stepCount(): number {
  return store.build?.design.steps.length ?? 0;
}

export function setBuild(b: Build): void {
  store.build = b;
  store.buildingOf = new Map(b.model.buildings.map((x, i) => [x.id, i]));
  store.brickOf = new Map();
  b.model.bricks.forEach((br, i) => br.nodes.forEach((n) => store.brickOf.set(n, i)));
  store.step = b.design.steps.length;
  store.playing = false;
  store.graphLoaded = true;
  if (store.selection !== null && buildingForNode(store.selection) === null) store.selection = null;
  emit("build");
  emit("step");
  emit("selection");
}

/** The building a node lives in: the file itself, or the file of a symbol. */
export function buildingForNode(id: number): number | null {
  const direct = store.buildingOf.get(id);
  if (direct !== undefined) return direct;
  const brick = store.brickOf.get(id);
  return brick === undefined ? null : store.build!.model.bricks[brick].building;
}

export function buildingAt(i: number | null): Building | null {
  return i === null ? null : store.build?.model.buildings[i] ?? null;
}

export function setStep(n: number): void {
  const clamped = Math.max(0, Math.min(stepCount(), Math.round(n)));
  if (clamped === store.step) return;
  store.step = clamped;
  emit("step");
}

export function setTab(t: Tab): void {
  if (store.tab === t) return;
  store.tab = t;
  emit("tab");
}

export function select(id: number | null): void {
  if (store.selection === id) return;
  store.selection = id;
  emit("selection");
}

export function setHover(id: number | null): void {
  if (store.hover === id) return;
  store.hover = id;
  emit("hover");
}

export function setTrace(t: Trace | null): void {
  store.trace = t;
  emit("trace");
}

/** Building indices the current trace passes through. */
export function traceBuildings(t: Trace | null = store.trace): Set<number> {
  const out = new Set<number>();
  if (!t || !store.build) return out;
  const byPath = new Map(store.build.model.buildings.map((b, i) => [b.path, i]));
  for (const s of t.steps) {
    const i = byPath.get(s.path.split("#")[0]);
    if (i !== undefined) out.add(i);
  }
  return out;
}

export function snapshot(): Record<string, unknown> {
  const s = store;
  const b = s.build;
  const selB = s.selection !== null ? buildingAt(buildingForNode(s.selection)) : null;
  return {
    repo: s.repo,
    graph_loaded: s.graphLoaded,
    tab: s.tab,
    step: s.step,
    steps: stepCount(),
    step_title: s.step > 0 ? b?.design.steps[s.step - 1]?.title ?? null : null,
    playing: s.playing,
    view: s.view,
    spin: s.spin,
    selection: s.selection,
    selection_path: selB ? (s.selection !== null && s.brickOf.has(s.selection) ? `${selB.path}#${b!.model.bricks[s.brickOf.get(s.selection)!].name}` : selB.path) : null,
    hover: s.hover,
    trace: s.trace?.entry_path ?? null,
    trace_on_model: s.traceOnModel,
    build: b ? { title: b.design.title, source: b.design.source, model: b.design.model ?? null, pieces: b.check.pieces, steps: b.check.steps, sub_builds: b.check.sub_builds, weak: b.check.weak.length, repairs: b.check.repairs?.length ?? 0 } : null,
    designing: s.design.running,
    search: s.search,
    panels: { shelf: s.shelfOpen, shelf_tab: s.shelfTab, card: s.selection !== null && s.tab !== "manual", empty: !s.graphLoaded, scanning: s.scanning !== null },
    scanning: s.scanning,
    bridge_port: s.bridgePort,
    ts: new Date().toISOString(),
  };
}
