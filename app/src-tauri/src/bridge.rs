//! The agent bridge: a localhost HTTP API for verifying, debugging, profiling and
//! driving the app from outside (the `terrarium app ...` CLI is a thin client).
//!
//! Auth: `x-terrarium-token` header (or `?token=`) matching the token written to
//! `<terrarium home>/bridge.json` at startup. Set `TERRARIUM_BRIDGE_NO_AUTH=1` to
//! disable for local experiments. Binds 127.0.0.1 only.

use crate::commands;
use crate::state::{AppState, level_name};
use axum::body::Bytes;
use axum::extract::{Path as AxPath, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router, middleware};
use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;

#[derive(Clone)]
pub struct Ctx {
    pub app: AppHandle,
    pub state: Arc<AppState>,
}

pub const DEFAULT_PORT: u16 = 47311;

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError(StatusCode::BAD_REQUEST, e.to_string())
    }
}

impl From<String> for ApiError {
    fn from(e: String) -> Self {
        ApiError(StatusCode::BAD_REQUEST, e)
    }
}

type ApiResult = Result<Json<Value>, ApiError>;

/// Ask the frontend to do something and wait for its answer.
pub async fn ask_frontend(
    ctx: &Ctx,
    op: &str,
    payload: Value,
    timeout_ms: u64,
) -> anyhow::Result<Value> {
    let id = ctx.state.next_request.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = oneshot::channel();
    ctx.state.pending.lock().unwrap().insert(id, tx);
    ctx.state.count(&ctx.state.counters.bridge_requests);
    ctx.app.emit(
        "bridge:request",
        json!({ "id": id, "op": op, "payload": payload }),
    )?;
    match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), rx).await {
        Ok(Ok(v)) => {
            if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
                anyhow::bail!("{e}");
            }
            Ok(v)
        }
        Ok(Err(_)) => anyhow::bail!("frontend dropped request {op}"),
        Err(_) => {
            ctx.state.pending.lock().unwrap().remove(&id);
            anyhow::bail!(
                "frontend did not answer `{op}` within {timeout_ms}ms (is the window open?)"
            )
        }
    }
}

fn describe() -> Value {
    json!({
        "name": "terrarium",
        "description": "Agent bridge for the Terrarium app. All responses are JSON. Mutations return the resulting state.",
        "version": env!("CARGO_PKG_VERSION"),
        "auth": "header x-terrarium-token (see <terrarium home>/bridge.json)",
        "endpoints": [
            { "method": "GET",  "path": "/health",         "summary": "liveness, repo, counts, fps" },
            { "method": "GET",  "path": "/state",          "summary": "full UI + backend state (selection, camera, level, layout, filters, panels)" },
            { "method": "POST", "path": "/scan",           "summary": "{path, fresh?} scan or load from cache, then show it" },
            { "method": "GET",  "path": "/graph",          "summary": "?level=&focus= current view graph with positions" },
            { "method": "GET",  "path": "/graph/full",     "summary": "the raw graph (all nodes and edges)" },
            { "method": "GET",  "path": "/node/{id}",      "summary": "node detail with neighbours" },
            { "method": "GET",  "path": "/search",         "summary": "?q=&limit= fuzzy node search" },
            { "method": "GET",  "path": "/flows",          "summary": "cross-language data flows" },
            { "method": "POST", "path": "/select",         "summary": "{node} select by id or path" },
            { "method": "POST", "path": "/focus",          "summary": "{node} center camera on node and expand it" },
            { "method": "POST", "path": "/level",          "summary": "{level: package|file|symbol}" },
            { "method": "POST", "path": "/search",         "summary": "{q} type into the search box" },
            { "method": "POST", "path": "/filter",         "summary": "{langs?, edges?, tag?} visibility filters" },
            { "method": "POST", "path": "/camera",         "summary": "{x?, y?, zoom?, fit?}" },
            { "method": "POST", "path": "/layout",         "summary": "{iterations?, backend?} run layout" },
            { "method": "POST", "path": "/reset",          "summary": "clear selection, filters, camera" },
            { "method": "GET",  "path": "/screenshot",     "summary": "PNG of the window (?format=json for a data URL)" },
            { "method": "GET",  "path": "/ui",             "summary": "semantic snapshot: panels, testids, texts, toasts" },
            { "method": "POST", "path": "/ui/click",       "summary": "{testid} click an element" },
            { "method": "POST", "path": "/ui/type",        "summary": "{testid, text} type into an element" },
            { "method": "POST", "path": "/eval",           "summary": "{js} evaluate in the webview, JSON result" },
            { "method": "GET",  "path": "/logs",           "summary": "?limit=&level=&since=&grep= recent backend+frontend events" },
            { "method": "GET",  "path": "/metrics",        "summary": "fps, frame times, memory, counters" },
            { "method": "GET",  "path": "/profile",        "summary": "span timings aggregated since launch" },
            { "method": "POST", "path": "/quit",           "summary": "exit the app" }
        ]
    })
}

async fn auth(
    State(ctx): State<Ctx>,
    req: axum::extract::Request,
    next: middleware::Next,
) -> Response {
    if std::env::var("TERRARIUM_BRIDGE_NO_AUTH")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        return next.run(req).await;
    }
    let header_ok = req
        .headers()
        .get("x-terrarium-token")
        .and_then(|v| v.to_str().ok())
        .map(|t| t == ctx.state.bridge_token)
        .unwrap_or(false);
    let query_ok = req
        .uri()
        .query()
        .map(|q| {
            q.split('&')
                .any(|kv| kv == format!("token={}", ctx.state.bridge_token))
        })
        .unwrap_or(false);
    let path = req.uri().path();
    if header_ok || query_ok || path == "/" || path == "/health" {
        next.run(req).await
    } else {
        ApiError(
            StatusCode::UNAUTHORIZED,
            "missing or wrong x-terrarium-token (read <terrarium home>/bridge.json)".into(),
        )
        .into_response()
    }
}

pub fn router(ctx: Ctx) -> Router {
    Router::new()
        .route("/", get(|| async { Json(describe()) }))
        .route("/health", get(health))
        .route("/state", get(state))
        .route("/scan", post(scan))
        .route("/open", post(scan))
        .route("/graph", get(graph))
        .route("/graph/full", get(graph_full))
        .route("/node/{id}", get(node))
        .route("/search", get(search_get).post(search_post))
        .route("/flows", get(flows))
        .route("/select", post(select))
        .route("/focus", post(focus))
        .route("/level", post(level))
        .route("/filter", post(filter))
        .route("/camera", post(camera))
        .route("/layout", post(layout))
        .route("/reset", post(reset))
        .route("/screenshot", get(screenshot))
        .route("/ui", get(ui))
        .route("/ui/click", post(ui_click))
        .route("/ui/type", post(ui_type))
        .route("/eval", post(eval))
        .route("/logs", get(logs))
        .route("/metrics", get(metrics))
        .route("/profile", get(profile))
        .route("/quit", post(quit))
        .layer(middleware::from_fn_with_state(ctx.clone(), auth))
        .with_state(ctx)
}

fn backend_state(ctx: &Ctx) -> Value {
    let s = &ctx.state;
    let graph = s.graph();
    let view = s.view.read().unwrap();
    json!({
        "repo": graph.as_ref().map(|g| g.root.clone()),
        "scanning": s.scanning.load(Ordering::Relaxed),
        "stats": graph.as_ref().map(|g| g.stats.clone()),
        "view": view.as_ref().map(|v| json!({ "level": level_name(v.level), "focus": v.focus, "nodes": v.view.nodes.len(), "edges": v.view.edges.len(), "generation": v.generation })),
        "layout": *s.layout.lock().unwrap(),
        "ui_last_report": *s.ui.read().unwrap(),
        "uptime_s": s.uptime_s(),
    })
}

async fn health(State(ctx): State<Ctx>) -> Json<Value> {
    let s = &ctx.state;
    let graph = s.graph();
    Json(json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "pid": std::process::id(),
        "uptime_s": s.uptime_s(),
        "repo": graph.as_ref().map(|g| g.root.clone()),
        "nodes": graph.as_ref().map(|g| g.nodes.len()),
        "edges": graph.as_ref().map(|g| g.edges.len()),
        "scanning": s.scanning.load(Ordering::Relaxed),
        "layout_running": s.layout.lock().unwrap().running,
        "fps": s.metrics.read().unwrap().fps,
        "errors": s.telemetry.errors.load(Ordering::Relaxed),
    }))
}

async fn state(State(ctx): State<Ctx>) -> ApiResult {
    let mut v = backend_state(&ctx);
    match ask_frontend(&ctx, "state", json!({}), 2000).await {
        Ok(ui) => v["ui"] = ui,
        Err(e) => v["ui_error"] = json!(e.to_string()),
    }
    Ok(Json(v))
}

#[derive(Deserialize)]
struct ScanReq {
    path: String,
    #[serde(default)]
    fresh: bool,
}

async fn scan(State(ctx): State<Ctx>, Json(req): Json<ScanReq>) -> ApiResult {
    let app = ctx.app.clone();
    let path = req.path.clone();
    let fresh = req.fresh;
    let done = tauri::async_runtime::spawn_blocking(move || commands::do_scan(&app, &path, fresh))
        .await
        .map_err(|e| e.to_string())??;
    // The frontend loads the view on `scan:done`; wait for it so callers see a ready UI.
    let mut ui = Value::Null;
    for _ in 0..100 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if let Ok(s) = ask_frontend(&ctx, "state", json!({}), 1000).await
            && s.get("graph_loaded")
                .and_then(|b| b.as_bool())
                .unwrap_or(false)
            && s.get("repo").and_then(|r| r.as_str()) == Some(done.root.as_str())
        {
            ui = s;
            break;
        }
    }
    let mut v = serde_json::to_value(&done).map_err(|e| e.to_string())?;
    v["ui"] = ui;
    Ok(Json(v))
}

#[derive(Deserialize)]
struct GraphQ {
    level: Option<String>,
    focus: Option<u32>,
}

async fn graph(State(ctx): State<Ctx>, Query(q): Query<GraphQ>) -> ApiResult {
    if q.level.is_some() || q.focus.is_some() {
        let level = q.level.clone().unwrap_or_else(|| "file".into());
        ask_frontend(
            &ctx,
            "level",
            json!({ "level": level, "focus": q.focus }),
            15000,
        )
        .await?;
    }
    let view = ctx.state.view.read().unwrap();
    let Some(v) = view.as_ref() else {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "no view; scan a repository first".into(),
        ));
    };
    Ok(Json(
        json!({ "level": level_name(v.level), "focus": v.focus, "generation": v.generation, "view": v.view, "positions": v.positions }),
    ))
}

async fn graph_full(State(ctx): State<Ctx>) -> ApiResult {
    let g = ctx
        .state
        .graph()
        .ok_or_else(|| "no repository loaded".to_string())?;
    Ok(Json(serde_json::to_value(&*g).map_err(|e| e.to_string())?))
}

async fn node(State(ctx): State<Ctx>, AxPath(id): AxPath<String>) -> ApiResult {
    let g = ctx
        .state
        .graph()
        .ok_or_else(|| "no repository loaded".to_string())?;
    let id = commands::resolve_node(&g, &id)?;
    Ok(Json(
        serde_json::to_value(commands::node_detail(&g, id)?).map_err(|e| e.to_string())?,
    ))
}

#[derive(Deserialize)]
struct SearchQ {
    q: String,
    limit: Option<usize>,
}

async fn search_get(State(ctx): State<Ctx>, Query(q): Query<SearchQ>) -> ApiResult {
    let g = ctx
        .state
        .graph()
        .ok_or_else(|| "no repository loaded".to_string())?;
    let hits: Vec<Value> = g
        .search(&q.q, q.limit.unwrap_or(30))
        .into_iter()
        .map(|n| json!({ "id": n.id, "path": n.path, "kind": n.kind, "lang": n.lang }))
        .collect();
    Ok(Json(json!({ "results": hits })))
}

async fn search_post(State(ctx): State<Ctx>, Json(body): Json<Value>) -> ApiResult {
    Ok(Json(ask_frontend(&ctx, "search", body, 5000).await?))
}

async fn flows(State(ctx): State<Ctx>) -> ApiResult {
    let g = ctx
        .state
        .graph()
        .ok_or_else(|| "no repository loaded".to_string())?;
    Ok(Json(json!({ "flows": terrarium_core::query::flows(&g) })))
}

async fn resolve_body_node(ctx: &Ctx, body: &Value) -> Result<u32, ApiError> {
    let g = ctx
        .state
        .graph()
        .ok_or_else(|| "no repository loaded".to_string())?;
    let s = body
        .get("node")
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .ok_or_else(|| "missing `node`".to_string())?;
    Ok(commands::resolve_node(&g, &s)?)
}

async fn select(State(ctx): State<Ctx>, Json(body): Json<Value>) -> ApiResult {
    let id = resolve_body_node(&ctx, &body).await?;
    Ok(Json(
        ask_frontend(&ctx, "select", json!({ "id": id }), 10000).await?,
    ))
}

async fn focus(State(ctx): State<Ctx>, Json(body): Json<Value>) -> ApiResult {
    let id = resolve_body_node(&ctx, &body).await?;
    Ok(Json(
        ask_frontend(&ctx, "focus", json!({ "id": id }), 20000).await?,
    ))
}

async fn level(State(ctx): State<Ctx>, Json(body): Json<Value>) -> ApiResult {
    Ok(Json(ask_frontend(&ctx, "level", body, 20000).await?))
}

async fn filter(State(ctx): State<Ctx>, Json(body): Json<Value>) -> ApiResult {
    Ok(Json(ask_frontend(&ctx, "filter", body, 5000).await?))
}

async fn camera(State(ctx): State<Ctx>, Json(body): Json<Value>) -> ApiResult {
    Ok(Json(ask_frontend(&ctx, "camera", body, 5000).await?))
}

#[derive(Deserialize)]
struct LayoutReq {
    iterations: Option<u32>,
    backend: Option<String>,
}

async fn layout(State(ctx): State<Ctx>, Json(req): Json<LayoutReq>) -> ApiResult {
    let backend: terrarium_layout::Backend = req.backend.as_deref().unwrap_or("auto").parse()?;
    crate::layout_runner::start(&ctx.app, req.iterations.unwrap_or(200), backend);
    Ok(Json(
        serde_json::to_value(&*ctx.state.layout.lock().unwrap()).map_err(|e| e.to_string())?,
    ))
}

async fn reset(State(ctx): State<Ctx>) -> ApiResult {
    Ok(Json(ask_frontend(&ctx, "reset", json!({}), 5000).await?))
}

#[derive(Deserialize)]
struct ShotQ {
    format: Option<String>,
}

async fn screenshot(State(ctx): State<Ctx>, Query(q): Query<ShotQ>) -> Result<Response, ApiError> {
    let t0 = std::time::Instant::now();
    let (png, partial) = match crate::snapshot::capture_png(&ctx.app).await {
        Ok(png) => (png, false),
        Err(e) => {
            tracing::warn!(error = %e, "native snapshot failed; falling back to canvas capture");
            let v = ask_frontend(&ctx, "screenshot", json!({}), 10000).await?;
            let url = v
                .get("dataUrl")
                .and_then(|s| s.as_str())
                .ok_or_else(|| "no dataUrl from frontend".to_string())?;
            let b64 = url
                .split(',')
                .nth(1)
                .ok_or_else(|| "bad data url".to_string())?;
            (
                base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .map_err(|e| e.to_string())?,
                true,
            )
        }
    };
    tracing::info!(
        bytes = png.len(),
        partial,
        ms = t0.elapsed().as_millis() as u64,
        "screenshot"
    );
    if q.format.as_deref() == Some("json") {
        let data = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&png)
        );
        return Ok(
            Json(json!({ "dataUrl": data, "bytes": png.len(), "partial": partial }))
                .into_response(),
        );
    }
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "image/png".parse().unwrap());
    headers.insert("x-terrarium-partial", partial.to_string().parse().unwrap());
    Ok((headers, Bytes::from(png)).into_response())
}

async fn ui(State(ctx): State<Ctx>) -> ApiResult {
    Ok(Json(ask_frontend(&ctx, "ui", json!({}), 5000).await?))
}

async fn ui_click(State(ctx): State<Ctx>, Json(body): Json<Value>) -> ApiResult {
    Ok(Json(ask_frontend(&ctx, "click", body, 5000).await?))
}

async fn ui_type(State(ctx): State<Ctx>, Json(body): Json<Value>) -> ApiResult {
    Ok(Json(ask_frontend(&ctx, "type", body, 5000).await?))
}

async fn eval(State(ctx): State<Ctx>, Json(body): Json<Value>) -> ApiResult {
    Ok(Json(ask_frontend(&ctx, "eval", body, 15000).await?))
}

#[derive(Deserialize)]
struct LogsQ {
    limit: Option<usize>,
    level: Option<String>,
    since: Option<u64>,
    grep: Option<String>,
}

async fn logs(State(ctx): State<Ctx>, Query(q): Query<LogsQ>) -> ApiResult {
    let level = q
        .level
        .as_deref()
        .map(|l| match l.to_ascii_lowercase().as_str() {
            "error" => tracing::Level::ERROR,
            "warn" => tracing::Level::WARN,
            "info" => tracing::Level::INFO,
            "debug" => tracing::Level::DEBUG,
            _ => tracing::Level::TRACE,
        });
    let (events, latest) = ctx.state.telemetry.events(
        q.since.unwrap_or(0),
        level,
        q.grep.as_deref(),
        q.limit.unwrap_or(100),
    );
    Ok(Json(
        json!({ "count": events.len(), "latest_seq": latest, "events": events, "file": terrarium_core::cache::logs_dir().join("app.jsonl").to_string_lossy() }),
    ))
}

async fn metrics(State(ctx): State<Ctx>) -> ApiResult {
    let s = &ctx.state;
    let mem = memory_stats::memory_stats();
    let frame = s.metrics.read().unwrap().clone();
    Ok(Json(json!({
        "frame": frame,
        "memory_rss_mb": mem.map(|m| m.physical_mem as f64 / 1_048_576.0),
        "memory_virtual_mb": mem.map(|m| m.virtual_mem as f64 / 1_048_576.0),
        "uptime_s": s.uptime_s(),
        "counters": {
            "ipc_calls": s.counters.ipc_calls.load(Ordering::Relaxed),
            "bridge_requests": s.counters.bridge_requests.load(Ordering::Relaxed),
            "scans": s.counters.scans.load(Ordering::Relaxed),
            "layouts": s.counters.layouts.load(Ordering::Relaxed),
            "frontend_errors": s.counters.frontend_errors.load(Ordering::Relaxed),
            "log_errors": s.telemetry.errors.load(Ordering::Relaxed),
            "log_warnings": s.telemetry.warnings.load(Ordering::Relaxed),
        },
        "layout": *s.layout.lock().unwrap(),
        "scan_ms": s.graph().map(|g| g.stats.scan_ms),
    })))
}

async fn profile(State(ctx): State<Ctx>) -> ApiResult {
    Ok(Json(json!({ "spans": ctx.state.telemetry.spans() })))
}

async fn quit(State(ctx): State<Ctx>) -> ApiResult {
    let app = ctx.app.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        app.exit(0);
    });
    Ok(Json(json!({ "app": "quitting" })))
}

/// Start the bridge; returns the bound port. Writes bridge.json for CLI discovery.
pub async fn serve(ctx: Ctx) -> anyhow::Result<u16> {
    let want: u16 = std::env::var("TERRARIUM_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT);
    let mut listener = None;
    for port in [want, 0] {
        match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(l) => {
                listener = Some(l);
                break;
            }
            Err(e) => tracing::warn!(port, error = %e, "bridge port busy"),
        }
    }
    let listener = listener.ok_or_else(|| anyhow::anyhow!("cannot bind bridge"))?;
    let port = listener.local_addr()?.port();
    ctx.state.bridge_port.store(port as u64, Ordering::Relaxed);
    let home = terrarium_core::cache::home_dir();
    let _ = std::fs::create_dir_all(&home);
    let info = json!({ "port": port, "token": ctx.state.bridge_token, "pid": std::process::id(), "started_at": crate::telemetry::now_rfc3339() });
    std::fs::write(home.join("bridge.json"), serde_json::to_vec_pretty(&info)?)?;
    tracing::info!(port, file = %home.join("bridge.json").display(), "agent bridge listening");
    let app = router(ctx);
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "bridge server stopped");
        }
    });
    let _ = HashMap::<(), ()>::new();
    Ok(port)
}
