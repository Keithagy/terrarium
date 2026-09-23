// Discovery, live: the agents at work, what each one is reading, what they find
// as they find it, and the check at the end. Partial results land in the atlas
// as they arrive, so the diagram develops in front of you.

import { api, log, on } from "./tauri";
import { store, subscribe, emit, freshDiscovery, setAtlas, setLevel, elementName, type AgentState, type Note } from "./store";
import type { Atlas, DiscoveryRun, Progress } from "./types";
import { esc, toast } from "./panels";

const $ = <T extends HTMLElement>(sel: string): T => document.querySelector(sel) as T;

let ticker = 0;

export function initDiscovery(): void {
  $("#discover-btn").addEventListener("click", () => void runDiscovery());
  $("#notes-close").addEventListener("click", () => { store.notesOpen = false; emit("discovery"); emit("ui"); });
  $("#notes-btn").addEventListener("click", () => { store.notesOpen = !store.notesOpen; emit("discovery"); emit("ui"); });
  subscribe("discovery", render);
  subscribe("atlas", render);
  void on<Record<string, never>>("discover:started", () => start(""));
  void on<Progress>("discover:progress", onProgress);
  void on<DiscoveryRun>("discover:done", onDone);
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

export async function runDiscovery(): Promise<void> {
  if (store.discovery.running || !store.graphLoaded) return;
  start("");
  try {
    await api.discoverWithClaude();
  } catch (e) {
    log("warn", `discovery failed: ${String(e)}`);
  }
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
  field: "In the field: one agent per container and per journey, reading the code",
  editor: "Editing: the summary, where to start, what to know",
  verify: "Checking every relationship against the code",
};

function agentKey(role: string, target: string | null): string {
  return `${role}:${target ?? ""}`;
}

function onProgress(p: Progress): void {
  const d = store.discovery;
  if (!d.running) start("");
  switch (p.event) {
    case "started":
      d.model = p.model;
      d.total = 1 + p.containers + p.journeys + 1;
      note("stage", `Discovery started on ${p.model}: a surveyor, ${p.containers} container ${p.containers === 1 ? "agent" : "agents"}, ${p.journeys} journey ${p.journeys === 1 ? "narrator" : "narrators"} and an editor.`);
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

const STAGES: [string, string][] = [["survey", "Survey"], ["field", "Field"], ["editor", "Editor"], ["verify", "Check"]];

function renderNotes(): void {
  const panel = $("#notes");
  const d = store.discovery;
  const open = store.notesOpen && (d.running || d.notes.length > 0);
  panel.hidden = !open;
  document.body.classList.toggle("has-notes", open);
  if (!open) return;
  renderNotesHead();
  const stageIdx = STAGES.findIndex(([k]) => k === d.stage);
  $("#notes-stages").innerHTML = STAGES.map(([k, label], i) => `<span class="stage${i < stageIdx || (!d.running && !d.error && d.stage) ? " is-done" : ""}${i === stageIdx && d.running ? " is-current" : ""}" data-testid="stage-${k}">${label}</span>`).join(`<span class="stage-sep"></span>`);
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
