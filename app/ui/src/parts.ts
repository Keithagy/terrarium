// The parts inventory: what the model is made of, like the parts list at the
// front of a brick set. Bricks grouped by language and kind, then per sub-build.

import { store, subscribe } from "./store";
import { LANG_LABEL, type Lang } from "./types";
import { esc, jumpTo } from "./panels";

const $ = <T extends HTMLElement>(sel: string): T => document.querySelector(sel) as T;

const KIND_LABEL: Record<string, string> = {
  function: "function", method: "method", class: "class", struct: "struct", enum: "enum",
  trait: "trait", interface: "interface", type: "type", const: "constant", module: "module",
};

export function initParts(): void {
  subscribe("build", render);
  subscribe("tab", render);
}

function render(): void {
  const el = $("#parts-view");
  el.hidden = store.tab !== "parts" || !store.build;
  if (el.hidden) return;
  const b = store.build!;
  const m = b.model;
  // (lang, kind) -> symbol count
  const tally = new Map<string, { lang: Lang; kind: string; count: number }>();
  for (const br of m.bricks) {
    const lang = m.buildings[br.building].lang;
    const kind = br.kind ? KIND_LABEL[br.kind] ?? br.kind : "file";
    const key = `${lang}|${kind}`;
    const t = tally.get(key) ?? { lang, kind, count: 0 };
    t.count += br.nodes.length;
    tally.set(key, t);
  }
  const tiles = [...tally.values()].sort((a, c) => c.count - a.count).map((t) => `
    <div class="part-tile" data-testid="parts-${t.lang}-${t.kind}">
      <span class="brick-3d" data-lang="${t.lang}"><i></i><i></i></span>
      <b>${t.count}×</b>
      <span>${esc(t.kind)}</span>
      <span class="meta">${esc(LANG_LABEL[t.lang])}</span>
    </div>`).join("");
  const rows = b.design.sub_builds.map((sb) => {
    const di = m.districts.findIndex((d) => d.sub_build === sb.id);
    const blds = m.buildings.filter((x) => x.district === di);
    const bricks = m.bricks.filter((br) => m.buildings[br.building].district === di);
    const floors = blds.reduce((sum, x) => sum + x.layers, 0);
    return `<tr><td><span class="dot" data-lang="${m.districts[di]?.lang ?? "other"}"></span> ${esc(sb.name)}</td><td>${blds.length}</td><td>${bricks.reduce((s, br) => s + br.nodes.length, 0)}</td><td>${floors}</td><td>${b.design.steps.filter((s) => s.sub_build === sb.id).length}</td></tr>`;
  }).join("");
  const tallest = [...m.buildings].map((x, i) => [x, i] as const).sort((a, c) => c[0].layers - a[0].layers).slice(0, 8);
  el.innerHTML = `
    <div class="stage-head">
      <h2>Parts</h2>
      <p class="meta">${b.check.pieces.toLocaleString()} bricks in ${m.buildings.length} buildings across ${m.districts.length} sub-builds, on a ${m.studs[0]}×${m.studs[1]} baseplate. A brick is a function, type or constant; its colour is the language.</p>
    </div>
    <h3>By kind</h3>
    <div class="part-tiles">${tiles}</div>
    <h3>By sub-build</h3>
    <table class="bom" data-testid="parts-table"><thead><tr><th>Sub-build</th><th>Buildings</th><th>Parts</th><th>Floors</th><th>Steps</th></tr></thead><tbody>${rows}</tbody></table>
    <h3>Tallest buildings</h3>
    <div class="tallest">${tallest.map(([x]) => `<button class="row" data-jump="${x.id}"><span class="brick-swatch" data-lang="${x.lang}"></span><span class="name">${esc(x.name)}</span><span class="meta mono">${esc(x.path)}</span><span class="meta">${x.layers} ${x.layers === 1 ? "floor" : "floors"}</span></button>`).join("")}</div>`;
  el.querySelectorAll<HTMLButtonElement>("[data-jump]").forEach((x) => x.addEventListener("click", () => jumpTo(Number(x.dataset.jump))));
}
