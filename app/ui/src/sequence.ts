// The sequence view: one journey as lifelines and arrows, at the current level of
// the atlas, drawn from the same messages the map numbers. The participants are the
// atlas's own boxes (a person, the system, a container, a component, an outside
// system), so what this shows is exactly what the C4 diagrams show, one level at a
// time. Also the editor: a person changes the messages by hand, or leaves a note
// and has a narrator write them again.

import { api, log } from "./tauri";
import { store, subscribe, emit, select, setJourney, setJourneyStep, setView, startDraft, endDraft, setAtlas, currentProjection, participantChoices, elementById, elementName, containerOf, zoomInto, type PMessage, type Level } from "./store";
import { EXTERNAL_LABEL, KIND_LABEL, langOf, type Journey, type Message, type Lang } from "./types";
import { esc, prose, toast } from "./panels";

const SVG = "http://www.w3.org/2000/svg";
const XHTML = "http://www.w3.org/1999/xhtml";
const $ = <T extends HTMLElement>(sel: string): T => document.querySelector(sel) as T;

const BOX_W = 152;
const BOX_H = 58;
const GAP = 38;
const LEFT = 64;
const TOP = 16;
const ROW = 48;
const BOTTOM = 34;

let root: HTMLElement;
let svg: SVGSVGElement;
let editorKey = "";

export function initSequence(): void {
  root = $("#sequence");
  svg = root.querySelector("svg") as unknown as SVGSVGElement;
  subscribe("journey", render);
  subscribe("level", render);
  subscribe("atlas", render);
  subscribe("selection", applyEmphasis);
  subscribe("hover", applyEmphasis);
  document.querySelectorAll<HTMLButtonElement>("[data-view]").forEach((b) => b.addEventListener("click", () => setView(b.dataset.view as "map" | "sequence")));
  render();
}

/** Is the sequence view what the stage should show right now? */
export function sequenceShown(): boolean {
  return store.journey !== null && store.view === "sequence";
}

function shownJourney(): Journey | null {
  return store.draft ?? store.journey;
}

// ---- rendering --------------------------------------------------------------------------

function render(): void {
  const j = shownJourney();
  const show = sequenceShown() && j !== null;
  root.hidden = !show;
  document.body.classList.toggle("has-sequence", show);
  const toggle = $("#view-toggle");
  toggle.hidden = store.journey === null;
  toggle.querySelectorAll<HTMLButtonElement>("[data-view]").forEach((b) => b.classList.toggle("is-active", b.dataset.view === store.view));
  if (!show || !j) { editorKey = ""; return; }
  renderHead(j);
  renderSvg();
  renderEditor(j);
}

function levelName(level: Level): string {
  return level === "context" ? "system context" : level === "containers" ? "containers" : `components of ${store.focus ? elementName(containerOf(store.focus)?.id ?? store.focus) : "…"}`;
}

function renderHead(j: Journey): void {
  const head = root.querySelector<HTMLElement>(".seq-head")!;
  const p = currentProjection();
  const claimed = j.messages.filter((m) => m.source === "claimed").length;
  const yours = j.messages.filter((m) => m.by === "user").length;
  const narrating = store.narrating === j.id;
  const editing = store.draft !== null;
  head.innerHTML = `
    <div class="seq-kicker">
      <span>Sequence · ${esc(levelName(store.level))} · ${p?.messages.length ?? 0} ${p?.messages.length === 1 ? "arrow" : "arrows"} of ${j.messages.length}</span>
      ${j.source === "user" ? `<span class="chip is-user" data-testid="seq-source">edited by you</span>` : j.source === "claude" ? `<span class="chip is-claude" data-testid="seq-source">narrated by Claude</span>` : `<span class="chip" data-testid="seq-source">engine draft</span>`}
      ${claimed ? `<span class="chip is-claimed" title="Messages the code does not show; drawn dashed">${claimed} claimed</span>` : ""}
      ${yours && j.source !== "user" ? `<span class="chip is-user">${yours} yours</span>` : ""}
      ${narrating ? `<span class="chip is-busy"><span class="spinner is-small"></span> a narrator is rewriting this</span>` : ""}
    </div>
    <h2 data-testid="seq-title">${esc(j.name || "Untitled journey")}</h2>
    ${j.summary ? `<p class="seq-summary">${prose(j.summary)}</p>` : ""}
    ${j.why ? `<p class="seq-why" data-testid="seq-why" title="Why the scout chose this flow">Why it matters: ${esc(j.why)}</p>` : ""}
    ${j.note ? `<p class="seq-note" title="What you asked the narrator to pay attention to">✎ ${esc(j.note)}</p>` : ""}
    <div class="seq-actions">
      ${editing ? "" : `<button class="ghost" data-testid="seq-edit" ${narrating ? "disabled" : ""}>Edit</button>`}
      ${editing ? "" : `<button class="ghost" data-testid="seq-narrate" ${narrating || !store.graphLoaded ? "disabled" : ""} title="One narrator agent rewrites this journey with your note (spends money)">Narrate again…</button>`}
      <button class="ghost" data-testid="seq-mermaid" title="Copy this level as a Mermaid sequence diagram">Copy Mermaid</button>
      <button class="ghost" data-testid="seq-map" title="Show the same journey numbered on the C4 map">On the map</button>
    </div>
    <div class="seq-ask" data-testid="seq-ask" hidden>
      <input data-testid="seq-note" type="text" placeholder="What should the narrator pay attention to?" value="${esc(j.note ?? "")}" />
      <button class="primary" data-testid="seq-narrate-go">Narrate</button>
      <button class="ghost" data-testid="seq-narrate-cancel">Cancel</button>
    </div>`;
  head.querySelector("[data-testid=seq-edit]")?.addEventListener("click", () => startDraft());
  head.querySelector("[data-testid=seq-map]")?.addEventListener("click", () => setView("map"));
  head.querySelector("[data-testid=seq-mermaid]")?.addEventListener("click", () => void copyMermaid(j));
  const ask = head.querySelector<HTMLElement>("[data-testid=seq-ask]")!;
  head.querySelector("[data-testid=seq-narrate]")?.addEventListener("click", () => { ask.hidden = false; ask.querySelector<HTMLInputElement>("input")!.focus(); });
  head.querySelector("[data-testid=seq-narrate-cancel]")?.addEventListener("click", () => { ask.hidden = true; });
  const go = () => void narrateAgain(j.id, ask.querySelector<HTMLInputElement>("input")!.value);
  head.querySelector("[data-testid=seq-narrate-go]")?.addEventListener("click", go);
  ask.querySelector<HTMLInputElement>("input")!.addEventListener("keydown", (e) => { if (e.key === "Enter") go(); if (e.key === "Escape") ask.hidden = true; });
}

type PKind = "person" | "system" | "external" | "container" | "component";

function kindOf(id: string): { kind: PKind; sub: string; lang: Lang; externalKind?: string } {
  if (id === "s") return { kind: "system", sub: "Software system", lang: "other" };
  const e = elementById(id);
  if (!e) return { kind: "component", sub: "", lang: "other" };
  switch (e.kind) {
    case "person": return { kind: "person", sub: "Person", lang: "other" };
    case "external": return { kind: "external", sub: EXTERNAL_LABEL[e.external.kind] ?? "Outside", lang: "other", externalKind: e.external.kind };
    case "container": return { kind: "container", sub: KIND_LABEL[e.container.kind] ?? "Container", lang: langOf(e.container.technology, e.container.language) };
    case "component": return { kind: "component", sub: `in ${e.container.name}`, lang: langOf(e.component.technology || e.container.technology, e.container.language) };
    default: return { kind: "system", sub: "Software system", lang: "other" };
  }
}

function el(tag: string, attrs: Record<string, string | number> = {}): SVGElement {
  const e = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
  return e;
}

/** How far an activation started by message k reaches: through every deeper message that follows, and its return. */
function activationEnd(msgs: PMessage[], k: number): number {
  const m = msgs[k];
  let e = k;
  while (e + 1 < msgs.length && (msgs[e + 1].depth > m.depth || (msgs[e + 1].kind === "return" && msgs[e + 1].from === m.to && msgs[e + 1].to === m.from))) {
    e++;
    if (msgs[e].kind === "return" && msgs[e].from === m.to && msgs[e].to === m.from) break;
  }
  return e;
}

function renderSvg(): void {
  const p = currentProjection();
  if (!p) { svg.replaceChildren(); return; }
  const n = p.participants.length;
  const width = Math.max(320, LEFT + n * (BOX_W + GAP) + 24);
  const height = TOP + BOX_H + 30 + p.messages.length * ROW + BOTTOM;
  svg.setAttribute("width", String(width));
  svg.setAttribute("height", String(height));
  svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
  const cx = new Map<string, number>();
  p.participants.forEach((id, i) => cx.set(id, LEFT + i * (BOX_W + GAP) + BOX_W / 2));
  const y = (k: number) => TOP + BOX_H + 30 + k * ROW;

  const defs = el("defs");
  for (const kind of ["code", "survey", "claimed", "return"]) {
    const m = el("marker", { id: `seq-arrow-${kind}`, viewBox: "0 0 10 10", refX: 9, refY: 5, markerWidth: 8, markerHeight: 8, orient: "auto", class: `seq-arrow is-${kind}` });
    m.appendChild(el("path", { d: kind === "return" ? "M0,1 L9,5 L0,9" : "M0,0.5 L10,5 L0,9.5 z" }));
    defs.appendChild(m);
  }
  const gLines = el("g", { class: "lifelines" });
  const gActs = el("g", { class: "activations" });
  const gMsgs = el("g", { class: "messages" });
  const gParts = el("g", { class: "participants" });

  for (const id of p.participants) {
    const x = cx.get(id)!;
    gLines.appendChild(el("line", { class: "lifeline", x1: x, y1: TOP + BOX_H, x2: x, y2: height - BOTTOM + 10, "data-id": id }));
    const k = kindOf(id);
    const g = el("g", { class: `seq-part is-${k.kind}`, transform: `translate(${x - BOX_W / 2},${TOP})`, "data-id": id, "data-lang": k.lang, "data-testid": `seq-${id}` }) as SVGGElement;
    if (k.kind === "person") {
      g.appendChild(el("circle", { class: "frame head", cx: BOX_W / 2, cy: 12, r: 11 }));
      g.appendChild(el("rect", { class: "frame body", x: 0, y: 22, width: BOX_W, height: BOX_H - 22, rx: 12 }));
    } else if (k.externalKind === "database") {
      const r = 7;
      g.appendChild(el("path", { class: "frame", d: `M0,${r} a${BOX_W / 2},${r} 0 0 1 ${BOX_W},0 v${BOX_H - 2 * r} a${BOX_W / 2},${r} 0 0 1 -${BOX_W},0 z` }));
      g.appendChild(el("path", { class: "frame-line", d: `M0,${r} a${BOX_W / 2},${r} 0 0 0 ${BOX_W},0` }));
    } else {
      g.appendChild(el("rect", { class: "frame", x: 0, y: 0, width: BOX_W, height: BOX_H, rx: k.kind === "system" ? 16 : 12 }));
      if (k.kind === "container" || k.kind === "component") g.appendChild(el("rect", { class: "tint", x: 0, y: 0, width: 4, height: BOX_H, rx: 2 }));
    }
    const fo = el("foreignObject", { x: 0, y: k.kind === "person" ? 22 : 0, width: BOX_W, height: k.kind === "person" ? BOX_H - 22 : BOX_H });
    const div = document.createElementNS(XHTML, "div");
    div.className = `c4 c4-seq c4-seq-${k.kind}`;
    div.innerHTML = `<div class="c4-title">${esc(elementName(id))}</div>${k.sub ? `<div class="c4-sub">${esc(k.sub)}</div>` : ""}`;
    fo.appendChild(div);
    g.appendChild(fo);
    g.appendChild(el("rect", { class: "ring", x: -4, y: -4, width: BOX_W + 8, height: BOX_H + 8, rx: 16 }));
    g.addEventListener("click", (ev) => { ev.stopPropagation(); select(id); });
    g.addEventListener("dblclick", (ev) => { ev.stopPropagation(); if (id !== "s") zoomInto(id); });
    gParts.appendChild(g);
  }

  p.messages.forEach((m, k) => {
    if (m.kind === "call" || m.kind === "flow") {
      const e = activationEnd(p.messages, k);
      const x = cx.get(m.to)!;
      gActs.appendChild(el("rect", { class: "activation", x: x - 5, y: y(k) - 2, width: 10, height: y(e) - y(k) + (e > k ? 8 : 14), rx: 2, "data-n": k + 1 }));
    }
  });

  p.messages.forEach((m, k) => {
    const x1 = cx.get(m.from)!;
    const x2 = cx.get(m.to)!;
    const yy = y(k);
    const dir = x2 > x1 ? 1 : -1;
    const end = x2 - dir * 6;
    const src = m.kind === "return" ? "return" : m.source === "claimed" ? "claimed" : m.source === "survey" ? "survey" : "code";
    const g = el("g", { class: `seq-msg is-${m.kind} is-src-${m.source || "code"} by-${m.by || "engine"}`, "data-n": k + 1, "data-testid": `msg-${k + 1}` });
    const title = el("title");
    title.textContent = `${k + 1}. ${elementName(m.from)} → ${elementName(m.to)}: ${m.label}${m.caption ? `\n${m.caption}` : ""}`;
    g.appendChild(title);
    g.appendChild(el("line", { class: "hit", x1: Math.min(x1, x2), y1: yy, x2: Math.max(x1, x2), y2: yy }));
    g.appendChild(el("line", { class: "wire", x1, y1: yy, x2: end, y2: yy, "marker-end": `url(#seq-arrow-${src})` }));
    const t = el("text", { class: "label", x: (x1 + x2) / 2, y: yy - 7, "text-anchor": "middle" });
    t.textContent = m.label.length > 38 ? `${m.label.slice(0, 37)}…` : m.label;
    g.appendChild(t);
    if (m.source === "claimed" || m.by === "user") {
      const b = el("text", { class: `badge is-${m.source === "claimed" ? "claimed" : "user"}`, x: (x1 + x2) / 2, y: yy + 13, "text-anchor": "middle" });
      b.textContent = m.source === "claimed" ? (m.by === "user" ? "yours, not in code" : "claimed") : "yours";
      g.appendChild(b);
    }
    const num = el("g", { class: "seq-num", transform: `translate(${LEFT - 34},${yy})` });
    num.appendChild(el("circle", { r: 11 }));
    const nt = el("text", { y: 4, "text-anchor": "middle" });
    nt.textContent = String(k + 1);
    num.appendChild(nt);
    g.appendChild(num);
    g.addEventListener("click", (ev) => { ev.stopPropagation(); setJourneyStep(store.journeyStep === k + 1 ? 0 : k + 1); });
    gMsgs.appendChild(g);
  });

  svg.replaceChildren(defs, gLines, gActs, gMsgs, gParts);
  applyEmphasis();
}

function applyEmphasis(): void {
  if (root.hidden) return;
  const p = currentProjection();
  if (!p) return;
  const step = store.journeyStep;
  const reached = new Set<string>();
  p.messages.forEach((m, k) => { if (step === 0 || k + 1 <= step) { reached.add(m.from); reached.add(m.to); } });
  for (const g of svg.querySelectorAll<SVGGElement>("g.seq-msg")) {
    const n = Number(g.dataset.n);
    g.classList.toggle("is-future", step > 0 && n > step);
    g.classList.toggle("is-current", step === n);
  }
  for (const r of svg.querySelectorAll<SVGRectElement>("rect.activation")) {
    const n = Number(r.dataset.n);
    r.classList.toggle("is-future", step > 0 && n > step);
  }
  for (const g of svg.querySelectorAll<SVGGElement>("g.seq-part")) {
    const id = g.dataset.id!;
    g.classList.toggle("is-dim", step > 0 && !reached.has(id));
    g.classList.toggle("is-selected", store.selection === id);
    g.classList.toggle("is-hover", store.hover === id);
  }
  // keep the current arrow in view
  if (step > 0) svg.querySelector(`g.seq-msg[data-n="${step}"]`)?.scrollIntoView({ block: "nearest", inline: "nearest" });
}

// ---- mermaid ------------------------------------------------------------------------------

function mermaidId(id: string): string {
  const s = id.replace(/[^A-Za-z0-9]/g, "_");
  return /^\d/.test(s) ? `_${s}` : s;
}

export function toMermaid(j: Journey): string {
  const p = currentProjection() ?? { participants: [], messages: [] };
  const lines = ["sequenceDiagram", `    %% ${j.name}: ${j.summary}`];
  for (const id of p.participants) lines.push(`    ${id.startsWith("p:") ? "actor" : "participant"} ${mermaidId(id)} as ${elementName(id).replace(/"/g, "'")}`);
  for (const m of p.messages) lines.push(`    ${mermaidId(m.from)} ${m.kind === "return" ? "-->>" : "->>"} ${mermaidId(m.to)}: ${m.label.replace(/:/g, " -")}${m.source === "claimed" ? " (claimed)" : ""}`);
  return lines.join("\n") + "\n";
}

async function copyMermaid(j: Journey): Promise<void> {
  try {
    await navigator.clipboard.writeText(toMermaid(j));
    toast("Copied this level as a Mermaid sequence diagram", "ok");
  } catch (e) { toast(`Cannot copy: ${String(e)}`, "error"); }
}

// ---- narrate again ----------------------------------------------------------------------

async function narrateAgain(id: string, note: string): Promise<void> {
  if (store.narrating) return;
  store.narrating = id;
  emit("journey");
  try {
    await api.narrateJourney(id, note);
  } catch (e) {
    log("warn", `narrate failed: ${String(e)}`);
  }
}

// ---- the editor ----------------------------------------------------------------------------

const KINDS: Message["kind"][] = ["call", "flow", "store", "return"];

function editorKeyOf(d: Journey): string {
  return `${d.id}|${d.messages.map((m) => `${m.from}>${m.to}:${m.kind}`).join("|")}`;
}

function renderEditor(_j: Journey): void {
  const ed = root.querySelector<HTMLElement>(".seq-editor")!;
  const d = store.draft;
  ed.hidden = d === null;
  if (!d) { editorKey = ""; ed.innerHTML = ""; return; }
  const key = editorKeyOf(d);
  if (key === editorKey) return; // words changed, not shape: leave the inputs alone
  editorKey = key;
  const choices = participantChoices();
  const options = (sel: string) => {
    const groups = [...new Set(choices.map((c) => c.group))];
    return `<option value="" ${sel === "" ? "selected" : ""}>—</option>` + groups.map((g) => `<optgroup label="${esc(g)}">${choices.filter((c) => c.group === g).map((c) => `<option value="${esc(c.id)}" ${c.id === sel ? "selected" : ""}>${esc(c.name)}</option>`).join("")}</optgroup>`).join("");
  };
  ed.innerHTML = `
    <div class="ed-head"><h3>Edit this journey</h3><span class="meta">Arrows join the atlas's own elements, so the diagrams stay in step. The engine checks every message you save.</span></div>
    <label class="ed-field"><span>Name</span><input data-testid="ed-name" data-field="name" type="text" value="${esc(d.name)}" placeholder="What the user is doing" /></label>
    <label class="ed-field"><span>Summary</span><textarea data-testid="ed-summary" data-field="summary" rows="2" placeholder="Two sentences from the user's point of view">${esc(d.summary)}</textarea></label>
    <label class="ed-field"><span>Note for the narrator</span><input data-testid="ed-note" data-field="note" type="text" value="${esc(d.note ?? "")}" placeholder="What to pay attention to, in your words" /></label>
    <div class="ed-messages" data-testid="ed-messages">
      ${d.messages.map((m, i) => `
      <div class="ed-msg is-src-${esc(m.source || "")}" data-i="${i}" data-testid="ed-msg-${i + 1}">
        <div class="ed-row">
          <span class="ed-n">${i + 1}</span>
          <select data-i="${i}" data-field="from" data-testid="ed-msg-${i + 1}-from" title="From">${options(m.from)}</select>
          <span class="ed-arrow">→</span>
          <select data-i="${i}" data-field="to" data-testid="ed-msg-${i + 1}-to" title="To">${options(m.to)}</select>
          <select data-i="${i}" data-field="kind" data-testid="ed-msg-${i + 1}-kind" title="Kind">${KINDS.map((k) => `<option value="${k}" ${m.kind === k ? "selected" : ""}>${k}</option>`).join("")}</select>
        </div>
        <div class="ed-row">
          <input data-i="${i}" data-field="label" data-testid="ed-msg-${i + 1}-label" type="text" value="${esc(m.label)}" placeholder="Words on the arrow" />
          <button class="ghost is-round" data-move="-1" data-i="${i}" title="Move up" ${i === 0 ? "disabled" : ""}>↑</button>
          <button class="ghost is-round" data-move="1" data-i="${i}" title="Move down" ${i === d.messages.length - 1 ? "disabled" : ""}>↓</button>
          <button class="ghost is-round" data-remove="${i}" data-testid="ed-msg-${i + 1}-remove" title="Remove">×</button>
        </div>
        <input class="ed-caption" data-i="${i}" data-field="caption" data-testid="ed-msg-${i + 1}-caption" type="text" value="${esc(m.caption)}" placeholder="One sentence: what happens and what is carried across" />
        ${m.source === "claimed" ? `<span class="ed-flag">not seen in the code</span>` : m.by === "user" ? `<span class="ed-flag is-user">yours</span>` : ""}
      </div>`).join("")}
      <button class="ghost ed-add" data-testid="ed-add">+ Add a message</button>
    </div>
    <div class="ed-foot">
      <button class="primary" data-testid="ed-save">Save</button>
      <button class="ghost" data-testid="ed-cancel">Cancel</button>
      ${d.id ? `<button class="ghost is-danger" data-testid="ed-delete" title="Remove this journey from the atlas">Delete</button>` : ""}
    </div>`;
  const redraw = () => { renderSvg(); renderHead(d); emit("ui"); };
  ed.querySelectorAll<HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement>("[data-field]").forEach((inp) => {
    inp.addEventListener("input", () => {
      const f = inp.dataset.field!;
      if (inp.dataset.i === undefined) { (d as unknown as Record<string, string>)[f] = inp.value; renderHead(d); return; }
      const m = d.messages[Number(inp.dataset.i)];
      if (!m) return;
      if (f === "kind") m.kind = inp.value as Message["kind"];
      else (m as unknown as Record<string, string>)[f] = inp.value;
      if (f === "from" || f === "to" || f === "kind") { m.by = "user"; editorKey = ""; emit("journey"); return; }
      m.by = "user";
      redraw();
    });
  });
  ed.querySelectorAll<HTMLButtonElement>("[data-move]").forEach((b) => b.addEventListener("click", () => {
    const i = Number(b.dataset.i);
    const k = i + Number(b.dataset.move);
    if (k < 0 || k >= d.messages.length) return;
    [d.messages[i], d.messages[k]] = [d.messages[k], d.messages[i]];
    emit("journey");
  }));
  ed.querySelectorAll<HTMLButtonElement>("[data-remove]").forEach((b) => b.addEventListener("click", () => { d.messages.splice(Number(b.dataset.remove), 1); emit("journey"); }));
  ed.querySelector("[data-testid=ed-add]")!.addEventListener("click", () => {
    const last = d.messages[d.messages.length - 1];
    d.messages.push({ from: last?.to ?? "", to: "", label: "", caption: "", kind: "call", depth: last?.depth ?? 0, source: "", by: "user" });
    emit("journey");
    setTimeout(() => ed.querySelector<HTMLSelectElement>(`[data-testid="ed-msg-${d.messages.length}-to"]`)?.focus(), 0);
  });
  ed.querySelector("[data-testid=ed-save]")!.addEventListener("click", () => void saveDraft());
  ed.querySelector("[data-testid=ed-cancel]")!.addEventListener("click", () => { const wasNew = !d.id; endDraft(); if (wasNew) setJourney(null); });
  ed.querySelector("[data-testid=ed-delete]")?.addEventListener("click", () => void deleteJourney(d.id));
}

/** The core's slug rule, so a new journey's id can be found once it is saved. */
function slug(s: string): string {
  const t = s.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
  return t || "x";
}

export async function saveDraft(): Promise<void> {
  const d = store.draft;
  if (!d) return;
  const msgs = d.messages.filter((m) => m.from && m.to && m.from !== m.to);
  if (!msgs.length) { toast("A journey needs at least one message between two different elements", "info"); return; }
  const j: Journey = { ...d, messages: msgs, name: d.name.trim() || "Untitled journey", id: d.id || `j:${slug(d.entry || d.name.trim() || "untitled")}` };
  try {
    const v = await api.saveJourney(j);
    store.draft = null;
    setAtlas(v.atlas, v.stale ?? null);
    const saved = v.atlas.journeys.find((x) => x.id === j.id);
    if (saved) { store.journey = saved; store.view = "sequence"; emit("journey"); }
    const dropped = (v.atlas.report.notes ?? []).find((n) => n.startsWith(`\`${j.name}\``));
    toast(dropped ? `Saved. ${dropped}` : `Saved “${j.name}”; the engine checked ${msgs.length} ${msgs.length === 1 ? "message" : "messages"}`, dropped ? "info" : "ok", 5000);
    log("info", "journey saved", { id: j.id, messages: msgs.length });
  } catch (e) { toast(`Cannot save: ${String(e)}`, "error"); }
}

async function deleteJourney(id: string): Promise<void> {
  try {
    const v = await api.deleteJourney(id);
    store.draft = null;
    setJourney(null);
    setAtlas(v.atlas, v.stale ?? null);
    toast("Journey removed", "ok");
  } catch (e) { toast(`Cannot delete: ${String(e)}`, "error"); }
}

/** A blank journey of the person's own, opened in the editor. */
export function newJourney(): void {
  const j: Journey = { id: "", name: "", summary: "", entry: "", messages: [], steps: [], source: "user", note: "" };
  startDraft(j);
  setTimeout(() => root.querySelector<HTMLInputElement>("[data-testid=ed-name]")?.focus(), 0);
}

/** What an agent needs to see of the sequence view without a screenshot. */
export function sequenceSnapshot(): Record<string, unknown> | null {
  if (!sequenceShown()) return null;
  const p = currentProjection();
  if (!p) return null;
  return {
    participants: p.participants.map((id) => ({ id, name: elementName(id) })),
    messages: p.messages.map((m, k) => ({ n: k + 1, from: m.from, to: m.to, kind: m.kind, label: m.label, source: m.source, by: m.by, current: store.journeyStep === k + 1 || undefined })),
    editing: store.draft !== null,
  };
}
