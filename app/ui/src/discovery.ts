// Discovery, live: the agents at work, what each one is reading, what they find
// as they find it, and the check at the end. Partial results land in the atlas
// as they arrive, so the diagram develops in front of you.

import { api, log, on } from "./tauri";
import { store, subscribe, emit, freshDiscovery, setAtlas, setLevel, elementName, type AgentState, type Note } from "./store";
import type { Atlas, DiscoveryRun, FlowRequest, NarrateDone, Progress, Proposal, ProposeDone } from "./types";
import { esc, toast } from "./panels";

const $ = <T extends HTMLElement>(sel: string): T => document.querySelector(sel) as T;

let ticker = 0;
/** Container agents in the running discovery, for recounting the total once the scout answers. */
let fieldContainers = 0;
/** Whether the running discovery has a scout; it has none when the person chose the flows. */
let scouting = true;

export function initDiscovery(): void {
  $("#discover-btn").addEventListener("click", () => openPlan());
  $("#notes-close").addEventListener("click", () => { store.notesOpen = false; emit("discovery"); emit("ui"); });
  $("#notes-btn").addEventListener("click", () => { store.notesOpen = !store.notesOpen; emit("discovery"); emit("ui"); });
  initPlan();
  subscribe("discovery", render);
  subscribe("atlas", render);
  void on<Record<string, never>>("discover:started", () => start(""));
  void on<Progress>("discover:progress", onProgress);
  void on<DiscoveryRun>("discover:done", onDone);
  void on<{ journey: string; note: string }>("journey:started", (p) => { store.narrating = p.journey; emit("journey"); emit("atlas"); emit("ui"); });
  void on<NarrateDone>("journey:done", (p) => {
    store.narrating = null;
    void reloadAtlas().then(() => {
      const j = p.journey;
      toast(j ? `Narrated “${j.name}”: ${j.messages.length} messages, ${j.messages.filter((m) => m.source === "claimed").length} claimed, for $${p.run.cost_usd.toFixed(2)}` : "Narrated", "ok", 6000);
      log("info", "journey narrated", { cost_usd: p.run.cost_usd, secs: p.run.secs });
      emit("journey");
      emit("ui");
    });
  });
  void on<{ journey: string; error: string }>("journey:error", (p) => {
    store.narrating = null;
    toast(`Narration failed: ${p.error}`, "error", 7000);
    emit("journey");
    emit("atlas");
    emit("ui");
  });
  void on<{ error: string }>("discover:error", (e) => {
    const d = store.discovery;
    d.running = false;
    d.finishedAt = Date.now();
    d.error = e.error;
    note("error", `Discovery failed: ${e.error}`);
    stopTicker();
    toast(`Discovery failed: ${e.error}`, "error", 7000);
    emit("discovery");
  });
  void on<Record<string, never>>("discover:reset", () => void reloadAtlas());
}

export async function runDiscovery(flows: FlowRequest[] = []): Promise<void> {
  if (store.discovery.running || !store.graphLoaded) return;
  start("");
  try {
    await api.discoverWithClaude(undefined, flows);
  } catch (e) {
    log("warn", `discovery failed: ${String(e)}`);
  }
}

// ---- the plan: what the person wants narrated, before anything is spent ----------------

interface PlanRow {
  entry: string;
  name: string;
  note: string;
  /** Why the scout proposed it; empty for a flow the person added or the atlas already had. */
  why: string;
  on: boolean;
}

let plan: PlanRow[] = [];
/** The scout at work for the plan sheet: what it is reading now. */
let proposing: { on: boolean; current: string | null } = { on: false, current: null };

function initPlan(): void {
  $("#plan-cancel").addEventListener("click", () => { $("#plan").hidden = true; emit("ui"); });
  $("#plan").addEventListener("click", (e) => { if (e.target === $("#plan")) { $("#plan").hidden = true; emit("ui"); } });
  $("#plan-add").addEventListener("click", () => addPlanRow());
  $("#plan-new-name").addEventListener("keydown", (e) => { if (e.key === "Enter") addPlanRow(); });
  $("#plan-propose").addEventListener("click", () => {
    if (proposing.on || store.discovery.running) return;
    log("info", "proposing flows");
    // the propose:* events drive the sheet; the promise only reports that the call went out
    api.proposeFlows().catch((e) => log("warn", `propose failed: ${String(e)}`));
  });
  $("#plan-run").addEventListener("click", () => {
    const flows: FlowRequest[] = plan.filter((r) => r.on && (r.entry || r.name.trim())).map((r) => ({ entry: r.entry, name: r.name.trim(), note: r.note.trim(), why: r.why }));
    $("#plan").hidden = true;
    emit("ui");
    log("info", "discovery planned", { flows: flows.length });
    void runDiscovery(flows);
  });
  void on<Record<string, never>>("propose:started", () => { proposing = { on: true, current: null }; renderProposing(); });
  void on<Progress>("propose:progress", (p) => {
    if (p.event === "agent_activity") { proposing.current = p.kind === "reading" ? p.path ?? null : `searching ${p.query ?? ""}`.trim(); renderProposing(); }
  });
  void on<ProposeDone>("propose:done", (v) => {
    proposing = { on: false, current: null };
    fillPlan(v.proposals);
    renderProposing();
    toast(v.proposals.length ? `Claude proposed ${v.proposals.length} key ${v.proposals.length === 1 ? "flow" : "flows"} for $${v.run.cost_usd.toFixed(2)}` : "Claude proposed no flows; the scanner's traces stand", v.proposals.length ? "ok" : "info", 5000);
    log("info", "flows proposed", { flows: v.proposals.length, cost_usd: v.run.cost_usd });
  });
  void on<{ error: string }>("propose:error", (e) => {
    proposing = { on: false, current: null };
    renderProposing();
    toast(`Proposing flows failed: ${e.error}`, "error", 7000);
  });
}

/** The scout's proposals become ticked rows at the top; rows it did not propose stay below as they were. */
function fillPlan(proposals: Proposal[]): void {
  const same = (r: PlanRow, p: Proposal) => (p.entry && r.entry === p.entry) || r.name.trim().toLowerCase() === p.name.toLowerCase();
  const rows: PlanRow[] = proposals.map((p) => {
    const had = plan.find((r) => same(r, p));
    return { entry: p.entry, name: had?.name.trim() || p.name, note: had?.note ?? "", why: p.why, on: true };
  });
  const rest = plan.filter((r) => !proposals.some((p) => same(r, p)));
  plan = [...rows, ...rest];
  if (!$("#plan").hidden) renderPlan();
}

function renderProposing(): void {
  const b = $<HTMLButtonElement>("#plan-propose");
  b.disabled = proposing.on || store.discovery.running;
  b.innerHTML = proposing.on ? `<span class="spinner is-small"></span> Scouting…` : "Propose with Claude";
  // one agent at a time: the backend refuses a discovery while the scout runs
  $<HTMLButtonElement>("#plan-run").disabled = proposing.on;
  const s = $("#plan-proposing");
  s.hidden = !proposing.on;
  s.textContent = proposing.on ? (proposing.current ? `reading ${proposing.current}` : "reading where flows begin") : "";
  emit("ui");
}

/** What the plan sheet shows, for the bridge's `/ui`. */
export function planSnapshot(): { open: boolean; proposing: boolean; rows: { name: string; entry: string; why: string; on: boolean }[] } {
  return { open: !$("#plan").hidden, proposing: proposing.on, rows: plan.map((r) => ({ name: r.name, entry: r.entry, why: r.why, on: r.on })) };
}

function addPlanRow(): void {
  const name = $<HTMLInputElement>("#plan-new-name").value.trim();
  if (!name) return;
  plan.push({ entry: "", name, note: $<HTMLInputElement>("#plan-new-note").value.trim(), why: "", on: true });
  $<HTMLInputElement>("#plan-new-name").value = "";
  $<HTMLInputElement>("#plan-new-note").value = "";
  renderPlan();
}

/** Open the plan sheet with the flows the atlas knows: what the scanner can trace, and what stands now. */
export function openPlan(): void {
  if (store.discovery.running || !store.graphLoaded) return;
  const a = store.atlas;
  if (!a) return;
  plan = a.journeys.map((j) => ({ entry: j.entry, name: j.source === "engine" ? "" : j.name, note: j.note ?? "", why: j.why ?? "", on: j.source !== "engine" }));
  renderPlan();
  renderProposing();
  $("#plan").hidden = false;
  emit("ui");
}

function renderPlan(): void {
  const ul = $("#plan-flows");
  ul.innerHTML = plan.map((r, i) => `
    <li class="plan-flow${r.on ? " is-on" : ""}" data-testid="plan-flow-${i + 1}">
      <label class="plan-check"><input type="checkbox" data-i="${i}" data-field="on" data-testid="plan-flow-${i + 1}-on" ${r.on ? "checked" : ""} /></label>
      <div class="plan-fields">
        <input type="text" data-i="${i}" data-field="name" data-testid="plan-flow-${i + 1}-name" value="${esc(r.name)}" placeholder="${esc(r.entry ? `What the user is doing (from ${r.entry})` : "What the user is doing")}" />
        <input type="text" data-i="${i}" data-field="note" data-testid="plan-flow-${i + 1}-note" value="${esc(r.note)}" placeholder="Note for the narrator (optional)" />
        ${r.why ? `<span class="plan-why" data-testid="plan-flow-${i + 1}-why" title="Why the scout proposed it">${esc(r.why)}</span>` : ""}
        ${r.entry ? `<span class="plan-entry mono">${esc(r.entry)}</span>` : `<span class="plan-entry">no start in the code yet: the narrator finds it</span>`}
      </div>
      <button class="ghost is-round" data-remove="${i}" title="Remove">×</button>
    </li>`).join("");
  ul.querySelectorAll<HTMLInputElement>("[data-field]").forEach((inp) => inp.addEventListener("input", () => {
    const r = plan[Number(inp.dataset.i)];
    if (!r) return;
    if (inp.dataset.field === "on") r.on = inp.checked;
    else if (inp.dataset.field === "name") { r.name = inp.value; if (inp.value.trim()) r.on = true; }
    else r.note = inp.value;
    ul.querySelector(`[data-testid="plan-flow-${Number(inp.dataset.i) + 1}"]`)?.classList.toggle("is-on", r.on);
    ul.querySelector<HTMLInputElement>(`[data-testid="plan-flow-${Number(inp.dataset.i) + 1}-on"]`)!.checked = r.on;
    renderPlanSummary();
  }));
  ul.querySelectorAll<HTMLButtonElement>("[data-remove]").forEach((b) => b.addEventListener("click", () => { plan.splice(Number(b.dataset.remove), 1); renderPlan(); }));
  renderPlanSummary();
}

function renderPlanSummary(): void {
  const n = plan.filter((r) => r.on && (r.entry || r.name.trim())).length;
  $("#plan-summary").textContent = n ? `${n} ${n === 1 ? "journey" : "journeys"} chosen by you` : "the scout will propose the journeys";
}

export async function resetAtlas(): Promise<void> {
  try {
    const v = await api.resetAtlas();
    setAtlas(v.atlas, v.stale ?? null);
    toast("Back to the engine's atlas", "ok");
  } catch (e) { toast(`Cannot reset: ${String(e)}`, "error"); }
}

async function reloadAtlas(): Promise<void> {
  try { const v = await api.getAtlas(); setAtlas(v.atlas, v.stale ?? null); } catch (e) { log("warn", `reload atlas failed: ${String(e)}`); }
}

function start(model: string): void {
  if (store.discovery.running) return;
  store.discovery = { ...freshDiscovery(), running: true, model, startedAt: Date.now() };
  store.notesOpen = true;
  // The field stage happens on the containers: watch it there.
  if (store.level === "code" || store.level === "context") setLevel("containers");
  stopTicker();
  ticker = window.setInterval(() => { renderHeader(); renderNotesHead(); }, 1000);
  emit("discovery");
  emit("ui");
}

function stopTicker(): void {
  if (ticker) { clearInterval(ticker); ticker = 0; }
}

function note(kind: Note["kind"], text: string): void {
  store.discovery.notes.push({ t: Date.now(), kind, text });
}

const STAGE_WORDS: Record<string, string> = {
  survey: "Surveying: what the system is, who uses it, what it talks to",
  scout: "Scouting: which flows show what the system does, including the ones the scanner cannot trace",
  field: "In the field: one agent per container and per journey, reading the code",
  editor: "Editing: the summary, where to start, what to know",
  verify: "Checking every relationship against the code",
};

function agentKey(role: string, target: string | null): string {
  return `${role}:${target ?? ""}`;
}

function onProgress(p: Progress): void {
  const d = store.discovery;
  // one narrator rewriting one journey is not a discovery: nothing to open, only a journey to refresh
  if (store.narrating && !d.running) {
    if (p.event === "journey_done") {
      const a = store.atlas;
      if (a) {
        const i = a.journeys.findIndex((j) => j.id === p.journey.id);
        if (i >= 0) a.journeys[i] = p.journey; else a.journeys.push(p.journey);
        if (store.journey?.id === p.journey.id) store.journey = p.journey;
        emit("atlas");
        emit("journey");
      }
    }
    return;
  }
  if (!d.running) start("");
  switch (p.event) {
    case "started":
      d.model = p.model;
      fieldContainers = p.containers;
      scouting = p.scout;
      d.total = 1 + (p.scout ? 1 : 0) + p.containers + p.journeys + 1;
      note("stage", `Discovery started on ${p.model}: a surveyor, ${p.scout ? `a scout, ${p.containers} container ${p.containers === 1 ? "agent" : "agents"}, up to ${p.journeys}` : `${p.containers} container ${p.containers === 1 ? "agent" : "agents"}, ${p.journeys}`} journey ${p.journeys === 1 ? "narrator" : "narrators"} and an editor.`);
      break;
    case "proposed":
      // the scout may propose fewer flows than the most it was allowed
      d.total = 1 + 1 + fieldContainers + p.flows.length + 1;
      note("found", `The scout proposed ${p.flows.length} key ${p.flows.length === 1 ? "flow" : "flows"}.`);
      for (const f of p.flows) note("found", `“${f.name}”${f.why ? `: ${f.why}` : ""} ${f.entry ? `Starts at ${f.entry}.` : "The narrator finds where it starts."}`);
      break;
    case "stage":
      d.stage = p.stage;
      note("stage", STAGE_WORDS[p.stage] ?? p.stage);
      break;
    case "agent_started": {
      const a: AgentState = { key: agentKey(p.role, p.target), role: p.role, target: p.target, name: p.name, done: false, ok: null, error: null, cost_usd: 0, secs: 0, reads: 0, current: null };
      d.agents.push(a);
      break;
    }
    case "agent_activity": {
      const a = d.agents.find((x) => x.key === agentKey(p.role, p.target));
      if (!a) break;
      if (p.kind === "reading" && p.path) {
        a.reads++;
        a.current = p.path;
        const key = p.target ?? (p.role === "survey" ? "survey" : p.role);
        const list = d.reads.get(key) ?? [];
        list.push(p.path);
        d.reads.set(key, list);
      } else if (p.kind === "searching") {
        a.current = `searching ${p.query ?? ""}`.trim();
      }
      break;
    }
    case "agent_done": {
      const a = d.agents.find((x) => x.key === agentKey(p.role, p.target));
      if (a) { a.done = true; a.ok = p.ok; a.error = p.error; a.cost_usd = p.cost_usd; a.secs = p.secs; a.current = null; }
      d.done++;
      d.cost_usd += p.cost_usd;
      const who = a?.name ?? p.role;
      note(p.ok ? "agent" : "error", p.ok ? `${who}: done in ${fmtSecs(p.secs)}${a?.reads ? `, ${a.reads} ${a.reads === 1 ? "file" : "files"} read` : ""}, $${p.cost_usd.toFixed(2)}.` : `${who} failed: ${p.error ?? "unknown"}. The engine's words stand for it.`);
      break;
    }
    case "survey_done": {
      const a = store.atlas;
      if (a) {
        a.system = p.system;
        a.people = p.people;
        a.externals = p.externals;
        for (const c of p.containers) {
          const mine = a.containers.find((x) => x.id === c.id);
          if (mine) Object.assign(mine, { name: c.name, kind: c.kind, technology: c.technology, description: c.description, hidden: c.hidden });
        }
        a.source = "claude";
        emit("atlas");
        emit("level");
        emit("repo");
      }
      note("found", `The system is “${p.system.name}”: ${p.system.purpose} ${p.people.length} ${p.people.length === 1 ? "kind of person uses" : "kinds of people use"} it; it depends on ${p.externals.length} outside ${p.externals.length === 1 ? "system" : "systems"}.`);
      for (const c of p.containers.filter((c) => !c.hidden)) note("found", `${c.name} (${c.kind}, ${c.technology}): ${c.description}`);
      break;
    }
    case "container_done": {
      const a = store.atlas;
      if (a) {
        const i = a.containers.findIndex((x) => x.id === p.container.id);
        if (i >= 0) a.containers[i] = p.container;
        emit("atlas");
        emit("level");
      }
      note("found", `${p.container.name}: ${p.container.components.length} components: ${p.container.components.map((k) => k.name).join(", ")}.`);
      break;
    }
    case "journey_done": {
      const a = store.atlas;
      if (a && !a.journeys.some((j) => j.id === p.journey.id)) { a.journeys.push(p.journey); emit("atlas"); }
      note("found", `Journey “${p.journey.name}”: ${p.journey.steps.length} steps. ${p.journey.summary}`);
      break;
    }
    case "verified": {
      const r = p.atlas.report;
      note("check", `Checked: ${r.backed} relationships backed by code, ${r.survey} from the survey, ${r.claimed} claimed but not seen in the code.${r.unplaced?.length ? ` ${r.unplaced.length} files the agents did not place went into “Other files”.` : ""}`);
      for (const n of r.notes ?? []) note("check", n);
      setAtlas(p.atlas as Atlas, null);
      break;
    }
  }
  emit("discovery");
}

function onDone(run: DiscoveryRun): void {
  const d = store.discovery;
  d.running = false;
  d.finishedAt = Date.now();
  d.cost_usd = run.cost_usd;
  stopTicker();
  const failed = run.agents.filter((a) => !a.ok).length;
  note("stage", `Done in ${fmtSecs(run.secs)} for $${run.cost_usd.toFixed(2)}.${failed ? ` ${failed} ${failed === 1 ? "agent" : "agents"} failed; the engine's words stand for ${failed === 1 ? "it" : "them"}.` : ""}`);
  void reloadAtlas().then(() => {
    toast(`Discovered ${store.atlas?.system.name ?? "the atlas"} in ${fmtSecs(run.secs)} for $${run.cost_usd.toFixed(2)}`, failed ? "info" : "ok", 6000);
    log("info", "discovery done", { cost_usd: run.cost_usd, secs: run.secs, failed });
  });
  emit("discovery");
  emit("ui");
}

function fmtSecs(s: number): string {
  if (s < 60) return `${Math.round(s)}s`;
  return `${Math.floor(s / 60)}m ${Math.round(s % 60)}s`;
}

// ---- rendering: the header pill and the notes panel --------------------------------------

function render(): void {
  renderHeader();
  renderNotes();
}

function renderHeader(): void {
  const d = store.discovery;
  const btn = $<HTMLButtonElement>("#discover-btn");
  const a = store.atlas;
  btn.disabled = d.running || !store.graphLoaded;
  if (d.running) {
    const elapsed = fmtSecs((Date.now() - d.startedAt) / 1000);
    btn.innerHTML = `<span class="pulse"></span>Discovering · ${d.done}/${d.total || "?"} · $${d.cost_usd.toFixed(2)} · ${elapsed}`;
    btn.classList.add("is-running");
  } else {
    btn.classList.remove("is-running");
    btn.textContent = a?.source === "claude" ? "Rediscover with Claude" : "Discover with Claude";
  }
  const nb = $<HTMLButtonElement>("#notes-btn");
  nb.hidden = !(d.running || d.notes.length > 0);
  nb.classList.toggle("is-active", store.notesOpen);
}

function renderNotesHead(): void {
  const d = store.discovery;
  const head = $("#notes-head");
  if (!head) return;
  const elapsed = fmtSecs(((d.finishedAt ?? Date.now()) - d.startedAt) / 1000);
  head.innerHTML = `<div class="notes-kicker">${d.running ? `<span class="pulse"></span>Discovering` : d.error ? "Discovery failed" : "Discovered"}${d.model ? ` on ${esc(d.model)}` : ""}</div><h2>${d.running ? `${d.done} of ${d.total || "?"} agents done` : d.error ? "Stopped" : "Field notes"}</h2><div class="notes-meta">${elapsed} · $${d.cost_usd.toFixed(2)}</div>`;
}

const STAGES: [string, string][] = [["survey", "Survey"], ["scout", "Scout"], ["field", "Field"], ["editor", "Editor"], ["verify", "Check"]];

function renderNotes(): void {
  const panel = $("#notes");
  const d = store.discovery;
  const open = store.notesOpen && (d.running || d.notes.length > 0);
  panel.hidden = !open;
  document.body.classList.toggle("has-notes", open);
  if (!open) return;
  renderNotesHead();
  const stages = STAGES.filter(([k]) => k !== "scout" || scouting);
  const stageIdx = stages.findIndex(([k]) => k === d.stage);
  $("#notes-stages").innerHTML = stages.map(([k, label], i) => `<span class="stage${i < stageIdx || (!d.running && !d.error && d.stage) ? " is-done" : ""}${i === stageIdx && d.running ? " is-current" : ""}" data-testid="stage-${k}">${label}</span>`).join(`<span class="stage-sep"></span>`);
  const agents = $("#notes-agents");
  agents.innerHTML = d.agents.map((a) => `
    <li class="agent ${a.done ? (a.ok ? "is-ok" : "is-fail") : "is-busy"}" data-testid="agent-${esc(a.key)}">
      <span class="agent-state">${a.done ? (a.ok ? "✓" : "✗") : `<span class="spinner is-small"></span>`}</span>
      <span class="agent-name">${esc(a.name)}${a.target && a.role !== "container" ? "" : ""}</span>
      <span class="agent-now">${a.done ? `${a.reads ? `${a.reads} ${a.reads === 1 ? "file" : "files"} · ` : ""}${a.ok ? `$${a.cost_usd.toFixed(2)}` : esc(a.error ?? "failed")}` : a.current ? esc(a.current) : "starting"}</span>
    </li>`).join("");
  const log = $("#notes-log");
  const atBottom = log.scrollHeight - log.scrollTop - log.clientHeight < 40;
  log.innerHTML = d.notes.map((n) => `<li class="note is-${n.kind}"><span class="note-t">${fmtSecs((n.t - d.startedAt) / 1000)}</span><span>${esc(n.text)}</span></li>`).join("");
  if (atBottom) log.scrollTop = log.scrollHeight;
  const foot = $("#notes-foot");
  foot.innerHTML = d.running
    ? `<span class="meta">Agents may only read files. The engine checks every relationship they claim.</span>`
    : `<button class="ghost" data-testid="notes-reset">Use the engine's atlas</button><span class="meta">${store.atlas ? `${store.atlas.report.backed} backed · ${store.atlas.report.survey} survey · ${store.atlas.report.claimed} claimed` : ""}</span>`;
  foot.querySelector("[data-testid=notes-reset]")?.addEventListener("click", () => void resetAtlas());
  void elementName;
}
