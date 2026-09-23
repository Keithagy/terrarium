//! Tauri commands: the frontend's view of the backend. The agent bridge reuses these.

use crate::state::{AppState, FrameMetrics, Loaded, UiReport};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, State};
use terrarium_core::atlas::{self, Atlas};
use terrarium_core::{Graph, NodeId, NodeKind, ScanOptions, cache, discovery, query};

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

/// The atlas as the frontend receives it.
#[derive(Serialize)]
pub struct AtlasView<'a> {
    pub atlas: &'a Atlas,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale: &'a Option<String>,
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
        let (a, stale) = {
            let _s = tracing::info_span!("assemble_atlas").entered();
            atlas::for_graph(&graph, cache::load_atlas(root))
        };
        tracing::info!(source = %a.source, containers = a.containers.len(), relationships = a.relationships.len(), backed = a.report.backed, claimed = a.report.claimed, "atlas assembled");
        *state.graph.write().unwrap() = Some(Arc::new(graph));
        *state.atlas.write().unwrap() = Some(Arc::new(Loaded { atlas: a, stale }));
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

pub fn atlas_value(state: &AppState) -> Res<Value> {
    let l = state.atlas().ok_or_else(|| "no repository loaded".to_string())?;
    serde_json::to_value(AtlasView { atlas: &l.atlas, stale: &l.stale }).map_err(err)
}

#[tauri::command]
pub fn get_atlas(state: State<'_, Arc<AppState>>) -> Res<Value> {
    state.count(&state.counters.ipc_calls);
    atlas_value(&state)
}

#[tauri::command]
pub fn atlas_dsl(state: State<'_, Arc<AppState>>) -> Res<String> {
    let l = state.atlas().ok_or("no repository loaded")?;
    Ok(atlas::to_dsl(&l.atlas))
}

/// Have Claude discover the atlas: a surveyor, one agent per container and per
/// journey, and an editor. Progress arrives as `discover:progress` events, with
/// partial results the UI draws as they land; the checked atlas is saved next to
/// the graph and becomes the current one.
pub fn do_discover(app: &AppHandle, model: Option<String>) -> anyhow::Result<Value> {
    let state = app.state::<Arc<AppState>>().inner().clone();
    let graph = state.graph().ok_or_else(|| anyhow::anyhow!("no repository loaded"))?;
    if state.discovering.swap(true, Ordering::SeqCst) {
        anyhow::bail!("a discovery is already running");
    }
    let _ = app.emit("discover:started", json!({}));
    let result = (|| {
        let _span = tracing::info_span!("discover_with_claude").entered();
        let mut opts = discovery::Options::default();
        if let Some(m) = model {
            opts.model = m;
        }
        let root = Path::new(&graph.root);
        let runner = discovery::claude_runner(root, &opts);
        let progress = |p: discovery::Progress| {
            match &p {
                discovery::Progress::AgentActivity { .. } => tracing::debug!(progress = %serde_json::to_string(&p).unwrap_or_default(), "discovery activity"),
                discovery::Progress::Verified { .. } => tracing::info!(progress = "verified", "discovery progress"),
                _ => tracing::info!(progress = %serde_json::to_string(&p).unwrap_or_default(), "discovery progress"),
            }
            let _ = app.emit("discover:progress", &p);
        };
        let (a, run) = discovery::discover(&graph, &opts, &runner, &progress)?;
        cache::store_atlas(root, &a)?;
        tracing::info!(model = %run.model, cost_usd = run.cost_usd, secs = run.secs, containers = a.containers.len(), backed = a.report.backed, claimed = a.report.claimed, "discovery done");
        *state.atlas.write().unwrap() = Some(Arc::new(Loaded { atlas: a, stale: None }));
        state.count(&state.counters.discoveries);
        Ok::<_, anyhow::Error>(serde_json::to_value(&run)?)
    })();
    state.discovering.store(false, Ordering::SeqCst);
    match &result {
        Ok(run) => {
            let _ = app.emit("discover:done", run);
        }
        Err(e) => {
            tracing::warn!(error = %e, "discovery failed");
            let _ = app.emit("discover:error", json!({ "error": e.to_string() }));
        }
    }
    result
}

#[tauri::command]
pub async fn discover_with_claude(app: AppHandle, state: State<'_, Arc<AppState>>, model: Option<String>) -> Res<Value> {
    state.count(&state.counters.ipc_calls);
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || do_discover(&app2, model)).await.map_err(err)?.map_err(err)
}

/// Forget Claude's atlas and go back to the engine's.
pub fn do_reset_atlas(state: &AppState) -> anyhow::Result<()> {
    let graph = state.graph().ok_or_else(|| anyhow::anyhow!("no repository loaded"))?;
    cache::clear_atlas(Path::new(&graph.root))?;
    let a = atlas::engine_atlas(&graph);
    *state.atlas.write().unwrap() = Some(Arc::new(Loaded { atlas: a, stale: None }));
    Ok(())
}

#[tauri::command]
pub fn reset_atlas(state: State<'_, Arc<AppState>>) -> Res<Value> {
    do_reset_atlas(&state).map_err(err)?;
    atlas_value(&state)
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
    let children = graph.children(id).take(200).map(|c| json!({ "id": c.id, "name": c.name, "kind": c.kind, "lang": c.lang, "loc": c.loc, "tags": c.tags, "symbol_kind": c.symbol_kind, "span": c.span })).collect();
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

/// Node detail by path, for the code level of the atlas.
#[tauri::command]
pub fn get_file(state: State<'_, Arc<AppState>>, path: String) -> Res<NodeDetail> {
    state.count(&state.counters.ipc_calls);
    let g = state.graph().ok_or("no repository loaded")?;
    let id = resolve_node(&g, &path).map_err(err)?;
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
pub fn get_trace(state: State<'_, Arc<AppState>>, entry: String) -> Res<query::Trace> {
    let g = state.graph().ok_or("no repository loaded")?;
    let id = resolve_node(&g, &entry).map_err(err)?;
    query::trace_from(&g, id).ok_or_else(|| format!("{} crosses no boundary", g.node(id).path))
}

#[tauri::command]
pub fn list_endpoints(state: State<'_, Arc<AppState>>) -> Res<Vec<query::Endpoint>> {
    let g = state.graph().ok_or("no repository loaded")?;
    Ok(query::endpoints(&g))
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
