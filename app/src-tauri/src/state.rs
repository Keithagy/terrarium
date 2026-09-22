//! Everything the app knows, in one place, so the bridge can report it.

use crate::telemetry::Telemetry;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;
use terrarium_core::{Graph, NodeId, NodeKind, ViewGraph};
use tokio::sync::oneshot;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Camera {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

/// What the frontend last told us about itself. Refreshed every ~500ms and on change.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UiReport {
    pub camera: Camera,
    pub selection: Option<NodeId>,
    pub hover: Option<NodeId>,
    pub level: String,
    pub focus: Option<NodeId>,
    pub graph_loaded: bool,
    pub search: String,
    pub filters: Value,
    pub panels: Value,
    pub nodes_visible: u32,
    pub edges_visible: u32,
    /// `traces` or `map`: which stage fills the window.
    #[serde(default)]
    pub stage: String,
    /// Entry path of the trace on stage, if any.
    #[serde(default)]
    pub trace: Option<String>,
    pub ts: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FrameMetrics {
    pub fps: f64,
    pub frame_ms_p50: f64,
    pub frame_ms_p95: f64,
    pub draw_calls: u32,
    pub nodes_drawn: u32,
    pub edges_drawn: u32,
    pub labels_drawn: u32,
    pub renderer: String,
    pub ts: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct LayoutStatus {
    pub running: bool,
    pub backend: String,
    pub adapter: String,
    pub iteration: u32,
    pub max_iterations: u32,
    pub energy: f32,
    pub ms: f64,
}

pub struct ViewState {
    pub level: NodeKind,
    pub focus: Option<NodeId>,
    pub view: ViewGraph,
    /// positions aligned with `view.nodes`
    pub positions: Vec<[f32; 2]>,
    pub generation: u64,
}

#[derive(Default)]
pub struct Counters {
    pub ipc_calls: AtomicU64,
    pub bridge_requests: AtomicU64,
    pub scans: AtomicU64,
    pub layouts: AtomicU64,
    pub frontend_errors: AtomicU64,
}

pub struct AppState {
    pub started: Instant,
    pub telemetry: Arc<Telemetry>,
    pub graph: RwLock<Option<Arc<Graph>>>,
    pub view: RwLock<Option<ViewState>>,
    pub ui: RwLock<UiReport>,
    pub metrics: RwLock<FrameMetrics>,
    pub layout: Mutex<LayoutStatus>,
    pub layout_cancel: Arc<AtomicBool>,
    pub scanning: AtomicBool,
    pub counters: Counters,
    pub pending: Mutex<HashMap<u64, oneshot::Sender<Value>>>,
    pub next_request: AtomicU64,
    pub bridge_port: AtomicU64,
    pub bridge_token: String,
}

impl AppState {
    pub fn new(telemetry: Arc<Telemetry>) -> Self {
        Self {
            started: Instant::now(),
            telemetry,
            graph: RwLock::new(None),
            view: RwLock::new(None),
            ui: RwLock::new(UiReport::default()),
            metrics: RwLock::new(FrameMetrics::default()),
            layout: Mutex::new(LayoutStatus::default()),
            layout_cancel: Arc::new(AtomicBool::new(false)),
            scanning: AtomicBool::new(false),
            counters: Counters::default(),
            pending: Mutex::new(HashMap::new()),
            next_request: AtomicU64::new(1),
            bridge_port: AtomicU64::new(0),
            bridge_token: std::env::var("TERRARIUM_TOKEN")
                .unwrap_or_else(|_| uuid::Uuid::new_v4().simple().to_string()),
        }
    }

    pub fn graph(&self) -> Option<Arc<Graph>> {
        self.graph.read().unwrap().clone()
    }

    pub fn uptime_s(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    pub fn count(&self, c: &AtomicU64) {
        c.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn parse_level(s: &str) -> NodeKind {
    match s {
        "package" | "packages" => NodeKind::Package,
        "symbol" | "symbols" => NodeKind::Symbol,
        _ => NodeKind::File,
    }
}

pub fn level_name(k: NodeKind) -> &'static str {
    match k {
        NodeKind::Repo => "repo",
        NodeKind::Package => "package",
        NodeKind::File => "file",
        NodeKind::Symbol => "symbol",
    }
}
