//! Everything the app knows, in one place, so the bridge can report it.

use crate::telemetry::Telemetry;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;
use terrarium_core::Graph;
use terrarium_core::atlas::Atlas;
use tokio::sync::oneshot;

/// What the frontend last told us about itself (level, focus, selection, panels…).
/// Refreshed every ~500ms and on change; kept as JSON so the UI can grow fields freely.
pub type UiReport = Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FrameMetrics {
    pub fps: f64,
    pub frame_ms_p50: f64,
    pub frame_ms_p95: f64,
    #[serde(default)]
    pub elements_drawn: u32,
    pub renderer: String,
    pub ts: String,
}

#[derive(Default)]
pub struct Counters {
    pub ipc_calls: AtomicU64,
    pub bridge_requests: AtomicU64,
    pub scans: AtomicU64,
    pub discoveries: AtomicU64,
    pub frontend_errors: AtomicU64,
}

/// The atlas on show, and why it may not match the scan exactly.
pub struct Loaded {
    pub atlas: Atlas,
    /// Set when a saved agent atlas was made for an older scan and was re-checked against this one.
    pub stale: Option<String>,
}

pub struct AppState {
    pub started: Instant,
    pub telemetry: Arc<Telemetry>,
    pub graph: RwLock<Option<Arc<Graph>>>,
    /// The atlas of the current graph, rebuilt after every scan and discovery.
    pub atlas: RwLock<Option<Arc<Loaded>>>,
    pub ui: RwLock<UiReport>,
    pub metrics: RwLock<FrameMetrics>,
    pub scanning: AtomicBool,
    pub discovering: AtomicBool,
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
            atlas: RwLock::new(None),
            ui: RwLock::new(Value::Null),
            metrics: RwLock::new(FrameMetrics::default()),
            scanning: AtomicBool::new(false),
            discovering: AtomicBool::new(false),
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

    pub fn atlas(&self) -> Option<Arc<Loaded>> {
        self.atlas.read().unwrap().clone()
    }

    pub fn uptime_s(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    pub fn count(&self, c: &AtomicU64) {
        c.fetch_add(1, Ordering::Relaxed);
    }
}
