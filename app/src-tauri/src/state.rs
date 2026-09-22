//! Everything the app knows, in one place, so the bridge can report it.

use crate::telemetry::Telemetry;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;
use terrarium_core::Graph;
use terrarium_core::build::Build;
use tokio::sync::oneshot;

/// What the frontend last told us about itself (tab, step, selection, panels…).
/// Refreshed every ~500ms and on change; kept as JSON so the UI can grow fields freely.
pub type UiReport = Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FrameMetrics {
    pub fps: f64,
    pub frame_ms_p50: f64,
    pub frame_ms_p95: f64,
    pub draw_calls: u32,
    #[serde(default)]
    pub pieces_drawn: u32,
    pub renderer: String,
    pub ts: String,
}

#[derive(Default)]
pub struct Counters {
    pub ipc_calls: AtomicU64,
    pub bridge_requests: AtomicU64,
    pub scans: AtomicU64,
    pub designs: AtomicU64,
    pub frontend_errors: AtomicU64,
}

pub struct AppState {
    pub started: Instant,
    pub telemetry: Arc<Telemetry>,
    pub graph: RwLock<Option<Arc<Graph>>>,
    /// The brick model of the current graph, rebuilt after every scan and design.
    pub build: RwLock<Option<Arc<Build>>>,
    pub ui: RwLock<UiReport>,
    pub metrics: RwLock<FrameMetrics>,
    pub scanning: AtomicBool,
    pub designing: AtomicBool,
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
            build: RwLock::new(None),
            ui: RwLock::new(Value::Null),
            metrics: RwLock::new(FrameMetrics::default()),
            scanning: AtomicBool::new(false),
            designing: AtomicBool::new(false),
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

    pub fn build(&self) -> Option<Arc<Build>> {
        self.build.read().unwrap().clone()
    }

    pub fn uptime_s(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    pub fn count(&self, c: &AtomicU64) {
        c.fetch_add(1, Ordering::Relaxed);
    }
}
