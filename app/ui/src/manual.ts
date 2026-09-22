// The manual: the timeline that plays the build step by step, and the page that
// explains the current step (what it adds, what that rests on, where it leads).

import { store, subscribe, emit, setStep, setTab, stepCount, select } from "./store";
import { LANG_LABEL } from "./types";
import { esc, prose } from "./panels";
import { log } from "./tauri";

const $ = <T extends HTMLElement>(sel: string): T => document.querySelector(sel) as T;

let timer = 0;

export function initManual(): void {
  const range = $<HTMLInputElement>("#tl-range");
  range.addEventListener("input", () => { stop(); setStep(Number(range.value)); });
  $("#tl-first").addEventListener("click", () => { stop(); setStep(0); });
  $("#tl-last").addEventListener("click", () => { stop(); setStep(stepCount()); });
  $("#tl-play").addEventListener("click", () => togglePlay());
  $("#tl-status").addEventListener("click", () => setTab("manual"));
  document.querySelectorAll<HTMLButtonElement>("[data-speed]").forEach((b) =>
    b.addEventListener("click", () => {
      store.speed = Number(b.dataset.speed);
      document.querySelectorAll("[data-speed]").forEach((x) => x.classList.toggle("is-active", x === b));
      if (store.playing) { stop(); play(); }
    }),
  );
  subscribe("build", () => { stop(); renderTimeline(); renderPage(); });
  subscribe("step", () => { renderTimeline(); renderPage(); });
  subscribe("tab", renderPage);
}

export function togglePlay(): void {
  if (store.playing) stop(); else play();
}

function play(): void {
  const n = stepCount();
  if (!n) return;
  if (store.step >= n) setStep(0);
  store.playing = true;
  log("info", "build playing", { from: store.step, speed: store.speed });
  const tick = () => {
    if (store.step >= stepCount()) { stop(); return; }
    setStep(store.step + 1);
    timer = window.setTimeout(tick, 1100 / store.speed);
  };
  timer = window.setTimeout(tick, 250 / store.speed);
  renderTimeline();
}

export function stop(): void {
  clearTimeout(timer);
  if (!store.playing) return;
  store.playing = false;
  renderTimeline();
  emit("ui");
}

function renderTimeline(): void {
  const b = store.build;
  const n = stepCount();
  const range = $<HTMLInputElement>("#tl-range");
  range.max = String(n);
  range.value = String(store.step);
  range.disabled = !b;
  range.style.setProperty("--fill", n ? `${(store.step / n) * 100}%` : "0%");
  $("#tl-play").textContent = store.playing ? "❚❚" : "▶";
  $("#tl-play").title = store.playing ? "Pause (Space)" : "Play the build (Space)";
  const title = $("#tl-title");
  const meta = $("#tl-meta");
  const status = $("#tl-status");
  if (!b) { title.textContent = "No build yet"; meta.textContent = ""; status.textContent = ""; return; }
  if (store.step === 0) {
    title.textContent = "Empty baseplate";
    meta.textContent = `${n} steps to build`;
    status.textContent = "Start";
  } else if (store.step >= n && !store.playing) {
    title.textContent = "Finished model";
    meta.textContent = `${b.check.pieces.toLocaleString()} pieces · ${n} steps`;
    status.textContent = "Finished";
  } else {
    const s = b.design.steps[store.step - 1];
    title.textContent = `Step ${store.step} of ${n}`;
    meta.textContent = s.title;
    status.textContent = `Step ${store.step}`;
  }
}

function renderPage(): void {
  const el = $("#page");
  el.hidden = store.tab !== "manual" || !store.build;
  if (el.hidden) return;
  const b = store.build!;
  const d = b.design;
  const n = d.steps.length;
  const chapterOf = (id: string) => d.sub_builds.find((s) => s.id === id);
  let body: string;
  if (store.step === 0) {
    body = `
      <div class="page-kicker">Build manual</div>
      <h2 class="page-title" data-testid="page-title">${esc(d.title)}</h2>
      <p class="page-caption">${prose(d.summary)}</p>
      <p class="page-note">${d.source === "engine" ? "Grouped and captioned by the engine. Design with Claude to have agents read the code and write each step." : `Written by ${esc(d.model ?? d.source)}, checked joint by joint by the engine.`}</p>
      <h3>Chapters</h3>
      <ol class="chapters">${d.sub_builds.map((s) => {
        const first = d.steps.findIndex((x) => x.sub_build === s.id);
        return first < 0 ? "" : `<li><button data-goto="${first + 1}" data-testid="chapter-${esc(s.id)}"><b>${esc(s.name)}</b><span>${prose(s.blurb)}</span><span class="meta">from step ${first + 1}</span></button></li>`;
      }).join("")}</ol>`;
  } else {
    const k = Math.min(store.step, n);
    const s = d.steps[k - 1];
    const ch = chapterOf(s.sub_build);
    const byPath = new Map(b.model.buildings.map((x, i) => [x.path, i]));
    const added = s.files.map((p) => byPath.get(p)).filter((i): i is number => i !== undefined);
    const parts = added.map((i) => {
      const bl = b.model.buildings[i];
      const bricks = b.model.bricks.filter((br) => br.building === i).sort((x, y) => y.layer - x.layer);
      return `<div class="part" data-testid="part-${bl.id}">
        <button class="part-head" data-select="${bl.id}"><span class="brick-swatch" data-lang="${bl.lang}"></span><b>${esc(bl.name)}</b><span class="meta">${esc(LANG_LABEL[bl.lang])} · ${bricks.length} ${bricks.length === 1 ? "brick" : "bricks"}${bl.lamp ? " · lamp" : ""}</span></button>
        <div class="part-path">${esc(bl.path)}</div>
        <div class="part-bricks">${bricks.map((br) => `<button class="brick-chip" data-select="${br.nodes[0]}" data-lang="${bl.lang}" title="${esc(br.kind ?? "file")}${br.sinks?.length ? ` · ${br.sinks.join(", ")}` : ""}">${esc(br.name)}${br.sinks?.length ? `<span class="sink">${esc(br.sinks.join(" "))}</span>` : ""}</button>`).join("")}</div>
      </div>`;
    }).join("");
    const rests = [...new Set(added.flatMap((i) => b.model.buildings[i].rests_on))].filter((i) => !added.includes(i));
    const restsHtml = rests.length
      ? rests.map((i) => { const x = b.model.buildings[i]; return `<button class="rest-chip" data-goto="${x.step + 1}" title="${esc(x.path)}"><span class="dot" data-lang="${x.lang}"></span>${esc(x.name)}<span class="meta">step ${x.step + 1}</span></button>`; }).join("")
      : `<span class="meta">Nothing already built: this step stands on the baseplate.</span>`;
    const bridges = b.model.bridges.filter((br) => br.step === k - 1);
    body = `
      <div class="page-kicker"><span class="step-no">${k}</span><span>${esc(ch?.name ?? s.sub_build)}</span><span class="meta">step ${k} of ${n}</span></div>
      <h2 class="page-title" data-testid="page-title">${esc(s.title)}</h2>
      <p class="page-caption" data-testid="page-caption">${prose(s.caption)}</p>
      <h3>Parts for this step</h3>
      <div class="parts">${parts}</div>
      <h3>Rests on</h3>
      <div class="rests">${restsHtml}</div>
      ${bridges.length ? `<h3>Bridges completed</h3><div class="rests">${bridges.map((br) => `<span class="chip is-boundary">${esc(br.label)}</span>`).join("")}</div>` : ""}`;
  }
  el.innerHTML = `
    <div class="page-body">${body}</div>
    <nav class="page-nav">
      <button class="ghost" data-testid="page-prev" ${store.step <= 0 ? "disabled" : ""}>← Previous</button>
      <span class="meta">${store.step === 0 ? "Cover" : `${Math.min(store.step, n)} / ${n}`}</span>
      <button class="primary" data-testid="page-next" ${store.step >= n ? "disabled" : ""}>${store.step === 0 ? "Start building" : "Next step →"}</button>
    </nav>`;
  el.querySelector("[data-testid=page-prev]")!.addEventListener("click", () => { stop(); setStep(store.step - 1); });
  el.querySelector("[data-testid=page-next]")!.addEventListener("click", () => { stop(); setStep(store.step + 1); });
  el.querySelectorAll<HTMLButtonElement>("[data-goto]").forEach((x) => x.addEventListener("click", () => { stop(); setStep(Number(x.dataset.goto)); }));
  el.querySelectorAll<HTMLButtonElement>("[data-select]").forEach((x) => x.addEventListener("click", () => select(Number(x.dataset.select))));
}
