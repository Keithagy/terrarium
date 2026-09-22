// Bootstrap: brick scene, panels, manual, backend events, keyboard, agent hooks.

import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { api, inTauri, log, on } from "./tauri";
import { store, subscribe, emit, select, setBuild, setHover, setStep, setTab, stepCount, snapshot, traceBuildings, buildingForNode, buildingAt, type Tab } from "./store";
import { BrickScene, type Hit } from "./bricks";
import { initPanels, refreshRecent, setShelfTab, toast, toggleHelp, type Actions } from "./panels";
import { initBridge } from "./bridge";
import { initTraces, loadTraces, stepTrace } from "./traces";
import { initManual, stop, togglePlay } from "./manual";
import { initParts } from "./parts";
import { initDesign } from "./design";
import type { ScanDone } from "./types";

const scene = new BrickScene(document.getElementById("scene")!);

// ---- the scene follows the store ----------------------------------------------

let lastStep = -1;
subscribe("build", () => {
  const b = store.build!;
  scene.setModel(b.model, b.design.steps.length);
  lastStep = store.step;
  applyHighlight();
  applySelection();
});
subscribe("step", () => {
  // One step forward (playing or pressing next) drops the new pieces in; jumps just cut.
  const animate = store.step === lastStep + 1;
  scene.setStep(store.step - 1, { animate });
  lastStep = store.step;
});
subscribe("selection", applySelection);
subscribe("trace", applyHighlight);
let lastTab = store.tab;
subscribe("tab", () => {
  document.body.dataset.tab = store.tab;
  // Model and Manual give the scene different room; frame it again once the box has resized.
  const room = (t: Tab) => (t === "manual" ? "narrow" : t === "model" ? "wide" : "none");
  if (room(store.tab) !== room(lastTab) && room(store.tab) !== "none") requestAnimationFrame(() => requestAnimationFrame(() => scene.fit()));
  lastTab = store.tab;
  document.querySelectorAll<HTMLElement>("[data-stage-tab]").forEach((b) => b.classList.toggle("is-active", b.dataset.stageTab === store.tab));
  log("info", "tab", { tab: store.tab });
  emit("ui");
});
subscribe("view", () => {
  document.querySelectorAll<HTMLElement>("[data-view]").forEach((b) => b.classList.toggle("is-active", b.dataset.view === store.view));
  document.getElementById("spin-btn")!.classList.toggle("is-active", store.spin);
});

function applySelection(): void {
  const id = store.selection;
  if (id === null || !store.build) { scene.select(null); return; }
  const building = buildingForNode(id);
  if (building === null) { scene.select(null); return; }
  const brick = store.brickOf.get(id) ?? store.build.model.bricks.findIndex((br) => br.building === building);
  scene.select({ building, brick });
}

function applyHighlight(): void {
  scene.highlight(store.trace && store.traceOnModel ? traceBuildings() : null);
}

scene.onPick((hit: Hit | null) => {
  if (!hit || !store.build) { select(null); return; }
  const b = store.build.model;
  const brick = b.bricks[hit.brick];
  const building = b.buildings[hit.building];
  // In the manual, a brick is a way back to the step that added it.
  if (store.tab === "manual") { stop(); setStep(building.step + 1); }
  select(brick ? brick.nodes[0] : building.id);
});
scene.onHover((hit) => {
  const b = store.build?.model;
  setHover(hit && b ? b.bricks[hit.brick]?.nodes[0] ?? b.buildings[hit.building].id : null);
  document.getElementById("scene")!.classList.toggle("is-pointing", !!hit);
});

// ---- actions ---------------------------------------------------------------------

const actions: Actions = {
  async openRepo(path?: string) {
    if (!path) {
      if (!inTauri) return;
      const picked = await openDialog({ directory: true, multiple: false, title: "Open a repository" });
      if (!picked) return;
      path = Array.isArray(picked) ? picked[0] : picked;
    }
    if (!path) return;
    await scan(path, false);
  },
  focusBuilding(index) {
    const b = buildingAt(index);
    if (b) scene.focusDistrict(b.district);
  },
  focusDistrict(index) {
    scene.focusDistrict(index);
  },
  showTraceOnModel() {
    select(null);
    store.traceOnModel = true;
    setStep(stepCount());
    setTab("model");
    emit("trace");
  },
};

async function loadBuild(firstForRepo: boolean): Promise<void> {
  const t0 = performance.now();
  try {
    setBuild(await api.getBuild());
  } catch (e) {
    toast(`Cannot build the model: ${String(e)}`, "error");
    log("error", "get_build failed", { error: String(e) });
    return;
  }
  if (firstForRepo) {
    select(null);
    setTab("model");
    scene.fit();
  }
  await loadTraces(firstForRepo);
  emit("repo");
  emit("ui");
  const b = store.build!;
  log("info", "build loaded", { source: b.design.source, steps: b.check.steps, pieces: b.check.pieces, weak: b.check.weak.length, ms: Math.round(performance.now() - t0) });
}

let scanStartedHere = false;

async function scan(path: string, fresh: boolean): Promise<void> {
  store.scanning = path;
  scanStartedHere = true;
  emit("ui");
  const t0 = performance.now();
  try {
    const done: ScanDone = await api.scanRepo(path, fresh);
    log("info", "scan done", { path, from_cache: done.from_cache, files: done.stats.files, ms: Math.round(performance.now() - t0) });
    store.scanning = null;
    store.repo = done.root;
    store.stats = done.stats;
    await loadBuild(true);
    const b = store.build;
    toast(done.from_cache ? `Opened ${done.stats.files} files from cache. Press R to rescan.` : `Built ${done.stats.files} files in ${b?.check.steps ?? 0} steps`, "ok");
    void refreshRecent();
  } catch (e) {
    store.scanning = null;
    emit("ui");
    toast(`Scan failed: ${String(e)}`, "error", 6000);
    log("error", "scan failed", { path, error: String(e) });
  } finally {
    scanStartedHere = false;
  }
}

// ---- controls ----------------------------------------------------------------------

document.querySelectorAll<HTMLButtonElement>("[data-stage-tab]").forEach((b) => b.addEventListener("click", () => setTab(b.dataset.stageTab as Tab)));
document.querySelectorAll<HTMLButtonElement>("[data-view]").forEach((b) => b.addEventListener("click", () => setView(b.dataset.view as typeof store.view)));
document.getElementById("spin-btn")!.addEventListener("click", () => setSpin(!store.spin));
document.getElementById("fit-btn")!.addEventListener("click", () => scene.fit());

function setView(v: typeof store.view): void {
  store.view = v;
  scene.setView(v);
  emit("view");
}

function setSpin(on: boolean): void {
  store.spin = on;
  scene.setSpin(on);
  emit("view");
}

window.addEventListener("keydown", (e) => {
  const inInput = (e.target as HTMLElement)?.tagName === "INPUT";
  if (e.metaKey && e.key.toLowerCase() === "o") { e.preventDefault(); void actions.openRepo(); return; }
  if (e.metaKey && e.key.toLowerCase() === "k") { e.preventDefault(); (document.getElementById("search") as HTMLInputElement).focus(); return; }
  if (inInput || e.metaKey || e.ctrlKey) return;
  const tabs: Record<string, Tab> = { m: "model", n: "manual", p: "parts", t: "traces", d: "design" };
  const k = e.key.toLowerCase();
  if (tabs[k]) { setTab(tabs[k]); return; }
  switch (e.key) {
    case "/": e.preventDefault(); (document.getElementById("search") as HTMLInputElement).focus(); break;
    case " ": e.preventDefault(); togglePlay(); break;
    case "ArrowRight": stop(); setStep(store.step + 1); break;
    case "ArrowLeft": stop(); setStep(store.step - 1); break;
    case "Home": stop(); setStep(0); break;
    case "End": stop(); setStep(stepCount()); break;
    case "1": setView("iso"); break;
    case "2": setView("front"); break;
    case "3": setView("top"); break;
    case "s": case "S": setSpin(!store.spin); break;
    case "f": case "F": scene.fit(); break;
    case "j": case "J": stepTrace(1); break;
    case "k": case "K": stepTrace(-1); break;
    case "r": case "R": if (store.repo) void scan(store.repo, true); break;
    case "?": toggleHelp(); break;
    case "Escape": if (store.selection !== null) select(null); else toggleHelp(false); break;
    case "[": store.shelfOpen = !store.shelfOpen; emit("ui"); break;
  }
});

// ---- backend events ----------------------------------------------------------------

void on<{ path: string }>("scan:started", ({ path }) => { store.scanning = path; emit("ui"); });
void on<{ path: string; error: string }>("scan:error", ({ error }) => { store.scanning = null; emit("ui"); toast(`Scan failed: ${error}`, "error", 6000); });
// A scan started from the bridge (not from this UI) still needs the build loaded here.
void on<ScanDone>("scan:done", (done) => {
  if (scanStartedHere) return; // scan() loads it itself
  store.scanning = null;
  store.repo = done.root;
  store.stats = done.stats;
  void loadBuild(true);
});

if (inTauri) {
  void getCurrentWebview().onDragDropEvent((event) => {
    if (event.payload.type === "drop" && event.payload.paths.length) void scan(event.payload.paths[0], false);
  });
}

// ---- reporting (keeps /state and /metrics fresh even without a round trip) ---------

let lastReport = "";
setInterval(() => {
  if (!inTauri) return;
  const snap = snapshot();
  const { ts, ...rest } = snap;
  const key = JSON.stringify(rest);
  if (key !== lastReport) {
    lastReport = key;
    void api.reportUi({ ...rest, ts }).catch(() => {});
  }
}, 500);
setInterval(() => {
  if (inTauri) void api.reportMetrics({ ...scene.stats(), ts: new Date().toISOString() }).catch(() => {});
}, 1000);

// ---- boot --------------------------------------------------------------------------

initPanels(actions);
initTraces({ showOnModel: () => actions.showTraceOnModel() });
initManual();
initParts();
initDesign();
initBridge(scene, actions);
setShelfTab("sub-builds");

(async () => {
  if (!inTauri) { log("warn", "running outside Tauri: open the app with `terrarium app launch`"); return; }
  try {
    const info = await api.bridgeInfo();
    store.bridgePort = info.port || null;
    emit("ui");
    if (!info.port) setTimeout(async () => { store.bridgePort = (await api.bridgeInfo()).port || null; emit("ui"); }, 1500);
  } catch (e) { log("warn", `bridge info failed: ${String(e)}`); }
  try {
    const initial = await api.initialRepo();
    if (initial) await scan(initial, false);
  } catch (e) { log("warn", `initial repo failed: ${String(e)}`); }
  log("info", "ui ready", { renderer: scene.rendererName, dpr: window.devicePixelRatio });
})();
