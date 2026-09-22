// The Design tab: who designed the manual, running Claude's design agents, and
// the engine's joint check (weak joints, repairs, interlocked and loose files).

import { api, log, on } from "./tauri";
import { store, subscribe, emit, setBuild } from "./store";
import { esc, jumpTo, prose, setShelfTab, toast } from "./panels";

const $ = <T extends HTMLElement>(sel: string): T => document.querySelector(sel) as T;

interface ProgressEvent {
  event: "started" | "agent_done" | "assembling";
  agents?: number;
  model?: string;
  role?: string;
  ok?: boolean;
  done?: number;
  of?: number;
  cost_usd?: number;
}

export function initDesign(): void {
  $("#design-btn").addEventListener("click", () => void runDesign());
  subscribe("build", render);
  subscribe("tab", render);
  subscribe("design", () => { render(); emit("ui"); });
  void on<Record<string, never>>("design:started", () => {
    store.design = { running: true, model: "", agents: 0, done: 0, cost_usd: 0, log: [], error: null };
    emit("design");
  });
  void on<ProgressEvent>("design:progress", (p) => {
    const d = store.design;
    d.running = true;
    if (p.event === "started") { d.agents = p.agents ?? 0; d.model = p.model ?? ""; }
    if (p.event === "agent_done") {
      d.done = p.done ?? d.done + 1;
      d.cost_usd += p.cost_usd ?? 0;
      d.log.push({ role: p.role ?? "?", ok: !!p.ok, cost_usd: p.cost_usd ?? 0 });
    }
    if (p.event === "assembling") d.log.push({ role: "assembler (naming the model)", ok: true, cost_usd: 0 });
    emit("design");
  });
  void on<{ cost_usd: number; agents: { role: string; ok: boolean }[] }>("design:done", (run) => {
    store.design.running = false;
    store.design.cost_usd = run.cost_usd;
    void reloadBuild().then(() => {
      const failed = run.agents.filter((a) => !a.ok).length;
      toast(`Claude designed ${store.build?.check.steps ?? 0} steps for $${run.cost_usd.toFixed(2)}${failed ? `; ${failed} agents fell back to the engine` : ""}`, failed ? "info" : "ok", 5000);
    });
    emit("design");
  });
  void on<{ error: string }>("design:error", (e) => {
    store.design.running = false;
    store.design.error = e.error;
    toast(`Design failed: ${e.error}`, "error", 7000);
    emit("design");
  });
  void on<Record<string, never>>("design:reset", () => void reloadBuild());
}

async function reloadBuild(): Promise<void> {
  try { setBuild(await api.getBuild()); } catch (e) { log("warn", `reload build failed: ${String(e)}`); }
}

export async function runDesign(): Promise<void> {
  if (store.design.running || !store.graphLoaded) return;
  store.design = { running: true, model: "", agents: 0, done: 0, cost_usd: 0, log: [], error: null };
  emit("design");
  try {
    await api.designWithClaude();
  } catch (e) {
    // design:error already reported it
    log("warn", `design failed: ${String(e)}`);
  }
}

async function resetDesign(): Promise<void> {
  try {
    setBuild(await api.resetDesign());
    toast("Back to the engine's manual", "ok");
  } catch (e) { toast(`Cannot reset: ${String(e)}`, "error"); }
}

function render(): void {
  const el = $("#design-view");
  el.hidden = store.tab !== "design" || !store.build;
  if (el.hidden) return;
  const b = store.build!;
  const d = b.design;
  const c = b.check;
  const run = store.design;
  const byPath = new Map(b.model.buildings.map((x) => [x.path, x.id]));
  const fileBtn = (p: string) => byPath.has(p) ? `<button class="file-link" data-jump="${byPath.get(p)}">${esc(p)}</button>` : `<span class="mono">${esc(p)}</span>`;
  const agentRows = run.log.map((a) => `<li class="${a.ok ? "is-ok" : "is-fail"}"><span>${a.ok ? "✓" : "✗"}</span><span>${esc(a.role)}</span><span class="meta">${a.cost_usd ? `$${a.cost_usd.toFixed(2)}` : ""}</span></li>`).join("");
  el.innerHTML = `
    <div class="stage-head">
      <h2 data-testid="design-title">${esc(d.title)}</h2>
      <p class="meta">${d.source === "engine" ? "Designed by the engine: one sub-build per package, steps of a few files, captions made from the graph." : `Designed by ${esc(d.model ?? d.source)}: one agent per sub-build read the code and wrote its chapter, an assembler named the model, and the engine checked every joint.`}</p>
      <p>${prose(d.summary)}</p>
      <div class="card-actions">
        <button class="primary" data-testid="design-run" ${run.running ? "disabled" : ""}>${run.running ? "Designing…" : d.source === "engine" ? "Design with Claude" : "Redesign with Claude"}</button>
        ${d.source !== "engine" ? `<button class="ghost" data-testid="design-reset">Use the engine's design</button>` : ""}
      </div>
      ${b.stale ? `<p class="meta">${esc(b.stale)}</p>` : ""}
    </div>
    ${run.running || run.log.length || run.error ? `
    <section class="design-run" data-testid="design-progress">
      <h3>${run.running ? `Agents at work: ${run.done} of ${run.agents ? run.agents - 1 : "?"} chapters` : "Last run"}${run.model ? ` <span class="meta">on ${esc(run.model)}</span>` : ""}</h3>
      <ul class="agents">${agentRows || `<li><span class="spinner is-small"></span><span>Starting agents…</span></li>`}</ul>
      <p class="meta">${run.cost_usd ? `Spent $${run.cost_usd.toFixed(2)} so far.` : ""} Each agent may only read files; the engine verifies what they write.</p>
      ${run.error ? `<p class="is-weak">${esc(run.error)}</p>` : ""}
    </section>` : ""}
    <section>
      <h3>Joint check</h3>
      <div class="check-grid">
        <div class="check ${c.ok ? "is-ok" : "is-weak"}" data-testid="check-weak"><b>${c.weak.length}</b><span>weak joints</span></div>
        <div class="check"><b>${c.joints.toLocaleString()}</b><span>joints</span></div>
        <div class="check"><b>${c.bridges}</b><span>bridges</span></div>
        <div class="check ${c.interlocked.length ? "is-warn" : ""}"><b>${c.interlocked.length}</b><span>interlocked groups</span></div>
        <div class="check"><b>${c.loose.length}</b><span>loose files</span></div>
        <div class="check ${c.gaps ? "is-warn" : ""}"><button data-testid="check-gaps" class="link">${c.gaps}</button><span>endpoint gaps</span></div>
      </div>
      <p class="meta">A joint is an import or call between two files. The check places every file after everything it rests on; a weak joint is a file placed too early, left out, or invented.</p>
      ${c.weak.length ? `<h4>Weak joints</h4><ul class="plain">${c.weak.map((w) => `<li><b>${esc(w.kind)}</b> ${fileBtn(w.file)} <span class="meta">${esc(w.detail)}</span></li>`).join("")}</ul>` : ""}
      ${c.repairs?.length ? `<h4>Repaired by the engine</h4><ul class="plain" data-testid="repairs">${c.repairs.map((r) => `<li>${esc(r)}</li>`).join("")}</ul>` : ""}
      ${c.interlocked.length ? `<h4>Interlocked: these files depend on each other and go in together</h4>${c.interlocked.map((g) => `<div class="interlocked">${g.map(fileBtn).join("")}</div>`).join("")}` : ""}
      ${c.loose.length ? `<h4>Loose: nothing rests on these and they rest on nothing</h4><div class="interlocked">${c.loose.map(fileBtn).join("")}</div>` : ""}
    </section>
    <section>
      <h3>Chapters</h3>
      <ul class="plain">${d.sub_builds.map((s) => `<li><b>${esc(s.name)}</b> <span class="meta">${esc(s.package)}</span><br>${prose(s.blurb)}</li>`).join("")}</ul>
    </section>
    <details class="design-json"><summary>Design JSON</summary><pre>${esc(JSON.stringify(d, null, 2))}</pre></details>`;
  el.querySelector("[data-testid=design-run]")?.addEventListener("click", () => void runDesign());
  el.querySelector("[data-testid=design-reset]")?.addEventListener("click", () => void resetDesign());
  el.querySelector("[data-testid=check-gaps]")?.addEventListener("click", () => setShelfTab("endpoints"));
  el.querySelectorAll<HTMLButtonElement>("[data-jump]").forEach((x) => x.addEventListener("click", () => jumpTo(Number(x.dataset.jump))));
}
