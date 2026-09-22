//! Tauri commands: the frontend's view of the backend. The agent bridge reuses these.

use crate::layout_runner;
use crate::state::{AppState, FrameMetrics, UiReport, ViewState, level_name, parse_level};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, State};
use terrarium_core::{Graph, NodeId, NodeKind, ScanOptions, cache, query};
use terrarium_layout::Backend;

type Res<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[derive(Serialize, Clone)]
pub struct ScanDone {
    pub root: String,
    pub stats: terrarium_core::Stats,
    pub cached: String,
    pub from_cache: bool,
}

/// Scan (or load from cache when `fresh` is false and a cache exists), install as the current graph.
pub fn do_scan(app: &AppHandle, path: &str, fresh: bool) -> anyhow::Result<ScanDone> {
    let state = app.state::<Arc<AppState>>().inner().clone();
    if state.scanning.swap(true, Ordering::SeqCst) {
        anyhow::bail!("a scan is already running");
    }
    let _ = app.emit("scan:started", json!({ "path": path }));
    let result = (|| {
        let root = Path::new(path);
        let (graph, from_cache) = match (fresh, cache::load(root)) {
            (false, Some(g)) => (g, true),
            _ => (terrarium_core::scan(root, &ScanOptions::default())?, false),
        };
        let cached = if from_cache {
            cache::graph_file(root)
        } else {
            cache::store(&graph)?
        };
        state.count(&state.counters.scans);
        let done = ScanDone {
            root: graph.root.clone(),
            stats: graph.stats.clone(),
            cached: cached.to_string_lossy().to_string(),
            from_cache,
        };
        *state.graph.write().unwrap() = Some(Arc::new(graph));
        *state.view.write().unwrap() = None;
        Ok::<_, anyhow::Error>(done)
    })();
    state.scanning.store(false, Ordering::SeqCst);
    match &result {
        Ok(d) => {
            let _ = app.emit("scan:done", d.clone());
        }
        Err(e) => {
            tracing::error!(error = %e, path, "scan failed");
            let _ = app.emit(
                "scan:error",
                json!({ "path": path, "error": e.to_string() }),
            );
        }
    }
    result
}

#[tauri::command]
pub async fn scan_repo(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    path: String,
    fresh: Option<bool>,
) -> Res<ScanDone> {
    state.count(&state.counters.ipc_calls);
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || do_scan(&app2, &path, fresh.unwrap_or(false)))
        .await
        .map_err(err)?
        .map_err(err)
}

#[derive(Serialize)]
pub struct ViewPayload {
    pub generation: u64,
    pub level: &'static str,
    pub focus: Option<NodeId>,
    pub view: terrarium_core::ViewGraph,
    pub positions: Vec<f32>,
    pub root: String,
    pub stats: terrarium_core::Stats,
}

/// Build the view for a level/focus, seed positions (reusing old ones where ids match), start layout.
pub fn build_view(
    app: &AppHandle,
    level: NodeKind,
    focus: Option<NodeId>,
    backend: Backend,
) -> anyhow::Result<ViewPayload> {
    let state = app.state::<Arc<AppState>>().inner().clone();
    let graph = state
        .graph()
        .ok_or_else(|| anyhow::anyhow!("no repository loaded"))?;
    let _span = tracing::info_span!("build_view", level = level_name(level), focus).entered();
    let view = graph.view(level, focus);
    let mut positions: Vec<[f32; 2]> = Vec::with_capacity(view.nodes.len());
    let generation;
    {
        let mut slot = state.view.write().unwrap();
        let prev = slot.take();
        generation = prev.as_ref().map(|p| p.generation + 1).unwrap_or(1);
        // reuse positions: same id → same spot; new nodes → near parent's old spot or seeded
        let old: std::collections::HashMap<NodeId, [f32; 2]> = prev
            .as_ref()
            .map(|p| {
                p.view
                    .nodes
                    .iter()
                    .zip(&p.positions)
                    .map(|(n, pos)| (n.id, *pos))
                    .collect()
            })
            .unwrap_or_default();
        let group_ids: std::collections::HashMap<NodeId, u32> = {
            let mut m = std::collections::HashMap::new();
            for n in &view.nodes {
                let next = m.len() as u32;
                m.entry(n.group).or_insert(next);
            }
            m
        };
        let groups: Vec<u32> = view
            .nodes
            .iter()
            .map(|n| {
                if n.external {
                    terrarium_layout::NO_GROUP
                } else {
                    group_ids[&n.group]
                }
            })
            .collect();
        let seeded =
            terrarium_layout::seed_positions(view.nodes.len(), &groups, group_ids.len() as u32);
        for (i, n) in view.nodes.iter().enumerate() {
            let p = old.get(&n.id).copied().or_else(|| {
                // parent (or any ancestor) had a position: spawn near it
                let mut cur = graph.node(n.id).parent;
                while let Some(c) = cur {
                    if let Some(p) = old.get(&c) {
                        let s = seeded[i];
                        return Some([
                            p[0] + (s[0] - seeded[0][0]) * 0.15,
                            p[1] + (s[1] - seeded[0][1]) * 0.15,
                        ]);
                    }
                    cur = graph.node(c).parent;
                }
                None
            });
            positions.push(p.unwrap_or(seeded[i]));
        }
        *slot = Some(ViewState {
            level,
            focus,
            view: view.clone(),
            positions: positions.clone(),
            generation,
        });
    }
    let flat: Vec<f32> = positions.iter().flat_map(|p| [p[0], p[1]]).collect();
    let iterations = (600 - (view.nodes.len() as i64 / 20)).clamp(150, 600) as u32;
    layout_runner::start(app, iterations, backend);
    Ok(ViewPayload {
        generation,
        level: level_name(level),
        focus,
        view,
        positions: flat,
        root: graph.root.clone(),
        stats: graph.stats.clone(),
    })
}

#[tauri::command]
pub async fn get_view(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    level: String,
    focus: Option<NodeId>,
    backend: Option<String>,
) -> Res<ViewPayload> {
    state.count(&state.counters.ipc_calls);
    let backend: Backend = backend.as_deref().unwrap_or("auto").parse().map_err(err)?;
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        build_view(&app2, parse_level(&level), focus, backend)
    })
    .await
    .map_err(err)?
    .map_err(err)
}

#[tauri::command]
pub fn run_layout(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    iterations: Option<u32>,
    backend: Option<String>,
) -> Res<Value> {
    state.count(&state.counters.ipc_calls);
    let backend: Backend = backend.as_deref().unwrap_or("auto").parse().map_err(err)?;
    layout_runner::start(&app, iterations.unwrap_or(200), backend);
    serde_json::to_value(&*state.layout.lock().unwrap()).map_err(err)
}

#[tauri::command]
pub fn stop_layout(state: State<'_, Arc<AppState>>) -> Res<()> {
    state.count(&state.counters.ipc_calls);
    layout_runner::CANCEL_SLOT.cancel();
    Ok(())
}

/// Frontend moved a node by hand; keep the backend copy in sync.
#[tauri::command]
pub fn set_position(state: State<'_, Arc<AppState>>, index: usize, x: f32, y: f32) -> Res<()> {
    if let Some(v) = state.view.write().unwrap().as_mut()
        && index < v.positions.len()
    {
        v.positions[index] = [x, y];
    }
    Ok(())
}

#[derive(Serialize)]
pub struct NodeDetail {
    pub node: terrarium_core::Node,
    pub neighbours: Vec<query::Neighbour>,
    pub children: Vec<Value>,
    pub package: Option<String>,
    pub file: Option<String>,
}

pub fn node_detail(graph: &Graph, id: NodeId) -> anyhow::Result<NodeDetail> {
    anyhow::ensure!((id as usize) < graph.nodes.len(), "no node {id}");
    let node = graph.node(id).clone();
    let neighbours = query::neighbours(graph, id);
    let children = graph.children(id).take(200).map(|c| json!({ "id": c.id, "name": c.name, "kind": c.kind, "lang": c.lang, "loc": c.loc, "tags": c.tags })).collect();
    let package = graph
        .ancestor_of_kind(id, NodeKind::Package)
        .map(|p| graph.node(p).name.clone());
    let file = graph
        .ancestor_of_kind(id, NodeKind::File)
        .map(|p| graph.node(p).path.clone());
    Ok(NodeDetail {
        node,
        neighbours,
        children,
        package,
        file,
    })
}

#[tauri::command]
pub fn get_node(state: State<'_, Arc<AppState>>, id: NodeId) -> Res<NodeDetail> {
    state.count(&state.counters.ipc_calls);
    let g = state.graph().ok_or("no repository loaded")?;
    node_detail(&g, id).map_err(err)
}

pub fn resolve_node(graph: &Graph, s: &str) -> anyhow::Result<NodeId> {
    if let Ok(id) = s.parse::<u32>() {
        anyhow::ensure!((id as usize) < graph.nodes.len(), "no node with id {id}");
        return Ok(id);
    }
    if let Some(n) = graph.find_by_path(s) {
        return Ok(n.id);
    }
    let hits: Vec<&terrarium_core::Node> = graph
        .nodes
        .iter()
        .filter(|n| n.path.ends_with(s) || n.name == s)
        .collect();
    match hits.len() {
        1 => Ok(hits[0].id),
        0 => anyhow::bail!("no node matches `{s}`"),
        n => anyhow::bail!("`{s}` is ambiguous ({n} matches)"),
    }
}

#[tauri::command]
pub fn search_nodes(
    state: State<'_, Arc<AppState>>,
    q: String,
    limit: Option<usize>,
) -> Res<Vec<Value>> {
    state.count(&state.counters.ipc_calls);
    let g = state.graph().ok_or("no repository loaded")?;
    Ok(g.search(&q, limit.unwrap_or(30)).into_iter().map(|n| json!({ "id": n.id, "name": n.name, "path": n.path, "kind": n.kind, "lang": n.lang, "tags": n.tags })).collect())
}

#[tauri::command]
pub fn list_flows(state: State<'_, Arc<AppState>>) -> Res<Vec<query::FlowRow>> {
    let g = state.graph().ok_or("no repository loaded")?;
    Ok(query::flows(&g))
}

#[tauri::command]
pub fn list_boundaries(
    state: State<'_, Arc<AppState>>,
    tag: Option<String>,
) -> Res<Vec<query::Boundary>> {
    let g = state.graph().ok_or("no repository loaded")?;
    Ok(query::boundaries(&g, tag.as_deref()))
}

#[tauri::command]
pub fn list_hotspots(
    state: State<'_, Arc<AppState>>,
    level: String,
    limit: Option<usize>,
) -> Res<Vec<query::Hotspot>> {
    let g = state.graph().ok_or("no repository loaded")?;
    Ok(query::hotspots(
        &g,
        parse_level(&level),
        limit.unwrap_or(20),
    ))
}

#[tauri::command]
pub fn recent_repos() -> Vec<cache::CacheEntry> {
    cache::load_index().entries
}

#[tauri::command]
pub fn report_ui(state: State<'_, Arc<AppState>>, report: UiReport) -> Res<()> {
    *state.ui.write().unwrap() = report;
    Ok(())
}

#[tauri::command]
pub fn report_metrics(state: State<'_, Arc<AppState>>, metrics: FrameMetrics) -> Res<()> {
    *state.metrics.write().unwrap() = metrics;
    Ok(())
}

#[derive(Deserialize)]
pub struct UiLog {
    pub level: String,
    pub target: Option<String>,
    pub message: String,
    #[serde(default)]
    pub fields: Value,
}

/// Frontend logs land in the same ring buffer / file as backend logs, under target `ui`.
#[tauri::command]
pub fn log_event(state: State<'_, Arc<AppState>>, entry: UiLog) -> Res<()> {
    let target = entry.target.unwrap_or_else(|| "ui".into());
    let fields = entry.fields.to_string();
    match entry.level.as_str() {
        "error" => {
            state.count(&state.counters.frontend_errors);
            tracing::error!(target: "ui", ui_target = %target, fields = %fields, "{}", entry.message)
        }
        "warn" => {
            tracing::warn!(target: "ui", ui_target = %target, fields = %fields, "{}", entry.message)
        }
        "debug" => {
            tracing::debug!(target: "ui", ui_target = %target, fields = %fields, "{}", entry.message)
        }
        _ => {
            tracing::info!(target: "ui", ui_target = %target, fields = %fields, "{}", entry.message)
        }
    }
    Ok(())
}

#[tauri::command]
pub fn bridge_reply(state: State<'_, Arc<AppState>>, id: u64, result: Value) -> Res<()> {
    if let Some(tx) = state.pending.lock().unwrap().remove(&id) {
        let _ = tx.send(result);
    }
    Ok(())
}

#[tauri::command]
pub fn bridge_info(state: State<'_, Arc<AppState>>) -> Value {
    json!({ "port": state.bridge_port.load(Ordering::Relaxed), "pid": std::process::id(), "version": env!("CARGO_PKG_VERSION") })
}

#[tauri::command]
pub fn open_path(state: State<'_, Arc<AppState>>, path: String, line: Option<u32>) -> Res<()> {
    let g = state.graph().ok_or("no repository loaded")?;
    let abs = Path::new(&g.root).join(path.split('#').next().unwrap_or(&path));
    // Prefer an editor that understands line numbers when present.
    let editor = std::env::var("TERRARIUM_EDITOR")
        .ok()
        .or_else(|| std::env::var("VISUAL").ok())
        .or_else(|| std::env::var("EDITOR").ok());
    let status = match editor.as_deref() {
        Some(ed) if ed.contains("code") || ed.contains("cursor") || ed.contains("zed") => {
            let target = match line {
                Some(l) => format!("{}:{l}", abs.display()),
                None => abs.display().to_string(),
            };
            std::process::Command::new(ed.split_whitespace().next().unwrap_or(ed))
                .arg("--goto")
                .arg(target)
                .spawn()
        }
        _ => std::process::Command::new("open").arg(&abs).spawn(),
    };
    status.map(|_| ()).map_err(err)
}

/// Repository requested on the command line (`TERRARIUM_OPEN=<path>` or first argv), if any.
#[tauri::command]
pub fn initial_repo() -> Option<String> {
    if let Ok(p) = std::env::var("TERRARIUM_OPEN") {
        return Some(p);
    }
    std::env::args()
        .nth(1)
        .filter(|a| !a.starts_with('-') && Path::new(a).is_dir())
}
