//! Tracing layers that keep the app observable from the outside:
//! a ring buffer of recent events (served at `/logs`), per-span timing
//! aggregates (served at `/profile`), and a JSON-lines file on disk.

use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

pub const RING_CAPACITY: usize = 5000;

#[derive(Debug, Clone, Serialize)]
pub struct LogEvent {
    pub seq: u64,
    pub ts: String,
    pub level: String,
    pub target: String,
    pub message: String,
    #[serde(skip_serializing_if = "Map::is_empty")]
    pub fields: Map<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SpanStat {
    pub name: String,
    pub count: u64,
    pub total_ms: f64,
    pub mean_ms: f64,
    pub max_ms: f64,
    pub last_ms: f64,
}

#[derive(Default)]
pub struct Telemetry {
    seq: AtomicU64,
    ring: Mutex<VecDeque<LogEvent>>,
    spans: Mutex<HashMap<String, SpanStat>>,
    pub errors: AtomicU64,
    pub warnings: AtomicU64,
}

impl Telemetry {
    pub fn push(&self, mut ev: LogEvent) {
        ev.seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        match ev.level.as_str() {
            "ERROR" => {
                self.errors.fetch_add(1, Ordering::Relaxed);
            }
            "WARN" => {
                self.warnings.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
        let mut ring = self.ring.lock().unwrap();
        if ring.len() >= RING_CAPACITY {
            ring.pop_front();
        }
        ring.push_back(ev);
    }

    pub fn events(
        &self,
        since: u64,
        level: Option<Level>,
        grep: Option<&str>,
        limit: usize,
    ) -> (Vec<LogEvent>, u64) {
        let ring = self.ring.lock().unwrap();
        let latest = ring.back().map(|e| e.seq).unwrap_or(0);
        let min = level.unwrap_or(Level::TRACE);
        let out: Vec<LogEvent> = ring
            .iter()
            .rev()
            .filter(|e| e.seq > since)
            .filter(|e| level_of(&e.level) <= min)
            .filter(|e| {
                grep.map(|g| {
                    e.message.contains(g)
                        || e.target.contains(g)
                        || e.fields.values().any(|v| v.to_string().contains(g))
                })
                .unwrap_or(true)
            })
            .take(limit)
            .cloned()
            .collect();
        (out.into_iter().rev().collect(), latest)
    }

    pub fn record_span(&self, name: &str, ms: f64) {
        let mut spans = self.spans.lock().unwrap();
        let s = spans.entry(name.to_string()).or_insert_with(|| SpanStat {
            name: name.to_string(),
            ..Default::default()
        });
        s.count += 1;
        s.total_ms += ms;
        s.mean_ms = s.total_ms / s.count as f64;
        s.max_ms = s.max_ms.max(ms);
        s.last_ms = ms;
    }

    pub fn spans(&self) -> Vec<SpanStat> {
        let mut v: Vec<SpanStat> = self.spans.lock().unwrap().values().cloned().collect();
        v.sort_by(|a, b| b.total_ms.partial_cmp(&a.total_ms).unwrap());
        v
    }
}

fn level_of(s: &str) -> Level {
    match s {
        "ERROR" => Level::ERROR,
        "WARN" => Level::WARN,
        "INFO" => Level::INFO,
        "DEBUG" => Level::DEBUG,
        _ => Level::TRACE,
    }
}

pub fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

struct FieldVisitor {
    message: String,
    fields: Map<String, Value>,
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            self.fields
                .insert(field.name().into(), Value::String(format!("{value:?}")));
        }
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields
                .insert(field.name().into(), Value::String(value.into()));
        }
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields.insert(field.name().into(), value.into());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields.insert(field.name().into(), value.into());
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.fields.insert(field.name().into(), value.into());
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields.insert(field.name().into(), value.into());
    }
}

struct SpanStart(Instant);

pub struct TelemetryLayer(pub std::sync::Arc<Telemetry>);

impl<S> Layer<S> for TelemetryLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, _attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(SpanStart(Instant::now()));
        }
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(&id) {
            let started = span.extensions().get::<SpanStart>().map(|s| s.0);
            if let Some(s) = started {
                self.0
                    .record_span(span.name(), s.elapsed().as_secs_f64() * 1000.0);
            }
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut v = FieldVisitor {
            message: String::new(),
            fields: Map::new(),
        };
        event.record(&mut v);
        let span = ctx.event_span(event).map(|s| s.name().to_string());
        self.0.push(LogEvent {
            seq: 0,
            ts: now_rfc3339(),
            level: event.metadata().level().to_string(),
            target: event.metadata().target().to_string(),
            message: v.message,
            fields: v.fields,
            span,
        });
    }
}

/// Install the global subscriber: stderr (pretty), JSONL file, and the in-memory telemetry.
pub fn init() -> std::sync::Arc<Telemetry> {
    use tracing_subscriber::{EnvFilter, fmt, prelude::*};
    let telemetry = std::sync::Arc::new(Telemetry::default());
    let filter = EnvFilter::try_from_env("TERRARIUM_LOG").unwrap_or_else(|_| EnvFilter::new("info,terrarium_core=info,terrarium_layout=info,ui=info,wgpu_core=warn,wgpu_hal=warn,naga=warn"));
    let logs_dir = terrarium_core::cache::logs_dir();
    let _ = std::fs::create_dir_all(&logs_dir);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(logs_dir.join("app.jsonl"))
        .ok();
    let file_layer = file.map(|f| {
        fmt::layer()
            .json()
            .with_writer(std::sync::Mutex::new(f))
            .with_span_events(fmt::format::FmtSpan::CLOSE)
    });
    tracing_subscriber::registry()
        .with(filter)
        .with(
            fmt::layer()
                .with_writer(std::io::stderr)
                .with_target(true)
                .compact(),
        )
        .with(file_layer)
        .with(TelemetryLayer(telemetry.clone()))
        .init();
    telemetry
}
