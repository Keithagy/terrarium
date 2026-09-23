// DOM panels: header, shelf (search, map, journeys, guide), the annotation card,
// the journey bar, overlays and toasts.

import { api, log } from "./tauri";
import { store, subscribe, emit, select, selectRelationship, setLevel, setJourney, setJourneyStep, setView, startDraft, currentProjection, journeyLength, zoomInto, elementById, elementName, shownContainers, containerOf, relationshipsOf, relKey, componentOfFile, type ShelfTab } from "./store";
import { EXTERNAL_LABEL, KIND_LABEL, langOf, type Journey, type Relationship } from "./types";
import { newJourney } from "./sequence";
import { openPlan } from "./discovery";

const $ = <T extends HTMLElement>(sel: string): T => document.querySelector(sel) as T;

export interface Actions {
  openRepo(path?: string): Promise<void>;
  copyDsl(): Promise<void>;
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
  initJourneyBar();
  subscribe("atlas", () => { renderChips(); renderMap(); renderJourneys(); renderGuide(); });
  subscribe("level", () => { renderMap(); });
  subscribe("selection", () => { renderCard(); renderMap(); });
  subscribe("journey", () => { renderJourneys(); renderJourneyBar(); });
  subscribe("discovery", () => { renderCard(); renderJourneys(); });
  subscribe("repo", () => { renderChips(); renderOverlays(); });
  subscribe("ui", () => { renderChips(); renderOverlays(); renderBridge(); });
  renderOverlays();
}

// ---- header -------------------------------------------------------------------

function initHeader(): void {
  $("#repo-name").addEventListener("click", () => void actions.openRepo());
  $("#dsl-btn").addEventListener("click", () => void actions.copyDsl());
}

function renderChips(): void {
  const repo = $("#repo-name");
  const a = store.atlas;
  repo.textContent = a ? a.system.name : store.repo ? store.repo.split("/").filter(Boolean).pop() ?? store.repo : "No repository";
  repo.title = store.repo ? `${store.repo}\nClick to open another repository` : "Open a repository";
  const el = $("#chips");
  el.innerHTML = "";
  if (!a) return;
  const chip = (text: string, testid: string, title: string, cls = "") => {
    const s = document.createElement("span");
    s.className = `hchip ${cls}`;
    s.dataset.testid = testid;
    s.title = title;
    s.innerHTML = text;
    el.appendChild(s);
  };
  const cs = shownContainers();
  const comps = cs.reduce((n, c) => n + c.components.length, 0);
  chip(`<b>${cs.length}</b> containers`, "chip-containers", "The things that run: web apps, services, workers, command lines, libraries");
  chip(`<b>${comps}</b> components`, "chip-components", "The parts inside the containers");
  chip(`<b>${a.relationships.length}</b> relationships`, "chip-relationships", "Arrows on the diagrams");
  chip(`<b>${a.report.backed}</b> backed by code`, "chip-backed", "Relationships the engine found in imports, calls and flows; click one to see the evidence", "is-ok");
  if (a.report.claimed) chip(`<b>${a.report.claimed}</b> claimed`, "chip-claimed", "Relationships an agent asserted that the code does not show; drawn dashed", "is-weak");
  chip(a.source === "engine" ? "engine draft" : `discovered by ${esc(a.model ?? "claude")}`, "chip-source", a.source === "engine" ? "Names come from folders and manifests. Discover with Claude to have agents read the code." : `Agents read the code and named every part; the engine checked every relationship${store.stale ? `\n${store.stale}` : ""}`, a.source === "engine" ? "is-quiet" : "is-claude");
}

// ---- shelf --------------------------------------------------------------------

function initShelf(): void {
  const input = $<HTMLInputElement>("#search");
  const results = $<HTMLUListElement>("#search-results");
  let active = -1;
  let hits: { id: string; name: string; path: string; kind: string; lang: string }[] = [];
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
      const q = input.value.trim().toLowerCase();
      // Atlas elements first (by name), then files and symbols from the graph.
      const local: typeof hits = [];
      const a = store.atlas;
      if (a) {
        for (const c of shownContainers()) {
          if (c.name.toLowerCase().includes(q) || c.package.toLowerCase().includes(q)) local.push({ id: c.id, name: c.name, path: `container · ${c.package}`, kind: "container", lang: langOf(c.technology, c.language) });
          for (const k of c.components) if (k.name.toLowerCase().includes(q)) local.push({ id: k.id, name: k.name, path: `component in ${c.name}`, kind: "component", lang: langOf(k.technology || c.technology, c.language) });
        }
        for (const x of a.externals) if (x.name.toLowerCase().includes(q)) local.push({ id: x.id, name: x.name, path: EXTERNAL_LABEL[x.kind], kind: "external", lang: "other" });
        for (const j of a.journeys) if (j.name.toLowerCase().includes(q)) local.push({ id: j.id, name: j.name, path: `journey from ${j.entry}`, kind: "journey", lang: "other" });
      }
      try {
        const remote = (await api.search(input.value, 20)).filter((h) => h.kind === "file" || h.kind === "symbol").map((h) => ({ id: `path:${h.path}`, name: h.name, path: h.path, kind: h.kind, lang: h.lang }));
        hits = [...local.slice(0, 6), ...remote].slice(0, 12);
      } catch (e) { log("warn", `search failed: ${String(e)}`); hits = local.slice(0, 12); }
      active = hits.length ? 0 : -1;
      render();
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
    tab.addEventListener("click", () => setShelfTab(tab.dataset.tab as ShelfTab));
  });
}

export function setShelfTab(t: ShelfTab): void {
  document.querySelectorAll(".tab").forEach((x) => x.classList.toggle("is-active", (x as HTMLElement).dataset.tab === t));
  document.querySelectorAll<HTMLElement>(".tab-panel").forEach((p) => p.classList.toggle("is-active", p.dataset.tabPanel === t));
  store.shelfTab = t;
  emit("ui");
}

/** Show an element on the right diagram and select it. Accepts ids, `path:<file>` and journey ids. */
export function jumpTo(id: string): void {
  if (id.startsWith("path:")) {
    const k = componentOfFile(id.slice(5));
    if (!k) { toast("That file is not in any component", "info"); return; }
    setLevel("code", k);
    select(k);
    return;
  }
  if (id.startsWith("j:")) {
    const j = store.atlas?.journeys.find((x) => x.id === id);
    if (j) playJourney(j);
    return;
  }
  const e = elementById(id);
  if (!e) return;
  if (e.kind === "component") setLevel("components", e.container.id);
  else if (e.kind === "container") { if (store.level !== "containers" && !(store.level === "components" && store.focus === e.container.id)) setLevel("containers"); }
  else if (e.kind === "system") setLevel("context");
  else if (store.level === "code") setLevel("containers");
  select(id === "s" ? "s" : id);
}

const openContainers = new Set<string>();

function renderMap(): void {
  const el = $("[data-tab-panel=map]");
  const a = store.atlas;
  el.innerHTML = "";
  if (!a) return;
  const frag = document.createDocumentFragment();
  const sys = document.createElement("button");
  sys.className = `map-row is-system${store.selection === "s" ? " is-selected" : ""}`;
  sys.dataset.testid = "map-system";
  sys.innerHTML = `<span class="name">${esc(a.system.name)}</span><span class="meta">system context</span>`;
  sys.addEventListener("click", () => { setLevel("context"); select("s"); });
  frag.appendChild(sys);
  for (const c of shownContainers()) {
    const open = openContainers.has(c.id) || store.focus === c.id || (store.selection ? containerOf(store.selection)?.id === c.id : false);
    const head = document.createElement("button");
    head.className = `map-row is-container${open ? " is-open" : ""}${store.selection === c.id ? " is-selected" : ""}${store.focus === c.id ? " is-focus" : ""}`;
    head.dataset.testid = `map-${c.id}`;
    head.innerHTML = `<span class="dot" data-lang="${langOf(c.technology, c.language)}"></span><span class="name">${esc(c.name)}</span><span class="meta">${esc(KIND_LABEL[c.kind] ?? c.kind)}</span>`;
    head.addEventListener("click", () => {
      if (open && store.focus === c.id) { openContainers.delete(c.id); }
      else openContainers.add(c.id);
      jumpTo(c.id);
      renderMap();
    });
    frag.appendChild(head);
    if (!open) continue;
    for (const k of c.components) {
      const r = document.createElement("button");
      r.className = `map-row is-component${store.selection === k.id ? " is-selected" : ""}${store.focus === k.id ? " is-focus" : ""}`;
      r.dataset.testid = `map-${k.id}`;
      r.title = k.files.join("\n");
      r.innerHTML = `<span class="name">${esc(k.name)}</span><span class="meta">${k.files.length} ${k.files.length === 1 ? "file" : "files"}</span>`;
      r.addEventListener("click", () => jumpTo(k.id));
      frag.appendChild(r);
    }
  }
  if (a.people.length || a.externals.length) {
    const t = document.createElement("div");
    t.className = "group-title";
    t.textContent = "Around the system";
    frag.appendChild(t);
    for (const p of a.people) {
      const r = document.createElement("button");
      r.className = `map-row is-person${store.selection === p.id ? " is-selected" : ""}`;
      r.dataset.testid = `map-${p.id}`;
      r.innerHTML = `<span class="glyph">☺</span><span class="name">${esc(p.name)}</span><span class="meta">person</span>`;
      r.addEventListener("click", () => jumpTo(p.id));
      frag.appendChild(r);
    }
    for (const x of a.externals) {
      const r = document.createElement("button");
      r.className = `map-row is-external${store.selection === x.id ? " is-selected" : ""}`;
      r.dataset.testid = `map-${x.id}`;
      r.innerHTML = `<span class="glyph">◌</span><span class="name">${esc(x.name)}</span><span class="meta">${esc(EXTERNAL_LABEL[x.kind] ?? x.kind).toLowerCase()}</span>`;
      r.addEventListener("click", () => jumpTo(x.id));
      frag.appendChild(r);
    }
  }
  el.appendChild(frag);
}

export function playJourney(j: Journey, step = 0, view?: "map" | "sequence"): void {
  setJourney(j, step);
  if (view) setView(view);
  if (store.level === "code" && store.view === "map") setLevel("containers");
  select(null);
  setShelfTab("journeys");
}

function renderJourneys(): void {
  const el = $("[data-tab-panel=journeys]");
  const a = store.atlas;
  el.innerHTML = "";
  if (!a) return;
  const head = document.createElement("div");
  head.className = "journeys-head";
  const d = store.discovery;
  head.innerHTML = `<span class="meta">${a.journeys.length ? `${a.journeys.length} ${a.journeys.length === 1 ? "journey" : "journeys"}` : "No journeys yet"}</span><span class="journeys-actions"><button class="ghost" data-testid="journey-steer" title="Choose the flows for the next discovery, with a note for each narrator" ${d.running ? "disabled" : ""}>Steer…</button><button class="ghost" data-testid="journey-new" title="Write a journey of your own: pick the elements, add the arrows">+ New journey</button></span>`;
  head.querySelector("[data-testid=journey-new]")!.addEventListener("click", () => { setShelfTab("journeys"); newJourney(); });
  head.querySelector("[data-testid=journey-steer]")!.addEventListener("click", () => openPlan());
  el.appendChild(head);
  // The scout's picks, while the narrators are still writing them.
  const pending = d.running ? d.proposed.filter((f) => !a.journeys.some((j) => (f.entry && j.entry === f.entry) || j.name === f.name)) : [];
  for (const f of pending) {
    const row = document.createElement("div");
    row.className = "journey-row is-pending";
    row.dataset.testid = `journey-pending-${slugOf(f.name)}`;
    row.innerHTML = `<span class="jr-top"><span class="name">${esc(f.name)}</span><span class="spinner is-small"></span><span class="meta">being narrated</span></span>${f.why ? `<span class="jr-why">${esc(f.why)}</span>` : ""}<span class="jr-langs"><span class="jr-entry">${esc(f.entry || "no trace: the narrator finds where it starts")}</span></span>`;
    el.appendChild(row);
  }
  if (!a.journeys.length && !pending.length) { el.insertAdjacentHTML("beforeend", `<p class="empty-note">${a.source === "engine" ? "No journey crosses a boundary yet. Discover with Claude to have a scout find the key flows, or write one by hand." : "No journeys yet. Steer the next discovery, or write one by hand."}</p>`); return; }
  for (const j of a.journeys) {
    const b = document.createElement("button");
    b.className = `journey-row${store.journey?.id === j.id ? " is-selected" : ""}`;
    b.dataset.testid = `journey-${j.id}`;
    const langs = [...new Set(j.messages.flatMap((s) => [s.from, s.to]).map((id) => containerOf(id)).filter(Boolean).map((c) => langOf(c!.technology, c!.language)))];
    const claimed = j.messages.filter((m) => m.source === "claimed").length;
    const who = j.source === "user" ? `<span class="chip is-user">yours</span>` : j.source === "claude" ? `<span class="chip is-claude">Claude</span>` : "";
    const busy = store.narrating === j.id ? `<span class="spinner is-small"></span>` : "";
    b.innerHTML = `<span class="jr-top"><span class="name">${esc(j.name)}</span>${busy}${who}<span class="meta">${j.messages.length} ${j.messages.length === 1 ? "message" : "messages"}${claimed ? ` · <span class="is-claimed-text">${claimed} claimed</span>` : ""}</span></span>${j.why ? `<span class="jr-why" title="Why the scout picked it">${esc(j.why)}</span>` : ""}<span class="jr-summary">${esc(j.summary)}</span><span class="jr-langs">${langs.map((l) => `<span class="dot" data-lang="${l}"></span>`).join("")}<span class="jr-entry">${esc(j.entry || (j.note ? `✎ ${j.note}` : j.source === "user" ? "written by hand" : "no trace: followed in the code"))}</span><span class="jr-open" data-testid="journey-${j.id}-sequence" title="Open as a sequence diagram">sequence ›</span></span>`;
    b.querySelector(".jr-open")!.addEventListener("click", (e) => { e.stopPropagation(); playJourney(j, 0, "sequence"); });
    b.addEventListener("click", () => { if (store.journey?.id === j.id && !store.draft) setJourney(null); else playJourney(j); });
    el.appendChild(b);
  }
}

function renderGuide(): void {
  const el = $("[data-tab-panel=guide]");
  const a = store.atlas;
  el.innerHTML = "";
  if (!a) return;
  const g = a.guide;
  el.innerHTML = `
    <p class="guide-summary" data-testid="guide-summary">${prose(a.system.summary)}</p>
    ${g.start_here.length ? `<div class="group-title">Start here</div>${g.start_here.map((p) => `<button class="guide-row" data-el="${esc(p.element)}"><span class="name">${esc(elementName(p.element))}</span><span class="why">${esc(p.why)}</span></button>`).join("")}` : ""}
    ${g.callouts.length ? `<div class="group-title">Worth knowing</div>${g.callouts.map((c) => `<button class="guide-row is-callout" ${c.element ? `data-el="${esc(c.element)}"` : "disabled"}><span class="name">${esc(c.title)}</span><span class="why">${esc(c.detail)}</span></button>`).join("")}` : ""}
    ${a.report.notes?.length ? `<div class="group-title">What the engine fixed</div><ul class="notes-list">${a.report.notes.map((n) => `<li>${esc(n)}</li>`).join("")}</ul>` : ""}`;
  el.querySelectorAll<HTMLButtonElement>("[data-el]").forEach((b) => b.addEventListener("click", () => jumpTo(b.dataset.el!)));
}

// ---- the card ---------------------------------------------------------------------

function renderCard(): void {
  const card = $("#card");
  const id = store.selection;
  const rk = store.relSelection;
  if ((id === null && rk === null) || store.notesOpen) { card.hidden = true; card.innerHTML = ""; document.body.classList.remove("has-card"); return; }
  card.innerHTML = rk ? relationshipCard(rk) : elementCard(id!);
  card.hidden = false;
  document.body.classList.add("has-card");
  card.querySelector("[data-testid=card-close]")!.addEventListener("click", () => select(null));
  card.querySelectorAll<HTMLButtonElement>("[data-jump]").forEach((b) => b.addEventListener("click", () => jumpTo(b.dataset.jump!)));
  card.querySelectorAll<HTMLButtonElement>("[data-rel]").forEach((b) => b.addEventListener("click", () => selectRelationship(b.dataset.rel!)));
  card.querySelectorAll<HTMLButtonElement>("[data-zoom]").forEach((b) => b.addEventListener("click", () => zoomInto(b.dataset.zoom!)));
  card.querySelectorAll<HTMLButtonElement>("[data-open]").forEach((b) => b.addEventListener("click", () => void api.openPath(b.dataset.open!, b.dataset.line ? Number(b.dataset.line) : undefined).catch((e) => toast(`Cannot open: ${String(e)}`, "error"))));
  card.querySelectorAll<HTMLButtonElement>("[data-journey]").forEach((b) => b.addEventListener("click", () => { const j = store.atlas!.journeys.find((x) => x.id === b.dataset.journey); if (j) playJourney(j, Number(b.dataset.step ?? 0)); }));
}

function relRows(rels: Relationship[], dir: "out" | "in", self: string): string {
  if (!rels.length) return "";
  return `<div class="card-section"><h3>${dir === "out" ? "Talks to" : "Used by"}</h3>${rels.map((r) => {
    const other = dir === "out" ? r.to : r.from;
    return `<button class="nb is-${r.source}" data-rel="${esc(relKey(r))}" data-testid="rel-${esc(relKey(r))}" title="${esc(r.label)}${r.technology ? ` (${esc(r.technology)})` : ""}"><span class="arrow">${dir === "out" ? "→" : "←"}</span><span class="nb-name">${esc(elementName(other))}</span><span class="nb-label">${esc(r.label)}</span></button>`;
  }).join("")}</div>`.replace(self, self);
}

function sourceBadge(source: string): string {
  return source === "code" ? `<span class="chip is-ok">backed by code</span>` : source === "survey" ? `<span class="chip">from the survey</span>` : `<span class="chip is-claimed">claimed, not seen in code</span>`;
}

function elementCard(id: string): string {
  const e = elementById(id);
  const a = store.atlas!;
  if (!e) return `<button class="card-close" data-testid="card-close">×</button><p>Nothing selected.</p>`;
  const close = `<button class="card-close" data-testid="card-close" title="Close (Esc)">×</button>`;
  const rels = relationshipsOf(id);
  const outs = rels.filter((r) => r.from === id);
  const ins = rels.filter((r) => r.to === id);
  const journeys = a.journeys.filter((j) => j.messages.some((s) => s.from === id || s.to === id || containerOf(s.from)?.id === id || containerOf(s.to)?.id === id));
  const jrows = journeys.length ? `<div class="card-section"><h3>Journeys through here</h3>${journeys.map((j) => `<button class="nb" data-journey="${esc(j.id)}"><span class="arrow">▶</span><span class="nb-name">${esc(j.name)}</span><span class="nb-label">${j.messages.length} messages</span></button>`).join("")}</div>` : "";
  switch (e.kind) {
    case "system":
      return `${close}
        <div class="card-kind">Software system</div>
        <h2 data-testid="card-title">${esc(a.system.name)}</h2>
        <p class="card-lede">${esc(a.system.purpose)}</p>
        <p class="card-body">${prose(a.system.summary)}</p>
        <div class="card-actions"><button class="primary" data-zoom="s" data-testid="card-zoom">Open the containers</button></div>
        ${a.guide.start_here.length ? `<div class="card-section"><h3>Start here</h3>${a.guide.start_here.map((p) => `<button class="nb" data-jump="${esc(p.element)}"><span class="arrow">→</span><span class="nb-name">${esc(elementName(p.element))}</span></button><p class="nb-why">${esc(p.why)}</p>`).join("")}</div>` : ""}
        ${a.guide.callouts.length ? `<div class="card-section"><h3>Worth knowing</h3>${a.guide.callouts.map((c) => `<p class="callout"><b>${esc(c.title)}</b><br>${esc(c.detail)}</p>`).join("")}</div>` : ""}`;
    case "person":
      return `${close}
        <div class="card-kind">Person</div>
        <h2 data-testid="card-title">${esc(e.person.name)}</h2>
        <p class="card-body">${prose(e.person.description)}</p>
        ${relRows(outs, "out", id)}${jrows}`;
    case "external":
      return `${close}
        <div class="card-kind">${esc(EXTERNAL_LABEL[e.external.kind] ?? "Outside system")}</div>
        <h2 data-testid="card-title">${esc(e.external.name)}</h2>
        <p class="card-body">${prose(e.external.description)}</p>
        ${relRows(ins, "in", id)}${relRows(outs, "out", id)}${jrows}`;
    case "container": {
      const c = e.container;
      const d = store.discovery.agents.find((x) => x.target === c.id);
      return `${close}
        <div class="card-kind"><span class="dot" data-lang="${langOf(c.technology, c.language)}"></span>${esc(KIND_LABEL[c.kind] ?? "Container")} · ${esc(c.technology)}</div>
        <h2 data-testid="card-title">${esc(c.name)}</h2>
        <div class="card-path" data-testid="card-path">${esc(c.package)}</div>
        <p class="card-body">${prose(c.description)}</p>
        ${c.responsibilities?.length ? `<ul class="resp">${c.responsibilities.map((r) => `<li>${esc(r)}</li>`).join("")}</ul>` : ""}
        ${d && !d.done ? `<p class="meta"><span class="lamp-dot"></span> An agent is reading this container${d.current ? `: ${esc(d.current)}` : ""}.</p>` : ""}
        <div class="card-actions"><button class="primary" data-zoom="${esc(c.id)}" data-testid="card-zoom">Open its ${c.components.length} components</button></div>
        <div class="card-section"><h3>Components</h3>${c.components.map((k) => `<button class="nb" data-jump="${esc(k.id)}" data-testid="nb-${esc(k.id)}"><span class="arrow">▸</span><span class="nb-name">${esc(k.name)}</span><span class="nb-label">${k.files.length} ${k.files.length === 1 ? "file" : "files"}</span></button>`).join("")}</div>
        ${relRows(outs, "out", id)}${relRows(ins, "in", id)}${jrows}`;
    }
    case "component": {
      const { container: c, component: k } = e;
      return `${close}
        <div class="card-kind"><span class="dot" data-lang="${langOf(k.technology || c.technology, c.language)}"></span>component in <button class="link-quiet" data-jump="${esc(c.id)}">${esc(c.name)}</button></div>
        <h2 data-testid="card-title">${esc(k.name)}</h2>
        ${k.technology ? `<div class="card-path">${esc(k.technology)}</div>` : ""}
        <p class="card-body">${prose(k.description)}</p>
        ${k.responsibilities?.length ? `<ul class="resp">${k.responsibilities.map((r) => `<li>${esc(r)}</li>`).join("")}</ul>` : ""}
        <div class="card-actions"><button class="primary" data-zoom="${esc(k.id)}" data-testid="card-zoom">Read the code</button></div>
        <div class="card-section"><h3>Files</h3>${k.files.map((f) => `<button class="nb is-file" data-open="${esc(f)}" title="Open in editor"><span class="arrow">·</span><span class="nb-name mono">${esc(f)}</span></button>`).join("")}</div>
        ${relRows(outs, "out", id)}${relRows(ins, "in", id)}${jrows}`;
    }
  }
}

function relationshipCard(key: string): string {
  const a = store.atlas!;
  const [from, to] = key.split(">");
  // Several relationships can share a drawn edge (context level rolls them up).
  const rels = a.relationships.filter((r) => relKey(r) === key);
  const close = `<button class="card-close" data-testid="card-close" title="Close (Esc)">×</button>`;
  if (!rels.length) {
    const rolled = a.relationships.filter((r) => (from === "s" ? containerOf(r.from) !== null : r.from === from) && (to === "s" ? containerOf(r.to) !== null : r.to === to));
    if (!rolled.length) return `${close}<p>No relationship ${esc(from)} → ${esc(to)}.</p>`;
    return `${close}
      <div class="card-kind">Relationship</div>
      <h2 data-testid="card-title">${esc(elementName(from))} → ${esc(elementName(to))}</h2>
      <div class="card-section"><h3>Made of</h3>${rolled.map((r) => `<button class="nb is-${r.source}" data-rel="${esc(relKey(r))}"><span class="arrow">→</span><span class="nb-name">${esc(elementName(r.from))} → ${esc(elementName(r.to))}</span><span class="nb-label">${esc(r.label)}</span></button>`).join("")}</div>`;
  }
  const r = rels[0];
  const ev = r.evidence ?? [];
  const journeys = a.journeys.map((j) => ({ j, i: j.messages.findIndex((s) => s.from === r.from && s.to === r.to || (containerOf(s.from)?.id === r.from && containerOf(s.to)?.id === r.to)) })).filter((x) => x.i >= 0);
  return `${close}
    <div class="card-kind">Relationship · ${esc(r.level)} level</div>
    <h2 data-testid="card-title"><button class="link-quiet" data-jump="${esc(r.from)}">${esc(elementName(r.from))}</button> → <button class="link-quiet" data-jump="${esc(r.to)}">${esc(elementName(r.to))}</button></h2>
    <p class="card-lede">${esc(r.label)}${r.technology ? ` <span class="meta">over ${esc(r.technology)}</span>` : ""}</p>
    <div class="chips">${sourceBadge(r.source)}</div>
    ${r.source === "claimed" ? `<p class="callout is-claimed">An agent wrote this relationship, but the engine found no import, call or flow between these two in the code. Treat it as a hint, not a fact.</p>` : ""}
    ${r.source === "survey" ? `<p class="meta">People and outside systems are declared by the survey; the code cannot show who sits at the keyboard.</p>` : ""}
    ${ev.length ? `<div class="card-section"><h3>Evidence in the code (${ev.length})</h3>${ev.map((x) => `<button class="nb is-evidence" data-open="${esc(x.from.split("#")[0])}" ${x.line ? `data-line="${x.line}"` : ""} title="Open in editor"><span class="arrow">${x.via === "flow" ? "⇢" : x.via === "tag" ? "◌" : "→"}</span><span class="nb-name mono">${esc(x.from)}${x.line ? `:${x.line}` : ""}</span><span class="nb-label">${esc(x.via === "tag" ? (x.label ?? "tag") : x.via === "flow" ? (x.label ?? "flow") : x.via)}</span></button><div class="ev-to mono">${esc(x.via === "tag" ? "" : x.to)}</div>`).join("")}</div>` : ""}
    ${journeys.length ? `<div class="card-section"><h3>On these journeys</h3>${journeys.map(({ j, i }) => `<button class="nb" data-journey="${esc(j.id)}" data-step="${i + 1}"><span class="arrow">▶</span><span class="nb-name">${esc(j.name)}</span><span class="nb-label">step ${i + 1}</span></button>`).join("")}</div>` : ""}`;
}

// ---- journey bar ----------------------------------------------------------------

function initJourneyBar(): void {
  $("#jb-prev").addEventListener("click", () => setJourneyStep(store.journeyStep - 1));
  $("#jb-next").addEventListener("click", () => setJourneyStep(store.journeyStep + 1));
  $("#jb-close").addEventListener("click", () => setJourney(null));
  $("#jb-all").addEventListener("click", () => setJourneyStep(0));
}

function renderJourneyBar(): void {
  const bar = $("#journey-bar");
  const j = store.draft ?? store.journey;
  bar.hidden = !j;
  document.body.classList.toggle("has-journey", !!j);
  if (!j) return;
  const p = currentProjection();
  const total = journeyLength();
  const n = store.journeyStep;
  const step = n > 0 ? p?.messages[n - 1] ?? null : null;
  $("#jb-title").textContent = j.name || "Untitled journey";
  $("#jb-count").textContent = n > 0 ? `Step ${n} of ${total}` : `${total} ${total === 1 ? "step" : "steps"} at this level, all shown`;
  $("#jb-caption").innerHTML = step
    ? `<span class="jb-from">${esc(elementName(step.from))}</span><span class="jb-arrow">${step.kind === "return" ? "⇠" : "→"}</span><span class="jb-to">${esc(elementName(step.to))}</span><span class="jb-text">${esc(step.caption || step.label)}</span>${step.source === "claimed" ? `<span class="chip is-claimed">claimed</span>` : ""}`
    : `<span class="jb-text">${esc(j.summary || "No summary yet.")}</span>`;
  $<HTMLButtonElement>("#jb-prev").disabled = n <= 0;
  $<HTMLButtonElement>("#jb-next").disabled = n >= total;
  const range = $<HTMLInputElement>("#jb-range");
  range.max = String(total);
  range.value = String(n);
  range.style.setProperty("--fill", `${(n / Math.max(1, total)) * 100}%`);
  range.oninput = () => setJourneyStep(Number(range.value));
  const edit = $<HTMLButtonElement>("#jb-edit");
  edit.hidden = store.draft !== null;
  edit.onclick = () => startDraft();
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

function slugOf(s: string): string {
  return s.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "") || "x";
}

export function esc(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
}

/** Escape, then render `inline code` the way the agents write it. */
export function prose(s: string): string {
  return esc(s).replace(/`([^`]+)`/g, "<code>$1</code>");
}
