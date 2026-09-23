// Single source of truth for the UI. The diagram and the panels subscribe to it;
// the bridge reads it to answer `/state`.

import type { Atlas, Component, Container, Journey, Message, Proposal, Relationship, Stats } from "./types";

/** The C4 levels. `code` is one component's files. */
export type Level = "context" | "containers" | "components" | "code";
export type ShelfTab = "map" | "journeys" | "guide";
/** How a playing journey is shown: numbered on the C4 map, or as a sequence diagram. */
export type View = "map" | "sequence";

export interface AgentState {
  key: string;
  role: string;
  target: string | null;
  name: string;
  done: boolean;
  ok: boolean | null;
  error: string | null;
  cost_usd: number;
  secs: number;
  reads: number;
  current: string | null;
}

export interface Note {
  t: number;
  kind: "stage" | "agent" | "found" | "check" | "error";
  text: string;
}

export interface DiscoveryState {
  running: boolean;
  model: string;
  stage: string | null;
  startedAt: number;
  finishedAt: number | null;
  total: number;
  done: number;
  cost_usd: number;
  agents: AgentState[];
  notes: Note[];
  /** Files agents have opened, by container id. */
  reads: Map<string, string[]>;
  /** The key flows the scout proposed this run; the journeys tab shows them while the narrators work. */
  proposed: Proposal[];
  error: string | null;
}

export interface Store {
  repo: string | null;
  stats: Stats | null;
  atlas: Atlas | null;
  stale: string | null;
  level: Level;
  /** Container id at `components`, component id at `code`. */
  focus: string | null;
  /** Selected element id (container, component, person, external), or `null`. */
  selection: string | null;
  /** Selected relationship, as `from>to`, or `null`. */
  relSelection: string | null;
  hover: string | null;
  journey: Journey | null;
  /** 1-based step of the playing journey, counted over its projection at the current level; 0 shows the whole path. */
  journeyStep: number;
  view: View;
  /** A journey being edited: a copy that the sequence view draws live until it is saved. */
  draft: Journey | null;
  /** Id of the journey a narrator is rewriting, while it is. */
  narrating: string | null;
  shelfTab: ShelfTab;
  discovery: DiscoveryState;
  notesOpen: boolean;
  search: string;
  shelfOpen: boolean;
  bridgePort: number | null;
  scanning: string | null;
  graphLoaded: boolean;
}

export function freshDiscovery(): DiscoveryState {
  return { running: false, model: "", stage: null, startedAt: 0, finishedAt: null, total: 0, done: 0, cost_usd: 0, agents: [], notes: [], reads: new Map(), proposed: [], error: null };
}

export const store: Store = {
  repo: null,
  stats: null,
  atlas: null,
  stale: null,
  level: "containers",
  focus: null,
  selection: null,
  relSelection: null,
  hover: null,
  journey: null,
  journeyStep: 0,
  view: "map",
  draft: null,
  narrating: null,
  shelfTab: "map",
  discovery: freshDiscovery(),
  notesOpen: false,
  search: "",
  shelfOpen: true,
  bridgePort: null,
  scanning: null,
  graphLoaded: false,
};

type Topic = "atlas" | "level" | "selection" | "hover" | "journey" | "discovery" | "repo" | "ui";
const listeners = new Map<Topic, Set<() => void>>();

export function subscribe(topic: Topic, fn: () => void): () => void {
  if (!listeners.has(topic)) listeners.set(topic, new Set());
  listeners.get(topic)!.add(fn);
  return () => listeners.get(topic)!.delete(fn);
}

export function emit(topic: Topic): void {
  listeners.get(topic)?.forEach((fn) => fn());
}

export function setAtlas(a: Atlas, stale: string | null): void {
  store.atlas = a;
  store.stale = stale;
  store.graphLoaded = true;
  if (store.focus && !elementById(store.focus)) { store.focus = null; store.level = "containers"; }
  if (store.selection && !elementById(store.selection)) store.selection = null;
  if (store.journey) {
    store.journey = a.journeys.find((j) => j.id === store.journey!.id) ?? null;
    if (!store.journey) { store.view = "map"; store.draft = null; }
    store.journeyStep = Math.min(store.journeyStep, journeyLength());
  }
  emit("atlas");
  emit("level");
  emit("selection");
  emit("journey");
}

// ---- journeys projected onto a level ----------------------------------------------------

export interface PMessage {
  /** 1-based index of the first journey message behind this arrow. */
  n: number;
  from: string;
  to: string;
  label: string;
  caption: string;
  kind: Message["kind"];
  depth: number;
  source: Message["source"];
  by: Message["by"];
}

export interface Projection {
  participants: string[];
  messages: PMessage[];
}

/** Where an element sits at a level: itself, its container, or the system; `null` when it has no box there. */
export function atLevel(id: string, level: Level, focus: string | null): string | null {
  if (id.startsWith("p:") || id.startsWith("x:")) return id;
  const c = containerOf(id);
  if (!c || c.hidden) return null;
  if (level === "context") return "s";
  if (level === "containers") return c.id;
  const f = focus ? containerOf(focus)?.id ?? null : null;
  if (f === c.id) return id.includes("/") ? id : c.components[0]?.id ?? null;
  return c.id;
}

/** The same rule as `atlas::project` in the core: messages inside one box vanish, consecutive
 * messages that land on the same arrow merge. Every level of the atlas is a view of one list. */
export function project(j: Journey, level: Level, focus: string | null): Projection {
  const lvl: Level = level === "code" ? "components" : level;
  const participants: string[] = [];
  const messages: PMessage[] = [];
  j.messages.forEach((m, i) => {
    const from = atLevel(m.from, lvl, focus);
    const to = atLevel(m.to, lvl, focus);
    if (!from || !to || from === to) return;
    const last = messages[messages.length - 1];
    if (last && last.from === from && last.to === to && last.kind === m.kind && (last.label === m.label || lvl !== "components")) {
      if (!last.label.includes(m.label) && last.label.split(", ").length < 3) last.label = `${last.label}, ${m.label}`;
      return;
    }
    for (const p of [from, to]) if (!participants.includes(p)) participants.push(p);
    messages.push({ n: i + 1, from, to, label: m.label, caption: m.caption, kind: m.kind, depth: m.depth, source: m.source, by: m.by });
  });
  return { participants, messages };
}

/** The journey on show (the draft while editing), projected onto the current level. */
export function currentProjection(): Projection | null {
  const j = store.draft ?? store.journey;
  if (!j) return null;
  return project(j, store.level, store.focus);
}

export function journeyLength(): number {
  return currentProjection()?.messages.length ?? 0;
}

export function setView(v: View): void {
  if (store.view === v) return;
  store.view = v;
  emit("journey");
  emit("ui");
}

export function shownContainers(): Container[] {
  return store.atlas?.containers.filter((c) => !c.hidden) ?? [];
}

export function containerOf(id: string): Container | null {
  const cid = id.includes("/") ? id.slice(0, id.indexOf("/")) : id;
  return store.atlas?.containers.find((c) => c.id === cid) ?? null;
}

export type Element =
  | { kind: "system" }
  | { kind: "person"; person: import("./types").Person }
  | { kind: "external"; external: import("./types").External }
  | { kind: "container"; container: Container }
  | { kind: "component"; container: Container; component: Component };

export function elementById(id: string): Element | null {
  const a = store.atlas;
  if (!a) return null;
  if (id === "s") return { kind: "system" };
  const p = a.people.find((x) => x.id === id);
  if (p) return { kind: "person", person: p };
  const x = a.externals.find((x) => x.id === id);
  if (x) return { kind: "external", external: x };
  for (const c of a.containers) {
    if (c.id === id) return { kind: "container", container: c };
    const k = c.components.find((k) => k.id === id);
    if (k) return { kind: "component", container: c, component: k };
  }
  return null;
}

export function elementName(id: string): string {
  const e = elementById(id);
  if (!e) return store.atlas?.journeys.find((j) => j.id === id)?.name ?? id;
  switch (e.kind) {
    case "system": return store.atlas!.system.name;
    case "person": return e.person.name;
    case "external": return e.external.name;
    case "container": return e.container.name;
    case "component": return e.component.name;
  }
}

/** Component id that holds a file path, if any. */
export function componentOfFile(path: string): string | null {
  const file = path.split("#")[0];
  for (const c of store.atlas?.containers ?? []) for (const k of c.components) if (k.files.includes(file)) return k.id;
  return null;
}

export function relationshipsOf(id: string): Relationship[] {
  return store.atlas?.relationships.filter((r) => r.from === id || r.to === id) ?? [];
}

export function relKey(r: Relationship): string {
  return `${r.from}>${r.to}`;
}

export function setLevel(level: Level, focus: string | null = null): void {
  if (store.level === level && store.focus === focus) return;
  store.level = level;
  store.focus = focus;
  store.relSelection = null;
  // the step counts arrows at this level; keep it in range
  if (store.journey) store.journeyStep = Math.min(store.journeyStep, journeyLength());
  emit("level");
  if (store.journey) emit("journey");
}

/** Go one level in at the element: system → containers, container → components, component → code. */
export function zoomInto(id: string): void {
  const e = elementById(id);
  if (!e) return;
  if (e.kind === "system") setLevel("containers");
  else if (e.kind === "container") setLevel("components", e.container.id);
  else if (e.kind === "component") setLevel("code", e.component.id);
}

export function zoomOut(): void {
  if (store.level === "code") setLevel("components", containerOf(store.focus ?? "")?.id ?? null);
  else if (store.level === "components") setLevel("containers");
  else if (store.level === "containers") setLevel("context");
}

export function select(id: string | null): void {
  if (store.selection === id && store.relSelection === null) return;
  store.selection = id;
  store.relSelection = null;
  emit("selection");
}

export function selectRelationship(key: string | null): void {
  store.relSelection = key;
  store.selection = null;
  emit("selection");
}

export function setHover(id: string | null): void {
  if (store.hover === id) return;
  store.hover = id;
  emit("hover");
}

export function setJourney(j: Journey | null, step = 0): void {
  if (store.draft && store.draft.id !== j?.id) store.draft = null;
  store.journey = j;
  if (!j) store.view = "map";
  store.journeyStep = j ? Math.max(0, Math.min(journeyLength(), step)) : 0;
  emit("journey");
  emit("ui");
}

export function setJourneyStep(n: number): void {
  if (!store.journey) return;
  store.journeyStep = Math.max(0, Math.min(journeyLength(), n));
  emit("journey");
}

/** Start editing the playing journey: the sequence view draws the draft from now on. */
export function startDraft(from?: Journey): void {
  const j = from ?? store.journey;
  if (!j) return;
  store.draft = JSON.parse(JSON.stringify(j)) as Journey;
  if (!store.journey || store.journey.id !== j.id) { store.journey = j; store.journeyStep = 0; }
  store.view = "sequence";
  emit("journey");
  emit("ui");
}

export function endDraft(): void {
  store.draft = null;
  if (store.journey) store.journeyStep = Math.min(store.journeyStep, journeyLength());
  emit("journey");
  emit("ui");
}

/** Every element a message may join, finest grain first: people, components, containers, outside systems. */
export function participantChoices(): { id: string; name: string; group: string }[] {
  const a = store.atlas;
  if (!a) return [];
  const out: { id: string; name: string; group: string }[] = [];
  for (const p of a.people) out.push({ id: p.id, name: p.name, group: "People" });
  for (const c of shownContainers()) {
    out.push({ id: c.id, name: c.name, group: "Containers" });
    for (const k of c.components) out.push({ id: k.id, name: `${c.name} / ${k.name}`, group: "Components" });
  }
  for (const x of a.externals) out.push({ id: x.id, name: x.name, group: "Outside" });
  return out;
}

export function snapshot(): Record<string, unknown> {
  const s = store;
  const a = s.atlas;
  const d = s.discovery;
  return {
    repo: s.repo,
    graph_loaded: s.graphLoaded,
    level: s.level,
    focus: s.focus,
    focus_name: s.focus ? elementName(s.focus) : null,
    selection: s.selection,
    selection_name: s.selection ? elementName(s.selection) : null,
    relationship: s.relSelection,
    hover: s.hover,
    journey: s.journey?.id ?? null,
    journey_name: s.journey?.name ?? null,
    journey_step: s.journey ? s.journeyStep : null,
    journey_steps: s.journey ? journeyLength() : null,
    journey_source: s.journey?.source ?? null,
    view: s.journey ? s.view : null,
    editing: s.draft?.id ?? null,
    narrating: s.narrating,
    atlas: a ? { name: a.system.name, source: a.source, model: a.model ?? null, containers: shownContainers().length, components: shownContainers().reduce((n, c) => n + c.components.length, 0), people: a.people.length, externals: a.externals.length, relationships: a.relationships.length, journeys: a.journeys.length, backed: a.report.backed, survey: a.report.survey, claimed: a.report.claimed, stale: s.stale } : null,
    discovering: d.running,
    discovery: d.running || d.finishedAt ? { stage: d.stage, done: d.done, total: d.total, cost_usd: Math.round(d.cost_usd * 100) / 100, agents: d.agents.map((x) => ({ name: x.name, done: x.done, ok: x.ok, reads: x.reads })), notes: d.notes.length } : null,
    search: s.search,
    panels: { shelf: s.shelfOpen, shelf_tab: s.shelfTab, card: (s.selection !== null || s.relSelection !== null) && !s.notesOpen, notes: s.notesOpen, journey_bar: s.journey !== null, sequence: s.journey !== null && s.view === "sequence", editor: s.draft !== null, empty: !s.graphLoaded, scanning: s.scanning !== null },
    scanning: s.scanning,
    bridge_port: s.bridgePort,
    ts: new Date().toISOString(),
  };
}
