// Bootstrap: renderer, panels, interaction, backend events, agent hooks.

import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { api, inTauri, log, on } from "./tauri";
import { store, subscribe, emit, select, setGraph, setPositions, setHover, snapshot } from "./store";
import { Renderer } from "./renderer";
import { initPanels, refreshRecent, renderFps, toast, toggleHelp, type Actions } from "./panels";
import { initBridge } from "./bridge";
import type { LayoutTick, ScanDone, ViewPayload } from "./types";

const canvas = document.getElementById("gl") as HTMLCanvasElement;
const labels = document.getElementById("labels") as HTMLCanvasElement;
const renderer = new Renderer(canvas, labels);

// ---- actions -----------------------------------------------------------------

let firstLoad = true;

async function loadView(level: "package" | "file" | "symbol", focus: number | null): Promise<void> {
  const t0 = performance.now();
  let payload: ViewPayload;
  try {
    payload = await api.getView(level, focus);
  } catch (e) {
    toast(`Cannot build the ${level} view: ${String(e)}`, "error");
    log("error", "get_view failed", { level, focus, error: String(e) });
    return;
  }
  store.repo = payload.root;
  store.stats = payload.stats;
  setGraph(payload.view, payload.positions, payload.level, payload.focus, payload.generation);
  store.layout = { running: true, backend: store.layout.backend, iteration: 0, energy: 1 };
  emit("layout");
  emit("repo");
  emit("ui");
  if (firstLoad || focus === null) renderer.fit();
  firstLoad = false;
  log("info", "view loaded", { level, focus, nodes: payload.view.nodes.length, edges: payload.view.edges.length, ms: Math.round(performance.now() - t0) });
}

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
  async setLevel(level, focus = null) {
    if (level === store.level && focus === store.focus && store.graphLoaded) return;
    firstLoad = firstLoad || level !== store.level;
    await loadView(level, focus);
  },
  async focusNode(id) {
    const i = store.index.get(id);
    const n = i !== undefined ? store.nodes[i] : null;
    if (!n) return;
    if (n.kind === "symbol") { select(id); actions.centerOn(id); return; }
    if (n.kind === "package" && store.level === "package") { await loadView("file", null); await selectAndCenter(id, true); return; }
    await loadView(store.level === "package" ? "package" : "file", id);
    // after expanding, the parent is gone from the view; select the first child if any
    const child = store.nodes.find((c) => c.id !== id && store.index.has(c.id) && c.group === n.group && c.kind !== n.kind);
    if (child) { select(child.id); actions.centerOn(child.id); }
  },
  centerOn(id) {
    const i = store.index.get(id);
    if (i === undefined) return;
    const x = store.positions[i * 2], y = store.positions[i * 2 + 1];
    animateCamera(x, y, Math.max(store.camera.zoom, 1.4));
  },
  relayout() {
    void api.runLayout(300).then(() => { store.layout.running = true; emit("layout"); toast("Settling the layout again", "info", 1600); }).catch((e) => toast(String(e), "error"));
  },
  fit() {
    renderer.fit();
  },
};

async function selectAndCenter(id: number, fallbackToChild = false): Promise<void> {
  if (store.index.has(id)) { select(id); actions.centerOn(id); return; }
  if (fallbackToChild) {
    const child = store.nodes.find((c) => c.group === id);
    if (child) { select(child.id); actions.centerOn(child.id); }
  }
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
    firstLoad = true;
    select(null);
    await loadView("file", null);
    toast(done.from_cache ? `Opened ${done.stats.files} files from cache. Press R to rescan.` : `Scanned ${done.stats.files} files, ${done.stats.flows} flows`, "ok");
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

// ---- camera ------------------------------------------------------------------

let camAnim = 0;
function animateCamera(x: number, y: number, zoom: number): void {
  cancelAnimationFrame(camAnim);
  const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const from = { ...store.camera };
  const start = performance.now();
  const dur = reduced ? 0 : 420;
  // aim for the free space between panels, not the window centre
  const offX = ((store.shelfOpen ? 330 : 40) - (store.selection !== null ? 370 : 40)) / 2;
  const step = (now: number) => {
    const t = dur === 0 ? 1 : Math.min(1, (now - start) / dur);
    const k = 1 - Math.pow(1 - t, 3);
    store.camera.zoom = from.zoom + (zoom - from.zoom) * k;
    store.camera.x = from.x + (x - offX / store.camera.zoom - from.x) * k;
    store.camera.y = from.y + (y - from.y) * k;
    emit("camera");
    if (t < 1) camAnim = requestAnimationFrame(step);
  };
  camAnim = requestAnimationFrame(step);
}

// ---- pointer interaction -----------------------------------------------------

let drag: { kind: "pan" | "node"; index: number; startX: number; startY: number; camX: number; camY: number; moved: boolean } | null = null;

canvas.addEventListener("pointerdown", (e) => {
  if (e.button !== 0) return;
  canvas.setPointerCapture(e.pointerId);
  const idx = renderer.pick(e.clientX, e.clientY);
  drag = { kind: idx >= 0 ? "node" : "pan", index: idx, startX: e.clientX, startY: e.clientY, camX: store.camera.x, camY: store.camera.y, moved: false };
  if (idx < 0) canvas.classList.add("is-grabbing");
});

canvas.addEventListener("pointermove", (e) => {
  if (drag) {
    const dx = e.clientX - drag.startX, dy = e.clientY - drag.startY;
    if (Math.abs(dx) + Math.abs(dy) > 3) drag.moved = true;
    if (drag.kind === "pan") {
      store.camera.x = drag.camX - dx / store.camera.zoom;
      store.camera.y = drag.camY - dy / store.camera.zoom;
      emit("camera");
    } else if (drag.moved) {
      const [wx, wy] = renderer.screenToWorld(e.clientX, e.clientY);
      store.positions[drag.index * 2] = wx;
      store.positions[drag.index * 2 + 1] = wy;
      emit("positions");
    }
    return;
  }
  const idx = renderer.pick(e.clientX, e.clientY);
  setHover(idx >= 0 ? store.nodes[idx].id : null);
  canvas.classList.toggle("is-node", idx >= 0);
});

canvas.addEventListener("pointerup", (e) => {
  if (!drag) return;
  canvas.classList.remove("is-grabbing");
  const d = drag;
  drag = null;
  if (d.kind === "node") {
    if (d.moved) {
      void api.setPosition(d.index, store.positions[d.index * 2], store.positions[d.index * 2 + 1]);
    } else {
      select(store.nodes[d.index].id);
    }
  } else if (!d.moved && e.detail === 1) {
    select(null);
  }
});

canvas.addEventListener("dblclick", (e) => {
  const idx = renderer.pick(e.clientX, e.clientY);
  if (idx >= 0) void actions.focusNode(store.nodes[idx].id);
});

canvas.addEventListener("wheel", (e) => {
  e.preventDefault();
  const cam = store.camera;
  if (e.ctrlKey || e.metaKey) {
    // pinch (ctrlKey on macOS trackpads) or cmd+scroll → zoom around the cursor
    const factor = Math.exp(-e.deltaY * (e.ctrlKey ? 0.012 : 0.0025));
    const [wx, wy] = renderer.screenToWorld(e.clientX, e.clientY);
    const zoom = Math.min(Math.max(cam.zoom * factor, 0.02), 12);
    cam.x = wx - (e.clientX - renderer.width / 2) / zoom;
    cam.y = wy - (e.clientY - renderer.height / 2) / zoom;
    cam.zoom = zoom;
  } else {
    cam.x += e.deltaX / cam.zoom;
    cam.y += e.deltaY / cam.zoom;
  }
  emit("camera");
}, { passive: false });

window.addEventListener("keydown", (e) => {
  const inInput = (e.target as HTMLElement)?.tagName === "INPUT";
  if (e.metaKey && e.key.toLowerCase() === "o") { e.preventDefault(); void actions.openRepo(); return; }
  if (e.metaKey && e.key.toLowerCase() === "k") { e.preventDefault(); (document.getElementById("search") as HTMLInputElement).focus(); return; }
  if (inInput) return;
  switch (e.key) {
    case "/": e.preventDefault(); (document.getElementById("search") as HTMLInputElement).focus(); break;
    case "1": void actions.setLevel("package"); break;
    case "2": void actions.setLevel("file"); break;
    case "3": void actions.setLevel("symbol"); break;
    case "f": case "F": renderer.fit(); break;
    case "l": case "L": actions.relayout(); break;
    case "r": case "R": if (store.repo) void scan(store.repo, true); break;
    case "?": toggleHelp(); break;
    case "Escape": if (store.selection !== null) select(null); else toggleHelp(false); break;
    case "[": store.shelfOpen = !store.shelfOpen; emit("ui"); break;
  }
});

// ---- backend events ----------------------------------------------------------

void on<LayoutTick>("layout:tick", (tick) => {
  if (!setPositions(tick.positions, tick.generation)) return;
  store.layout = { running: !tick.done, backend: tick.backend, iteration: tick.iteration, energy: tick.energy };
  emit("layout");
  if (tick.done && firstLoadFit) { renderer.fit(); firstLoadFit = false; }
});
let firstLoadFit = false;
subscribe("graph", () => { firstLoadFit = store.focus === null; });

void on<{ path: string }>("scan:started", ({ path }) => { store.scanning = path; emit("ui"); });
void on<{ path: string; error: string }>("scan:error", ({ error }) => { store.scanning = null; emit("ui"); toast(`Scan failed: ${error}`, "error", 6000); });
// A scan started from the bridge (not from this UI) still needs the view loaded here.
void on<ScanDone>("scan:done", () => {
  if (scanStartedHere) return; // scan() loads the view itself
  {
    store.scanning = null;
    firstLoad = true;
    select(null);
    void loadView("file", null);
  }
});

if (inTauri) {
  void getCurrentWebview().onDragDropEvent((event) => {
    if (event.payload.type === "drop" && event.payload.paths.length) void scan(event.payload.paths[0], false);
  });
}

// ---- reporting (keeps /state and /metrics fresh even without a round trip) ------

let lastReport = "";
setInterval(() => {
  if (!inTauri) return;
  const snap = snapshot();
  const key = JSON.stringify([snap.selection, snap.level, snap.focus, snap.hover, snap.camera, snap.filters, snap.panels, snap.graph_loaded]);
  if (key !== lastReport) {
    lastReport = key;
    const { camera, selection, hover, level, focus, graph_loaded, search, filters, panels, nodes_visible, edges_visible, ts } = snap as Record<string, never>;
    void api.reportUi({ camera, selection, hover, level, focus, graph_loaded, search, filters, panels, nodes_visible, edges_visible, ts }).catch(() => {});
  }
}, 500);
setInterval(() => {
  const s = renderer.stats();
  renderFps(s.fps, s.frame_ms_p50);
  if (inTauri) void api.reportMetrics({ ...s, ts: new Date().toISOString() }).catch(() => {});
}, 1000);

// ---- boot ----------------------------------------------------------------------

initPanels(actions);
initBridge(renderer, actions);

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
  log("info", "ui ready", { renderer: renderer.rendererName, dpr: window.devicePixelRatio });
})();
