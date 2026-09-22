//! Runs the GPU layout on a worker thread and streams positions to the UI.

use crate::state::{AppState, LayoutStatus};
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use terrarium_core::ViewGraph;
use terrarium_layout::{Backend, Input, Params};

#[derive(Serialize, Clone)]
pub struct LayoutTick {
    pub generation: u64,
    pub iteration: u32,
    pub energy: f32,
    pub backend: &'static str,
    pub done: bool,
    /// flat x,y pairs aligned with the view's node order
    pub positions: Vec<f32>,
}

pub fn layout_input(view: &ViewGraph, positions: &[[f32; 2]]) -> Input {
    let index: std::collections::HashMap<u32, u32> = view
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id, i as u32))
        .collect();
    let mut group_ids: std::collections::HashMap<u32, u32> = Default::default();
    let groups: Vec<u32> = view
        .nodes
        .iter()
        .map(|n| {
            if n.external {
                return terrarium_layout::NO_GROUP;
            }
            let next = group_ids.len() as u32;
            *group_ids.entry(n.group).or_insert(next)
        })
        .collect();
    let mass: Vec<f32> = view
        .nodes
        .iter()
        .map(|n| (1.0 + (n.loc as f32).ln_1p() * 0.25 + (n.degree as f32).sqrt() * 0.15).min(6.0))
        .collect();
    let edges: Vec<(u32, u32, f32)> = view
        .edges
        .iter()
        .filter_map(|e| Some((*index.get(&e.from)?, *index.get(&e.to)?, e.weight as f32)))
        .collect();
    Input {
        positions: if positions.len() == view.nodes.len() {
            positions.to_vec()
        } else {
            vec![]
        },
        edges,
        groups,
        mass,
    }
}

/// Cancel any running layout and start a new one for the current view.
pub fn start(app: &AppHandle, max_iterations: u32, backend: Backend) {
    let state = app.state::<Arc<AppState>>().inner().clone();
    // cancel previous
    state.layout_cancel.store(true, Ordering::SeqCst);
    let cancel = Arc::new(AtomicBool::new(false));
    // swap in the new flag: we cannot replace the Arc in the state (it is not a Mutex), so
    // keep a per-run flag and store a clone in the shared slot via a mutex-free trick:
    let (input, generation) = {
        let view = state.view.read().unwrap();
        let Some(v) = view.as_ref() else { return };
        (layout_input(&v.view, &v.positions), v.generation)
    };
    let n = input.n();
    if n == 0 {
        return;
    }
    // Scale iteration budget with size: tiny graphs converge quickly.
    let max_iterations = max_iterations.max(30);
    state.count(&state.counters.layouts);
    *state.layout.lock().unwrap() = LayoutStatus {
        running: true,
        max_iterations,
        ..Default::default()
    };
    let app = app.clone();
    let state2 = state.clone();
    let run_cancel = cancel.clone();
    CANCEL_SLOT.with_slot(|slot| *slot = Some(cancel));
    std::thread::Builder::new()
        .name("layout".into())
        .spawn(move || {
            let started = Instant::now();
            let params = Params {
                // denser graphs get a little more room
                ideal: (50.0 + (n as f32).sqrt() * 1.5).min(140.0),
                ..Params::default()
            };
            let mut engine = match terrarium_layout::engine(&input, params, backend) {
                Ok(e) => e,
                Err(err) => {
                    tracing::error!(error = %err, "layout engine failed");
                    let mut l = state2.layout.lock().unwrap();
                    l.running = false;
                    return;
                }
            };
            let backend_name = engine.backend();
            let adapter = engine.adapter();
            {
                let mut l = state2.layout.lock().unwrap();
                l.backend = backend_name.into();
                l.adapter = adapter;
            }
            let _span =
                tracing::info_span!("layout_run", nodes = n, backend = backend_name).entered();
            let step = if n > 3000 {
                2
            } else if n > 800 {
                4
            } else {
                8
            };
            let mut iteration = 0u32;
            let mut last_emit = Instant::now() - Duration::from_secs(1);
            loop {
                if run_cancel.load(Ordering::SeqCst) {
                    tracing::debug!(iteration, "layout cancelled");
                    break;
                }
                if let Err(e) = engine.step(step) {
                    tracing::error!(error = %e, "layout step failed");
                    break;
                }
                iteration += step;
                let energy = engine.energy().unwrap_or(0.0);
                let done = iteration >= max_iterations || (iteration > 40 && energy < 0.05);
                if last_emit.elapsed() >= Duration::from_millis(33) || done {
                    match engine.positions() {
                        Ok(pos) => {
                            {
                                let mut view = state2.view.write().unwrap();
                                if let Some(v) = view.as_mut()
                                    && v.generation == generation
                                {
                                    v.positions = pos.clone();
                                }
                            }
                            let flat: Vec<f32> = pos.iter().flat_map(|p| [p[0], p[1]]).collect();
                            let _ = app.emit(
                                "layout:tick",
                                LayoutTick {
                                    generation,
                                    iteration,
                                    energy,
                                    backend: backend_name,
                                    done,
                                    positions: flat,
                                },
                            );
                        }
                        Err(e) => tracing::error!(error = %e, "layout readback failed"),
                    }
                    last_emit = Instant::now();
                    {
                        let mut l = state2.layout.lock().unwrap();
                        l.iteration = iteration;
                        l.energy = energy;
                        l.ms = started.elapsed().as_secs_f64() * 1000.0;
                        l.running = !done;
                    }
                }
                if done {
                    tracing::info!(
                        iteration,
                        energy,
                        ms = started.elapsed().as_millis() as u64,
                        backend = backend_name,
                        "layout settled"
                    );
                    break;
                }
            }
            let mut l = state2.layout.lock().unwrap();
            l.running = false;
            l.ms = started.elapsed().as_secs_f64() * 1000.0;
        })
        .expect("spawn layout thread");
}

/// A process-wide slot holding the cancel flag of the current layout run.
pub struct CancelSlot(std::sync::Mutex<Option<Arc<AtomicBool>>>);
impl CancelSlot {
    fn with_slot(&self, f: impl FnOnce(&mut Option<Arc<AtomicBool>>)) {
        let mut g = self.0.lock().unwrap();
        if let Some(prev) = g.as_ref() {
            prev.store(true, Ordering::SeqCst);
        }
        f(&mut g);
    }
    pub fn cancel(&self) {
        let g = self.0.lock().unwrap();
        if let Some(prev) = g.as_ref() {
            prev.store(true, Ordering::SeqCst);
        }
    }
}
pub static CANCEL_SLOT: CancelSlot = CancelSlot(std::sync::Mutex::new(None));
