// Single source of truth for the UI. Panels and the renderer subscribe to it;
// the bridge reads it to answer `/state`.

import type { EdgeKind, Lang, Level, Stats, ViewEdge, ViewGraph, ViewNode } from "./types";

export interface Filters {
  langs: Set<Lang>;
  edges: Set<EdgeKind>;
  tag: string;
  externals: boolean;
}

export interface LayoutState {
  running: boolean;
  backend: string;
  iteration: number;
  energy: number;
}

export interface Camera {
  x: number;
  y: number;
  zoom: number;
}

export interface Store {
  repo: string | null;
  stats: Stats | null;
  level: Level;
  focus: number | null;
  generation: number;
  nodes: ViewNode[];
  edges: ViewEdge[];
  index: Map<number, number>; // node id -> array index
  positions: Float32Array;
  selection: number | null;
  hover: number | null;
  neighbourIds: Set<number>;
  filters: Filters;
  layout: LayoutState;
  camera: Camera;
  search: string;
  shelfTab: "packages" | "flows" | "boundaries";
  shelfOpen: boolean;
  bridgePort: number | null;
  scanning: string | null;
  graphLoaded: boolean;
}

export const store: Store = {
  repo: null,
  stats: null,
  level: "file",
  focus: null,
  generation: 0,
  nodes: [],
  edges: [],
  index: new Map(),
  positions: new Float32Array(0),
  selection: null,
  hover: null,
  neighbourIds: new Set(),
  filters: { langs: new Set(["rust", "typescript", "javascript", "python", "go", "other"]), edges: new Set(["imports", "calls", "flow"]), tag: "", externals: true },
  layout: { running: false, backend: "", iteration: 0, energy: 0 },
  camera: { x: 0, y: 0, zoom: 1 },
  search: "",
  shelfTab: "packages",
  shelfOpen: true,
  bridgePort: null,
  scanning: null,
  graphLoaded: false,
};

type Topic = "graph" | "positions" | "selection" | "hover" | "filters" | "layout" | "camera" | "repo" | "ui";
const listeners = new Map<Topic, Set<() => void>>();

export function subscribe(topic: Topic, fn: () => void): () => void {
  if (!listeners.has(topic)) listeners.set(topic, new Set());
  listeners.get(topic)!.add(fn);
  return () => listeners.get(topic)!.delete(fn);
}

export function emit(topic: Topic): void {
  listeners.get(topic)?.forEach((fn) => fn());
}

export function setGraph(view: ViewGraph, positions: number[], level: Level, focus: number | null, generation: number): void {
  store.nodes = view.nodes;
  store.edges = view.edges;
  store.index = new Map(view.nodes.map((n, i) => [n.id, i]));
  store.positions = Float32Array.from(positions);
  store.level = level;
  store.focus = focus;
  store.generation = generation;
  store.graphLoaded = true;
  if (store.selection !== null && !store.index.has(store.selection)) store.selection = null;
  store.hover = null;
  recomputeNeighbours();
  emit("graph");
  emit("selection");
}

export function setPositions(flat: number[], generation: number): boolean {
  if (generation !== store.generation || flat.length !== store.positions.length) return false;
  store.positions.set(flat);
  emit("positions");
  return true;
}

export function select(id: number | null): void {
  if (store.selection === id) return;
  store.selection = id;
  recomputeNeighbours();
  emit("selection");
}

export function setHover(id: number | null): void {
  if (store.hover === id) return;
  store.hover = id;
  emit("hover");
}

function recomputeNeighbours(): void {
  store.neighbourIds = new Set();
  const s = store.selection;
  if (s === null) return;
  for (const e of store.edges) {
    if (e.from === s) store.neighbourIds.add(e.to);
    else if (e.to === s) store.neighbourIds.add(e.from);
  }
}

export function nodeVisible(n: ViewNode): boolean {
  const f = store.filters;
  if (!f.langs.has(n.lang)) return false;
  if (!f.externals && n.external) return false;
  if (f.tag && !(n.tags ?? []).some((t) => t.startsWith(f.tag))) return false;
  return true;
}

export function edgeVisible(e: ViewEdge, visible: Uint8Array): boolean {
  if (!store.filters.edges.has(e.kind)) return false;
  const a = store.index.get(e.from);
  const b = store.index.get(e.to);
  if (a === undefined || b === undefined) return false;
  return visible[a] === 1 && visible[b] === 1;
}

export function snapshot(): Record<string, unknown> {
  const s = store;
  return {
    repo: s.repo,
    graph_loaded: s.graphLoaded,
    level: s.level,
    focus: s.focus,
    generation: s.generation,
    selection: s.selection,
    selection_path: s.selection !== null ? s.nodes[s.index.get(s.selection) ?? -1]?.path ?? null : null,
    hover: s.hover,
    camera: { ...s.camera },
    layout: { ...s.layout },
    filters: { langs: [...s.filters.langs], edges: [...s.filters.edges], tag: s.filters.tag, externals: s.filters.externals },
    search: s.search,
    panels: { shelf: s.shelfOpen, shelf_tab: s.shelfTab, card: s.selection !== null, empty: !s.graphLoaded, scanning: s.scanning !== null },
    nodes_visible: s.nodes.filter(nodeVisible).length,
    edges_visible: s.edges.filter((e) => s.filters.edges.has(e.kind)).length,
    nodes: s.nodes.length,
    edges: s.edges.length,
    scanning: s.scanning,
    bridge_port: s.bridgePort,
    ts: new Date().toISOString(),
  };
}
