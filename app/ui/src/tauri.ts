// Typed wrappers around the Tauri IPC surface, plus a logger that lands in the
// backend's ring buffer so agents see frontend and backend events in one stream.

import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Boundary, CacheEntry, Endpoint, FlowRow, NodeDetail, ScanDone, Trace, ViewPayload } from "./types";

export const inTauri = isTauri();

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!inTauri) throw new Error(`not running inside Tauri (cannot invoke ${cmd})`);
  const t0 = performance.now();
  try {
    return await invoke<T>(cmd, args);
  } finally {
    const ms = performance.now() - t0;
    if (ms > 250) log("debug", `slow ipc ${cmd}`, { ms: Math.round(ms) });
  }
}

export const api = {
  scanRepo: (path: string, fresh = false) => call<ScanDone>("scan_repo", { path, fresh }),
  getView: (level: string, focus: number | null, backend?: string) => call<ViewPayload>("get_view", { level, focus, backend }),
  runLayout: (iterations?: number, backend?: string) => call<unknown>("run_layout", { iterations, backend }),
  stopLayout: () => call<void>("stop_layout"),
  setPosition: (index: number, x: number, y: number) => call<void>("set_position", { index, x, y }),
  getNode: (id: number) => call<NodeDetail>("get_node", { id }),
  search: (q: string, limit = 30) => call<{ id: number; name: string; path: string; kind: string; lang: string; tags: string[] }[]>("search_nodes", { q, limit }),
  flows: () => call<FlowRow[]>("list_flows"),
  traces: () => call<Trace[]>("list_traces"),
  trace: (entry: number) => call<Trace>("get_trace", { entry }),
  endpoints: () => call<Endpoint[]>("list_endpoints"),
  boundaries: (tag?: string) => call<Boundary[]>("list_boundaries", { tag }),
  recent: () => call<CacheEntry[]>("recent_repos"),
  reportUi: (report: unknown) => call<void>("report_ui", { report }),
  reportMetrics: (metrics: unknown) => call<void>("report_metrics", { metrics }),
  bridgeReply: (id: number, result: unknown) => call<void>("bridge_reply", { id, result }),
  bridgeInfo: () => call<{ port: number; pid: number; version: string }>("bridge_info"),
  openPath: (path: string, line?: number) => call<void>("open_path", { path, line }),
  initialRepo: () => call<string | null>("initial_repo"),
};

export function on<T>(event: string, cb: (payload: T) => void): Promise<UnlistenFn> {
  if (!inTauri) return Promise.resolve(() => {});
  return listen<T>(event, (e) => cb(e.payload));
}

export type LogLevel = "debug" | "info" | "warn" | "error";

const pending: { level: LogLevel; message: string; fields: unknown }[] = [];
let flushing = false;

export function log(level: LogLevel, message: string, fields: Record<string, unknown> = {}): void {
  const line = `[ui] ${message}`;
  if (level === "error") console.error(line, fields);
  else if (level === "warn") console.warn(line, fields);
  else console.log(line, fields);
  if (!inTauri) return;
  pending.push({ level, message, fields });
  if (!flushing) void flush();
}

async function flush(): Promise<void> {
  flushing = true;
  while (pending.length) {
    const e = pending.shift()!;
    try {
      await invoke("log_event", { entry: { level: e.level, target: "ui", message: e.message, fields: e.fields } });
    } catch {
      /* backend gone */
    }
  }
  flushing = false;
}

window.addEventListener("error", (e) => log("error", `uncaught: ${e.message}`, { file: e.filename, line: e.lineno }));
window.addEventListener("unhandledrejection", (e) => log("error", `unhandled rejection: ${String(e.reason)}`));
