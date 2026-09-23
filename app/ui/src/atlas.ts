// The diagrams. One SVG per level, laid out by layout.ts, with the boxes drawn
// as HTML inside foreignObjects so names and descriptions wrap like text should.
// Context: people, the system, outside systems. Containers: what runs. Components:
// one container opened up, with its neighbours dimmed around it. Code: a page of
// one component's files.

import { api, log } from "./tauri";
import { store, subscribe, emit, select, selectRelationship, setHover, zoomInto, zoomOut, setLevel, elementById, shownContainers, containerOf, elementName, currentProjection, type Level } from "./store";
import { layout, type LNode, type LEdge, type Layout } from "./layout";
import { EXTERNAL_LABEL, KIND_LABEL, langOf, type Lang, type Relationship, type NodeDetail } from "./types";
import { esc, prose, toast } from "./panels";
import { sequenceShown } from "./sequence";

const SVG = "http://www.w3.org/2000/svg";
const XHTML = "http://www.w3.org/1999/xhtml";
const $ = <T extends HTMLElement>(sel: string): T => document.querySelector(sel) as T;

type DKind = "person" | "system" | "external" | "container" | "component" | "neighbour";

interface DNode {
  id: string;
  kind: DKind;
  title: string;
  subtitle: string;
  desc: string;
  lang: Lang;
  externalKind?: string;
  w: number;
  h: number;
  pin?: number;
}

interface DEdge {
  key: string;
  from: string;
  to: string;
  label: string;
  technology: string;
  source: Relationship["source"];
  /** The relationships this edge stands for (several roll up at the context level). */
  rels: Relationship[];
}

const SIZE: Record<DKind, [number, number]> = {
  person: [156, 104],
  system: [300, 128],
  external: [216, 84],
  container: [236, 112],
  component: [216, 100],
  neighbour: [200, 68],
};

let svg: SVGSVGElement;
let viewBox = { x: 0, y: 0, w: 1000, h: 600 };
let lastLevelKey = "";
let lastNames = new Map<string, string>();
let lastReads = new Map<string, number>();
let current: { nodes: DNode[]; edges: DEdge[]; lay: Layout } | null = null;
/** The user panned or zoomed since the last level change: leave the view alone. */
let userMoved = false;

export function initAtlas(): void {
  svg = document.getElementById("diagram") as unknown as SVGSVGElement;
  subscribe("atlas", render);
  subscribe("level", () => { render(); renderCrumbs(); });
  subscribe("selection", () => { applyEmphasis(); renderCrumbs(); });
  subscribe("hover", applyEmphasis);
  subscribe("journey", () => { showStage(); applyEmphasis(); });
  subscribe("discovery", renderLive);
  initPanZoom();
  document.querySelectorAll<HTMLButtonElement>("[data-level]").forEach((b) => b.addEventListener("click", () => goLevel(b.dataset.level as Level)));
  $("#fit-btn").addEventListener("click", () => { userMoved = false; fit(true); });
}

/** Move to a level, keeping the focus that makes sense from where we are. */
export function goLevel(level: Level): void {
  if (level === "context" || level === "containers") { setLevel(level); return; }
  if (level === "components") {
    const c = store.focus ? containerOf(store.focus) : null;
    const sel = store.selection ? containerOf(store.selection) : null;
    const target = c ?? sel ?? shownContainers()[0];
    if (target) setLevel("components", target.id);
    return;
  }
  const k = store.selection && elementById(store.selection)?.kind === "component" ? store.selection : store.focus && elementById(store.focus)?.kind === "component" ? store.focus : null;
  if (k) setLevel("code", k);
  else toast("Pick a component first, then open its code", "info");
}

// ---- building one diagram from the atlas ---------------------------------------------

function node(id: string, kind: DKind, title: string, subtitle: string, desc: string, lang: Lang, extra: Partial<DNode> = {}): DNode {
  const [w, h] = SIZE[kind];
  return { id, kind, title, subtitle, desc, lang, w, h, ...extra };
}

function diagram(level: Level, focus: string | null): { nodes: DNode[]; edges: DEdge[] } {
  const a = store.atlas!;
  const nodes: DNode[] = [];
  const edges = new Map<string, DEdge>();
  const addEdge = (from: string, to: string, r: Relationship) => {
    if (from === to) return;
    const key = `${from}>${to}`;
    const e = edges.get(key);
    if (e) {
      e.rels.push(r);
      if (!e.label.includes(r.label) && e.label.split(", ").length < 3 && r.label) e.label = e.label ? `${e.label}, ${r.label}` : r.label;
      if (r.source === "claimed" && e.source !== "code") e.source = "claimed";
      if (r.source === "code") e.source = "code";
    } else edges.set(key, { key, from, to, label: r.label, technology: r.technology ?? "", source: r.source, rels: [r] });
  };
  const containers = shownContainers();
  const shownIds = new Set(containers.map((c) => c.id));
  const people = () => a.people.forEach((p) => nodes.push(node(p.id, "person", p.name, "Person", p.description, "other", { pin: 0 })));
  const externals = (ids: Set<string>) => a.externals.filter((x) => ids.has(x.id)).forEach((x) => nodes.push(node(x.id, "external", x.name, EXTERNAL_LABEL[x.kind] ?? "Outside", x.description, "other", { pin: -1, externalKind: x.kind })));

  if (level === "context") {
    people();
    nodes.push(node("s", "system", a.system.name, "Software system", a.system.purpose, "other"));
    const used = new Set<string>();
    for (const r of a.relationships) {
      if (r.level !== "container") continue;
      const fromC = shownIds.has(r.from);
      const toC = shownIds.has(r.to);
      if (r.from.startsWith("p:") && toC) addEdge(r.from, "s", r);
      else if (fromC && r.to.startsWith("x:")) { used.add(r.to); addEdge("s", r.to, r); }
      else if (r.from.startsWith("x:") && toC) { used.add(r.from); addEdge(r.from, "s", r); }
    }
    externals(used);
  } else if (level === "containers") {
    people();
    for (const c of containers) nodes.push(node(c.id, "container", c.name, `${KIND_LABEL[c.kind] ?? "Container"} · ${c.technology}`, c.description, langOf(c.technology, c.language)));
    const used = new Set<string>();
    for (const r of a.relationships) {
      if (r.level !== "container") continue;
      const ok = (id: string) => shownIds.has(id) || id.startsWith("p:") || id.startsWith("x:");
      if (!ok(r.from) || !ok(r.to)) continue;
      if (r.from.startsWith("x:")) used.add(r.from);
      if (r.to.startsWith("x:")) used.add(r.to);
      addEdge(r.from, r.to, r);
    }
    externals(used);
  } else if (level === "components" && focus) {
    const c = a.containers.find((x) => x.id === focus);
    if (!c) return { nodes, edges: [] };
    for (const k of c.components) nodes.push(node(k.id, "component", k.name, k.technology || c.technology, k.description, langOf(k.technology || c.technology, c.language)));
    const mine = new Set(c.components.map((k) => k.id));
    const usedX = new Set<string>();
    const neighbours = new Set<string>();
    const side = (id: string): string | null => {
      if (mine.has(id)) return id;
      if (id.startsWith("x:")) { usedX.add(id); return id; }
      const other = containerOf(id);
      if (other && shownIds.has(other.id) && other.id !== c.id) { neighbours.add(other.id); return other.id; }
      return null;
    };
    for (const r of a.relationships) {
      if (r.level !== "component") continue;
      if (!mine.has(r.from) && !mine.has(r.to)) continue;
      const from = side(r.from);
      const to = side(r.to);
      if (from && to) addEdge(from, to, r);
    }
    for (const id of neighbours) {
      const o = a.containers.find((x) => x.id === id)!;
      nodes.push(node(o.id, "neighbour", o.name, KIND_LABEL[o.kind] ?? "Container", "", langOf(o.technology, o.language)));
    }
    externals(usedX);
  }
  return { nodes, edges: [...edges.values()] };
}

// ---- rendering ---------------------------------------------------------------------------

/** The sequence view sits over the map and the code page while a journey is shown that way. */
function showStage(): void {
  const seq = sequenceShown();
  svg.classList.toggle("is-hidden", seq || store.level === "code");
  $("#code-view").hidden = seq || store.level !== "code";
}

function render(): void {
  const stage = $("#stage");
  const code = $("#code-view");
  if (!store.atlas) { svg.replaceChildren(); code.hidden = true; current = null; return; }
  const levelKey = `${store.level}:${store.focus ?? ""}`;
  const levelChanged = levelKey !== lastLevelKey;
  lastLevelKey = levelKey;
  if (store.level === "code") {
    svg.classList.add("is-hidden");
    code.hidden = sequenceShown();
    void renderCode();
    stage.dataset.level = "code";
    return;
  }
  code.hidden = true;
  svg.classList.toggle("is-hidden", sequenceShown());
  stage.dataset.level = store.level;
  const { nodes, edges } = diagram(store.level, store.focus);
  const lay = layout(
    nodes.map<LNode>((n) => ({ id: n.id, w: n.w, h: n.h, pin: n.pin })),
    edges.map<LEdge>((e) => ({ from: e.from, to: e.to })),
    { hgap: 44, vgap: store.level === "context" ? 132 : 124, pad: 32 },
  );
  current = { nodes, edges, lay };
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const pos = new Map(lay.nodes.map((p) => [p.id, p]));

  const root = el("g", { class: `diagram-root${levelChanged ? " is-entering" : ""}` });
  root.appendChild(el("rect", { class: "plate", x: -20000, y: -20000, width: 40000, height: 40000, fill: "url(#studs)" }));
  const gEdges = el("g", { class: "edges" });
  const gNodes = el("g", { class: "nodes" });
  const gLabels = el("g", { class: "labels" });
  const gJourney = el("g", { class: "journey-marks" });

  for (const r of lay.edges) {
    const e = edges.find((x) => x.from === r.from && x.to === r.to)!;
    const g = el("g", { class: `edge is-${e.source}`, "data-key": e.key, "data-from": e.from, "data-to": e.to });
    g.appendChild(el("path", { class: "hit", d: r.d }));
    g.appendChild(el("path", { class: "wire", d: r.d, "marker-end": `url(#arrow-${e.source})` }));
    g.addEventListener("click", (ev) => { ev.stopPropagation(); selectRelationship(e.key); });
    g.addEventListener("mouseenter", () => setHover(e.key));
    g.addEventListener("mouseleave", () => setHover(null));
    gEdges.appendChild(g);
    if (e.label || e.technology) {
      const fo = el("foreignObject", { x: r.mid.x - 120, y: r.mid.y - 22, width: 240, height: 44, class: "edge-label", "data-key": e.key });
      const div = document.createElementNS(XHTML, "div");
      div.className = `pill-wrap`;
      div.innerHTML = `<span class="edge-pill is-${e.source}" title="${esc(e.rels.map((x) => x.label).join("; "))}">${esc(shorten(e.label, 34))}${e.technology ? `<i>${esc(e.technology)}</i>` : ""}${e.source === "claimed" ? `<b>claimed</b>` : ""}</span>`;
      div.addEventListener("click", (ev) => { ev.stopPropagation(); selectRelationship(e.key); });
      fo.appendChild(div);
      gLabels.appendChild(fo);
    }
  }
  for (const n of nodes) {
    const p = pos.get(n.id)!;
    gNodes.appendChild(nodeGroup(n, p.x, p.y, byId));
  }
  root.append(gEdges, gNodes, gLabels, gJourney);
  svg.replaceChildren(defs(), root);
  lastNames = new Map(nodes.map((n) => [n.id, n.title]));
  if (levelChanged) userMoved = false;
  if (!userMoved) fit(false);
  else applyViewBox();
  applyEmphasis();
  renderLive();
}

function shorten(s: string, n: number): string {
  return s.length > n ? `${s.slice(0, n - 1)}…` : s;
}

function nodeGroup(n: DNode, x: number, y: number, _byId: Map<string, DNode>): SVGGElement {
  const developed = lastNames.has(n.id) && lastNames.get(n.id) !== n.title;
  const g = el("g", { class: `node is-${n.kind}${developed ? " is-developed" : ""}`, transform: `translate(${x},${y})`, "data-id": n.id, "data-lang": n.lang, "data-testid": `c4-${n.id}` }) as SVGGElement;
  if (n.kind === "person") {
    g.appendChild(el("circle", { class: "frame head", cx: n.w / 2, cy: 18, r: 16 }));
    g.appendChild(el("rect", { class: "frame body", x: 0, y: 34, width: n.w, height: n.h - 34, rx: 14 }));
  } else if (n.kind === "external" && n.externalKind === "database") {
    const r = 9;
    g.appendChild(el("path", { class: "frame", d: `M0,${r} a${n.w / 2},${r} 0 0 1 ${n.w},0 v${n.h - 2 * r} a${n.w / 2},${r} 0 0 1 -${n.w},0 z` }));
    g.appendChild(el("path", { class: "frame-line", d: `M0,${r} a${n.w / 2},${r} 0 0 0 ${n.w},0` }));
  } else if (n.kind === "external" && n.externalKind === "queue") {
    g.appendChild(el("rect", { class: "frame", x: 0, y: 0, width: n.w, height: n.h, rx: n.h / 2 }));
    g.appendChild(el("path", { class: "frame-line", d: `M${n.h / 2},0 a${n.h / 4},${n.h / 2} 0 0 0 0,${n.h} M${n.w - n.h / 2},0 a${n.h / 4},${n.h / 2} 0 0 1 0,${n.h}` }));
  } else {
    g.appendChild(el("rect", { class: "frame", x: 0, y: 0, width: n.w, height: n.h, rx: n.kind === "system" ? 18 : n.kind === "neighbour" ? 10 : 14 }));
    if (n.kind === "container" || n.kind === "component" || n.kind === "neighbour") g.appendChild(el("rect", { class: "tint", x: 0, y: 0, width: 5, height: n.h, rx: 2.5 }));
  }
  const fo = el("foreignObject", { x: 0, y: n.kind === "person" ? 34 : 0, width: n.w, height: n.kind === "person" ? n.h - 34 : n.h });
  const div = document.createElementNS(XHTML, "div");
  div.className = `c4 c4-${n.kind}`;
  div.innerHTML = `
    <div class="c4-title">${esc(n.title)}</div>
    ${n.subtitle ? `<div class="c4-sub">${esc(n.subtitle)}</div>` : ""}
    ${n.desc ? `<div class="c4-desc">${prose(n.desc)}</div>` : ""}
    <div class="c4-live" hidden></div>`;
  fo.appendChild(div);
  g.appendChild(fo);
  if (n.kind !== "neighbour") g.appendChild(el("rect", { class: "ring", x: -5, y: -5, width: n.w + 10, height: n.h + 10, rx: n.kind === "system" ? 22 : 18 }));
  g.addEventListener("click", (ev) => { ev.stopPropagation(); onNodeClick(n); });
  g.addEventListener("dblclick", (ev) => { ev.stopPropagation(); zoomInto(n.id); });
  g.addEventListener("mouseenter", () => setHover(n.id));
  g.addEventListener("mouseleave", () => setHover(null));
  return g;
}

function onNodeClick(n: DNode): void {
  if (n.kind === "neighbour") { setLevel("components", n.id); return; }
  if (n.kind === "system") { select("s"); return; }
  select(n.id);
}

function defs(): SVGDefsElement {
  const d = el("defs") as SVGDefsElement;
  const pat = el("pattern", { id: "studs", width: 28, height: 28, patternUnits: "userSpaceOnUse" });
  pat.appendChild(el("circle", { cx: 14, cy: 14, r: 1.1, class: "stud" }));
  d.appendChild(pat);
  for (const kind of ["code", "survey", "claimed", "journey"]) {
    const m = el("marker", { id: `arrow-${kind}`, viewBox: "0 0 10 10", refX: 9, refY: 5, markerWidth: 9, markerHeight: 9, orient: "auto-start-reverse", class: `arrow is-${kind}` });
    m.appendChild(el("path", { d: "M0,0.5 L10,5 L0,9.5 z" }));
    d.appendChild(m);
  }
  return d;
}

function el(tag: string, attrs: Record<string, string | number> = {}): SVGElement {
  const e = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
  return e;
}

// ---- emphasis: hover, selection, journey -------------------------------------------------

/** The journey's arrows at this level, mapped onto the diagram's edges: edge key -> step numbers.
 * The same projection the sequence view draws, so the numbers agree between the two. */
function journeyOnDiagram(): { edges: Map<string, number[]>; nodes: Set<string>; current: string | null; currentNode: string | null } {
  const out = { edges: new Map<string, number[]>(), nodes: new Set<string>(), current: null as string | null, currentNode: null as string | null };
  const p = store.journey ? currentProjection() : null;
  if (!p || !current) return out;
  const keys = new Set(current.edges.map((e) => e.key));
  p.messages.forEach((m, i) => {
    if (store.journeyStep > 0 && i + 1 > store.journeyStep) return;
    // a return rides the arrow it answers; the map has no arrow the other way
    const key = m.kind === "return" ? `${m.to}>${m.from}` : `${m.from}>${m.to}`;
    const alt = m.kind === "return" ? `${m.from}>${m.to}` : `${m.to}>${m.from}`;
    const k = keys.has(key) ? key : keys.has(alt) ? alt : null;
    if (!k) { out.nodes.add(m.from); out.nodes.add(m.to); if (store.journeyStep === i + 1) out.currentNode = m.to; return; }
    out.nodes.add(m.from);
    out.nodes.add(m.to);
    const list = out.edges.get(k) ?? [];
    list.push(i + 1);
    out.edges.set(k, list);
    if (store.journeyStep === i + 1) out.current = k;
  });
  return out;
}

function applyEmphasis(): void {
  if (!current) return;
  const hover = store.hover;
  const sel = store.selection;
  const relSel = store.relSelection;
  const j = journeyOnDiagram();
  // Selection and journeys dim the rest; a resting pointer only brightens what it touches.
  const related = new Set<string>();
  const relatedEdges = new Set<string>();
  const collect = (id: string | null) => {
    if (!id) return;
    if (id.includes(">")) { const [f, t] = id.split(">"); related.add(f); related.add(t); relatedEdges.add(id); return; }
    related.add(id);
    for (const e of current!.edges) if (e.from === id || e.to === id) { related.add(e.from); related.add(e.to); relatedEdges.add(e.key); }
  };
  collect(relSel ?? sel);
  const dimming = related.size > 0 || j.edges.size > 0;
  const hot = new Set<string>();
  const hotEdges = new Set<string>();
  if (hover) {
    if (hover.includes(">")) { hotEdges.add(hover); const [f, t] = hover.split(">"); hot.add(f); hot.add(t); }
    else { hot.add(hover); for (const e of current.edges) if (e.from === hover || e.to === hover) { hotEdges.add(e.key); hot.add(e.from); hot.add(e.to); } }
  }
  svg.classList.toggle("has-emphasis", dimming);
  svg.classList.toggle("has-journey", store.journey !== null);
  for (const g of svg.querySelectorAll<SVGGElement>("g.node")) {
    const id = g.dataset.id!;
    const on = related.has(id) || j.nodes.has(id);
    g.classList.toggle("is-dim", dimming && !on && !hot.has(id));
    g.classList.toggle("is-selected", sel === id);
    g.classList.toggle("is-hover", hover === id);
    g.classList.toggle("is-hot", hot.has(id) && hover !== id);
    g.classList.toggle("is-onpath", j.nodes.has(id));
    g.classList.toggle("is-current-node", j.currentNode === id);
  }
  for (const g of svg.querySelectorAll<SVGGElement>("g.edge")) {
    const key = g.dataset.key!;
    const steps = j.edges.get(key);
    g.classList.toggle("is-dim", dimming && !relatedEdges.has(key) && !steps && !hotEdges.has(key));
    g.classList.toggle("is-hot", hotEdges.has(key));
    g.classList.toggle("is-selected", relSel === key);
    g.classList.toggle("is-journey", !!steps);
    g.classList.toggle("is-current", j.current === key);
    const wire = g.querySelector<SVGPathElement>(".wire")!;
    const e = current.edges.find((x) => x.key === key)!;
    wire.setAttribute("marker-end", `url(#arrow-${steps ? "journey" : e.source})`);
  }
  for (const fo of svg.querySelectorAll<SVGElement>(".edge-label")) {
    const key = fo.dataset.key!;
    fo.classList.toggle("is-dim", dimming && !relatedEdges.has(key) && !j.edges.has(key) && !hotEdges.has(key));
  }
  // numbered marks on the journey's edges
  const marks = svg.querySelector(".journey-marks");
  if (marks) {
    marks.replaceChildren();
    for (const [key, steps] of j.edges) {
      const r = current.lay.edges.find((x) => `${x.from}>${x.to}` === key);
      if (!r) continue;
      const g = el("g", { class: `jmark${j.current === key ? " is-current" : ""}`, transform: `translate(${r.mid.x - 118},${r.mid.y - 22})` });
      g.appendChild(el("circle", { cx: 0, cy: 0, r: 12 }));
      const t = el("text", { x: 0, y: 4, "text-anchor": "middle" });
      t.textContent = steps.length > 2 ? `${steps[0]}+` : steps.join(",");
      g.appendChild(t);
      marks.appendChild(g);
    }
  }
}

// ---- live discovery on the boxes ----------------------------------------------------------

function renderLive(): void {
  if (!current) return;
  const d = store.discovery;
  for (const g of svg.querySelectorAll<SVGGElement>("g.node")) {
    const id = g.dataset.id!;
    const live = g.querySelector<HTMLElement>(".c4-live");
    if (!live) continue;
    const agent = d.agents.find((a) => a.target === id) ?? (id === "s" ? d.agents.find((a) => a.role === "survey") : undefined);
    const reads = d.reads.get(id) ?? (id === "s" ? d.reads.get("survey") : undefined) ?? [];
    const state = !agent ? "" : agent.done ? (agent.ok ? "done" : "failed") : "reading";
    g.dataset.state = state;
    const wasRead = lastReads.get(id) ?? 0;
    if (reads.length > wasRead && state === "reading") {
      g.classList.remove("is-ping");
      void (g as unknown as HTMLElement).offsetWidth;
      g.classList.add("is-ping");
    }
    lastReads.set(id, reads.length);
    if (!agent || !d.running && agent.done) { live.hidden = true; continue; }
    live.hidden = false;
    const ticks = reads.slice(-24).map(() => `<i></i>`).join("");
    live.innerHTML = state === "reading"
      ? `<span class="lamp-dot"></span><span class="live-text">${agent.current ? `reading ${esc(agent.current.split("/").pop() ?? agent.current)}` : esc(agent.name)}</span><span class="ticks">${ticks}</span>`
      : state === "done" ? `<span class="live-text is-done">read ${reads.length} ${reads.length === 1 ? "file" : "files"}</span>`
      : `<span class="live-text is-failed">${esc(agent.error ?? "failed")}</span>`;
  }
}

// ---- breadcrumbs ----------------------------------------------------------------------------

function renderCrumbs(): void {
  const el = $("#crumbs");
  const a = store.atlas;
  if (!a) { el.innerHTML = ""; return; }
  const crumbs: { label: string; level: Level; focus: string | null; testid: string }[] = [{ label: a.system.name, level: "context", focus: null, testid: "crumb-context" }];
  if (store.level !== "context") crumbs.push({ label: "Containers", level: "containers", focus: null, testid: "crumb-containers" });
  if (store.level === "components" || store.level === "code") {
    const c = store.focus ? containerOf(store.focus) : null;
    if (c) crumbs.push({ label: c.name, level: "components", focus: c.id, testid: "crumb-components" });
  }
  if (store.level === "code" && store.focus) crumbs.push({ label: elementName(store.focus), level: "code", focus: store.focus, testid: "crumb-code" });
  el.innerHTML = crumbs.map((c, i) => `${i ? `<span class="crumb-sep">›</span>` : ""}<button class="crumb${i === crumbs.length - 1 ? " is-current" : ""}" data-testid="${c.testid}" data-i="${i}">${esc(c.label)}</button>`).join("");
  el.querySelectorAll<HTMLButtonElement>(".crumb").forEach((b) => b.addEventListener("click", () => { const c = crumbs[Number(b.dataset.i)]; setLevel(c.level, c.focus); }));
  const levelName: Record<Level, string> = { context: "System context", containers: "Containers", components: "Components", code: "Code" };
  void levelName;
  document.querySelectorAll<HTMLButtonElement>("[data-level]").forEach((b) => {
    b.classList.toggle("is-active", b.dataset.level === store.level);
    b.disabled = b.dataset.level === "code" && !(store.selection && elementById(store.selection)?.kind === "component") && store.level !== "code" && !(store.focus && elementById(store.focus)?.kind === "component");
  });
}

// ---- the code level -------------------------------------------------------------------------

async function renderCode(): Promise<void> {
  const el = $("#code-view");
  const e = store.focus ? elementById(store.focus) : null;
  if (!e || e.kind !== "component") { el.innerHTML = `<div class="code-empty"><h2>Pick a component</h2><p>Open a container, then a component, to read its files here.</p></div>`; return; }
  const { container: c, component: k } = e;
  const rels = store.atlas!.relationships.filter((r) => r.level === "component" && (r.from === k.id || r.to === k.id));
  el.innerHTML = `
    <div class="code-head">
      <div class="code-kicker"><span class="dot" data-lang="${langOf(k.technology || c.technology, c.language)}"></span>${esc(c.name)} · component</div>
      <h2 data-testid="code-title">${esc(k.name)}</h2>
      <p class="code-desc">${prose(k.description)}</p>
      ${k.responsibilities?.length ? `<ul class="resp">${k.responsibilities.map((r) => `<li>${esc(r)}</li>`).join("")}</ul>` : ""}
      <div class="code-rels">
        ${rels.filter((r) => r.from === k.id).map((r) => `<button class="rel-chip is-${r.source}" data-el="${esc(r.to)}" title="${esc(r.label)}">→ ${esc(elementName(r.to))} <i>${esc(r.label)}</i></button>`).join("")}
        ${rels.filter((r) => r.to === k.id).map((r) => `<button class="rel-chip is-${r.source}" data-el="${esc(r.from)}" title="${esc(r.label)}">← ${esc(elementName(r.from))} <i>${esc(r.label)}</i></button>`).join("")}
      </div>
    </div>
    <div class="files" data-testid="files">${k.files.map((f) => `<section class="file" data-path="${esc(f)}" data-testid="file-${esc(f)}"><header><span class="file-name">${esc(f.split("/").pop() ?? f)}</span><span class="file-path">${esc(f)}</span><button class="ghost open-file" data-path="${esc(f)}" data-testid="open-${esc(f)}">Open in editor</button></header><div class="symbols"><span class="meta">Loading parts…</span></div></section>`).join("")}</div>`;
  el.querySelectorAll<HTMLButtonElement>(".open-file").forEach((b) => b.addEventListener("click", () => void api.openPath(b.dataset.path!).catch((err) => toast(`Cannot open: ${String(err)}`, "error"))));
  el.querySelectorAll<HTMLButtonElement>(".rel-chip").forEach((b) => b.addEventListener("click", () => { const id = b.dataset.el!; const t = elementById(id); if (t?.kind === "component") setLevel("code", id); else select(id); }));
  for (const f of k.files) {
    let d: NodeDetail;
    try { d = await api.getFile(f); } catch (err) { log("warn", `file detail failed: ${String(err)}`); continue; }
    if (store.focus !== k.id) return;
    const sec = el.querySelector<HTMLElement>(`.file[data-path="${CSS.escape(f)}"] .symbols`);
    if (!sec) continue;
    const tags = (d.node.tags ?? []).filter((t) => t !== "external");
    const ins = d.neighbours.filter((n) => n.direction === "in" && n.kind !== "package");
    const outs = d.neighbours.filter((n) => n.direction === "out" && n.kind !== "package");
    sec.innerHTML = `
      <div class="file-facts"><span>${d.node.loc.toLocaleString()} lines</span><span>${d.children.length} ${d.children.length === 1 ? "part" : "parts"}</span><span>rests on ${outs.filter((x) => x.edge !== "flow").length}</span><span>holds up ${ins.filter((x) => x.edge !== "flow").length}</span>${tags.length ? `<span class="tags">${tags.map((t) => `<span class="chip${t.includes(":") || ["db", "fs", "queue", "http-server", "http-client", "ipc-server", "ipc-client"].includes(t) ? " is-boundary" : ""}">${esc(t)}</span>`).join("")}</span>` : ""}</div>
      ${d.children.length ? `<div class="parts">${d.children.map((s) => `<button class="part-chip" data-lang="${s.lang}" data-line="${s.span?.[0] ?? ""}" title="${esc(s.symbol_kind ?? "symbol")}${s.span ? ` at line ${s.span[0]}` : ""}"><span class="part-kind">${esc(shortKind(s.symbol_kind))}</span>${esc(s.name)}${s.tags?.length ? `<span class="part-tags">${s.tags.filter((t) => t !== "external").map((t) => esc(t)).join(" ")}</span>` : ""}</button>`).join("")}</div>` : `<p class="meta">No named parts.</p>`}`;
    sec.querySelectorAll<HTMLButtonElement>(".part-chip").forEach((b) => b.addEventListener("click", () => void api.openPath(f, b.dataset.line ? Number(b.dataset.line) : undefined).catch((err) => toast(`Cannot open: ${String(err)}`, "error"))));
  }
}

function shortKind(k?: string): string {
  return ({ function: "fn", method: "fn", class: "class", struct: "struct", enum: "enum", trait: "trait", interface: "iface", type: "type", const: "const", module: "mod" } as Record<string, string>)[k ?? ""] ?? "";
}

// ---- pan and zoom --------------------------------------------------------------------------

function applyViewBox(): void {
  svg.setAttribute("viewBox", `${viewBox.x} ${viewBox.y} ${viewBox.w} ${viewBox.h}`);
}

export function fit(animate: boolean): void {
  if (!current) return;
  const { width, height } = current.lay;
  const box = svg.getBoundingClientRect();
  const aspect = box.width / Math.max(1, box.height);
  const pad = 40;
  let w = width + pad * 2;
  let h = height + pad * 2;
  if (w / h < aspect) w = h * aspect; else h = w / aspect;
  // never blow a small diagram up past 1:1
  const scale = Math.min(1, box.width / w);
  if (scale === 1 && box.width > w) { w = box.width; h = box.height; }
  viewBox = { x: (width - w) / 2, y: (height - h) / 2, w, h };
  if (animate) svg.classList.add("is-easing");
  applyViewBox();
  if (animate) setTimeout(() => svg.classList.remove("is-easing"), 300);
}

function initPanZoom(): void {
  let dragging: { x: number; y: number; vx: number; vy: number } | null = null;
  svg.addEventListener("wheel", (e) => {
    e.preventDefault();
    userMoved = true;
    const box = svg.getBoundingClientRect();
    if (e.ctrlKey || e.metaKey) {
      const factor = Math.exp(e.deltaY * 0.01);
      const px = viewBox.x + ((e.clientX - box.left) / box.width) * viewBox.w;
      const py = viewBox.y + ((e.clientY - box.top) / box.height) * viewBox.h;
      const w = Math.max(200, Math.min(8000, viewBox.w * factor));
      const h = w * (viewBox.h / viewBox.w);
      viewBox = { x: px - (px - viewBox.x) * (w / viewBox.w), y: py - (py - viewBox.y) * (h / viewBox.h), w, h };
    } else {
      const k = viewBox.w / box.width;
      viewBox = { ...viewBox, x: viewBox.x + e.deltaX * k, y: viewBox.y + e.deltaY * k };
    }
    applyViewBox();
  }, { passive: false });
  svg.addEventListener("mousedown", (e) => { if (e.button === 0) dragging = { x: e.clientX, y: e.clientY, vx: viewBox.x, vy: viewBox.y }; });
  window.addEventListener("mousemove", (e) => {
    if (!dragging) return;
    const box = svg.getBoundingClientRect();
    const k = viewBox.w / box.width;
    const dx = e.clientX - dragging.x;
    const dy = e.clientY - dragging.y;
    if (Math.abs(dx) + Math.abs(dy) > 3) { svg.classList.add("is-dragging"); userMoved = true; }
    viewBox = { ...viewBox, x: dragging.vx - dx * k, y: dragging.vy - dy * k };
    applyViewBox();
  });
  window.addEventListener("mouseup", () => { dragging = null; setTimeout(() => svg.classList.remove("is-dragging"), 0); });
  svg.addEventListener("click", () => { if (!svg.classList.contains("is-dragging")) { select(null); } });
  svg.addEventListener("dblclick", (e) => { if (e.target === svg || (e.target as Element).classList.contains("plate")) { if (store.level !== "context") zoomOut(); else fit(true); } });
  window.addEventListener("resize", () => fit(false));
  document.getElementById("shelf")?.addEventListener("transitionend", () => fit(false));
  // panels opening and closing resize the stage; keep the diagram framed
  new ResizeObserver(() => { if (!userMoved) fit(false); }).observe($("#stage"));
}

/** The current diagram as a standalone SVG string (for export). */
export function exportSvg(): string {
  const clone = svg.cloneNode(true) as SVGSVGElement;
  clone.removeAttribute("class");
  clone.setAttribute("xmlns", SVG);
  const css = [...document.styleSheets].flatMap((s) => { try { return [...s.cssRules].map((r) => r.cssText); } catch { return []; } }).join("\n");
  const style = document.createElementNS(SVG, "style");
  style.textContent = css;
  clone.insertBefore(style, clone.firstChild);
  return new XMLSerializer().serializeToString(clone);
}

export function rerender(): void {
  render();
  renderCrumbs();
  emit("ui");
}
