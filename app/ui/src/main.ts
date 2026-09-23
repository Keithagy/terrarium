// Bootstrap: the diagram, the panels, backend events, keyboard, agent hooks.

import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { api, inTauri, log, on } from "./tauri";
import { store, emit, select, setAtlas, setLevel, setJourney, setJourneyStep, setView, startDraft, endDraft, zoomInto, zoomOut, snapshot, elementById } from "./store";
import { initPanels, refreshRecent, setShelfTab, toast, toggleHelp, type Actions } from "./panels";
import { initAtlas, goLevel } from "./atlas";
import { initDiscovery } from "./discovery";
import { initBridge } from "./bridge";
import { initSequence } from "./sequence";
import type { ScanDone } from "./types";

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
  async copyDsl() {
    try {
      const dsl = await api.atlasDsl();
      await navigator.clipboard.writeText(dsl);
      toast("Copied the atlas as Structurizr DSL", "ok");
    } catch (e) { toast(`Cannot copy: ${String(e)}`, "error"); }
  },
};

async function loadAtlas(firstForRepo: boolean): Promise<void> {
  const t0 = performance.now();
  try {
    const v = await api.getAtlas();
    if (firstForRepo) { store.level = "containers"; store.focus = null; store.selection = null; store.relSelection = null; store.journey = null; store.notesOpen = false; }
    setAtlas(v.atlas, v.stale ?? null);
  } catch (e) {
    toast(`Cannot load the atlas: ${String(e)}`, "error");
    log("error", "get_atlas failed", { error: String(e) });
    return;
  }
  emit("repo");
  emit("ui");
  const a = store.atlas!;
  log("info", "atlas loaded", { source: a.source, containers: a.containers.length, relationships: a.relationships.length, backed: a.report.backed, claimed: a.report.claimed, ms: Math.round(performance.now() - t0) });
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
    await loadAtlas(true);
    const a = store.atlas;
    toast(done.from_cache ? `Opened ${done.stats.files} files from cache. Press R to rescan.` : `Surveyed ${done.stats.files} files into ${a?.containers.filter((c) => !c.hidden).length ?? 0} containers`, "ok");
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

// ---- keyboard ----------------------------------------------------------------------

window.addEventListener("keydown", (e) => {
  const inInput = ["INPUT", "TEXTAREA", "SELECT"].includes((e.target as HTMLElement)?.tagName);
  if (e.metaKey && e.key.toLowerCase() === "o") { e.preventDefault(); void actions.openRepo(); return; }
  if (e.metaKey && e.key.toLowerCase() === "k") { e.preventDefault(); (document.getElementById("search") as HTMLInputElement).focus(); return; }
  if (inInput || e.metaKey || e.ctrlKey) return;
  switch (e.key) {
    case "/": e.preventDefault(); (document.getElementById("search") as HTMLInputElement).focus(); break;
    case "1": goLevel("context"); break;
    case "2": goLevel("containers"); break;
    case "3": goLevel("components"); break;
    case "4": goLevel("code"); break;
    case "Enter": if (store.selection) zoomInto(store.selection); break;
    case "Backspace": zoomOut(); break;
    case "ArrowRight": if (store.journey) setJourneyStep(store.journeyStep + 1); break;
    case "ArrowLeft": if (store.journey) setJourneyStep(store.journeyStep - 1); break;
    case "j": case "J": {
      const js = store.atlas?.journeys ?? [];
      if (!js.length) break;
      const i = store.journey ? js.findIndex((x) => x.id === store.journey!.id) : -1;
      setJourney(js[(i + 1) % js.length]);
      setShelfTab("journeys");
      break;
    }
    case "s": case "S": if (store.journey) setView(store.view === "map" ? "sequence" : "map"); break;
    case "e": case "E": if (store.journey && !store.draft && !store.narrating) startDraft(); break;
    case "f": case "F": document.getElementById("fit-btn")?.click(); break;
    case "r": case "R": if (store.repo) void scan(store.repo, true); break;
    case "?": toggleHelp(); break;
    case "Escape":
      if (!document.getElementById("plan")!.hidden) { document.getElementById("plan-cancel")?.click(); }
      else if (store.draft) endDraft();
      else if (store.selection !== null || store.relSelection !== null) select(null);
      else if (store.journey) setJourney(null);
      else if (store.notesOpen) { store.notesOpen = false; emit("discovery"); emit("ui"); }
      else toggleHelp(false);
      break;
    case "[": store.shelfOpen = !store.shelfOpen; emit("ui"); break;
  }
});

// ---- backend events ----------------------------------------------------------------

void on<{ path: string }>("scan:started", ({ path }) => { store.scanning = path; emit("ui"); });
void on<{ path: string; error: string }>("scan:error", ({ error }) => { store.scanning = null; emit("ui"); toast(`Scan failed: ${error}`, "error", 6000); });
// A scan started from the bridge (not from this UI) still needs the atlas loaded here.
void on<ScanDone>("scan:done", (done) => {
  if (scanStartedHere) return; // scan() loads it itself
  store.scanning = null;
  store.repo = done.root;
  store.stats = done.stats;
  void loadAtlas(true);
});

if (inTauri) {
  void getCurrentWebview().onDragDropEvent((event) => {
    if (event.payload.type === "drop" && event.payload.paths.length) void scan(event.payload.paths[0], false);
  });
}

// ---- reporting (keeps /state fresh even without a round trip) ----------------------

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
let frames = 0;
let lastFrames = performance.now();
const countFrame = () => { frames++; requestAnimationFrame(countFrame); };
requestAnimationFrame(countFrame);
setInterval(() => {
  if (!inTauri) return;
  const now = performance.now();
  const fps = (frames * 1000) / (now - lastFrames);
  frames = 0;
  lastFrames = now;
  void api.reportMetrics({ fps: Math.round(fps), frame_ms_p50: 0, frame_ms_p95: 0, elements_drawn: document.querySelectorAll("#diagram g.node, #diagram g.edge").length, renderer: "svg", ts: new Date().toISOString() }).catch(() => {});
}, 1000);

// ---- boot --------------------------------------------------------------------------

initPanels(actions);
initAtlas();
initSequence();
initDiscovery();
initBridge(actions);
setShelfTab("map");
void elementById;
void setLevel;

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
  log("info", "ui ready", { dpr: window.devicePixelRatio });
})();
