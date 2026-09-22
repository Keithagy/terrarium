//! Force-directed layout for architecture graphs.
//!
//! Two backends share one algorithm (see `force.wgsl` for the formulas):
//! - [`Backend::Gpu`]: a wgpu compute pipeline (Metal on macOS). Every node is one
//!   thread; the O(n²) repulsion loop is what the GPU is for.
//! - [`Backend::Cpu`]: rayon over nodes. Used when no adapter is available, and in
//!   tests as the reference implementation.

pub mod cpu;
pub mod gpu;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    #[default]
    Auto,
    Gpu,
    Cpu,
}

impl std::str::FromStr for Backend {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "auto" => Ok(Backend::Auto),
            "gpu" => Ok(Backend::Gpu),
            "cpu" => Ok(Backend::Cpu),
            _ => Err(format!("unknown backend `{s}` (expected auto, gpu or cpu)")),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Params {
    pub repulsion: f32,
    pub attraction: f32,
    pub gravity: f32,
    pub cluster_gravity: f32,
    pub damping: f32,
    pub dt: f32,
    /// Ideal edge length in layout units.
    pub ideal: f32,
    /// Repulsion cutoff radius.
    pub cutoff: f32,
    pub max_speed: f32,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            repulsion: 900.0,
            attraction: 0.35,
            gravity: 0.015,
            cluster_gravity: 0.04,
            damping: 0.82,
            dt: 0.9,
            ideal: 60.0,
            cutoff: 1200.0,
            max_speed: 40.0,
        }
    }
}

/// Group id meaning "no cluster": the node is held only by its edges and global gravity.
pub const NO_GROUP: u32 = u32::MAX;

#[derive(Debug, Clone, Default)]
pub struct Input {
    /// Initial positions (empty → seeded on a spiral so the layout is deterministic).
    pub positions: Vec<[f32; 2]>,
    /// Undirected edges; duplicates add weight.
    pub edges: Vec<(u32, u32, f32)>,
    /// Cluster id per node (dense, 0-based). Empty → all in one group.
    /// [`NO_GROUP`] opts a node out of cluster gravity (dependencies float between their importers).
    pub groups: Vec<u32>,
    /// Mass per node (≈ radius). Empty → 1.0.
    pub mass: Vec<f32>,
}

impl Input {
    pub fn n(&self) -> usize {
        self.mass
            .len()
            .max(self.positions.len())
            .max(self.groups.len())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub backend: &'static str,
    pub adapter: String,
    pub nodes: usize,
    pub edges: usize,
    pub iterations: u32,
    pub ms: f64,
    /// Mean speed after the last step; small means converged.
    pub energy: f32,
}

/// Shared prep: seed positions, build CSR adjacency, fill defaults.
pub(crate) struct Prepared {
    pub n: usize,
    pub positions: Vec<[f32; 2]>,
    pub mass: Vec<f32>,
    pub groups: Vec<u32>,
    pub group_count: u32,
    pub offsets: Vec<u32>,
    pub targets: Vec<u32>,
    pub weights: Vec<f32>,
}

impl Prepared {
    pub fn new(input: &Input) -> Self {
        let n = input.n();
        let mass: Vec<f32> = if input.mass.is_empty() {
            vec![1.0; n]
        } else {
            input.mass.iter().map(|m| m.max(0.25)).collect()
        };
        let groups: Vec<u32> = if input.groups.is_empty() {
            vec![0; n]
        } else {
            input.groups.clone()
        };
        let group_count = groups
            .iter()
            .copied()
            .filter(|g| *g != NO_GROUP)
            .max()
            .map(|m| m + 1)
            .unwrap_or(1);
        let positions = if input.positions.len() == n {
            input.positions.clone()
        } else {
            seed_positions(n, &groups, group_count)
        };
        // CSR, undirected
        let mut adj: Vec<Vec<(u32, f32)>> = vec![Vec::new(); n];
        for &(a, b, w) in &input.edges {
            if (a as usize) < n && (b as usize) < n && a != b {
                adj[a as usize].push((b, w));
                adj[b as usize].push((a, w));
            }
        }
        let mut offsets = Vec::with_capacity(n + 1);
        let mut targets = Vec::new();
        let mut weights = Vec::new();
        offsets.push(0u32);
        for list in &adj {
            for &(t, w) in list {
                targets.push(t);
                weights.push(w.max(0.05).sqrt());
            }
            offsets.push(targets.len() as u32);
        }
        Self {
            n,
            positions,
            mass,
            groups,
            group_count,
            offsets,
            targets,
            weights,
        }
    }
}

/// Deterministic seed: each group on its own spoke, nodes on a sunflower spiral within it.
pub fn seed_positions(n: usize, groups: &[u32], group_count: u32) -> Vec<[f32; 2]> {
    let golden = std::f32::consts::PI * (3.0 - 5.0f32.sqrt());
    let mut per_group: Vec<u32> = vec![0; group_count as usize];
    let group_radius = 80.0 + 40.0 * (group_count as f32).sqrt();
    (0..n)
        .map(|i| {
            let g = groups[i];
            if g == NO_GROUP {
                // ungrouped nodes start on a spiral around the origin
                let r = 10.0 * (i as f32 + 0.5).sqrt();
                let a = i as f32 * golden;
                return [r * a.cos(), r * a.sin()];
            }
            let k = per_group[g as usize];
            per_group[g as usize] += 1;
            let ga = g as f32 / group_count as f32 * std::f32::consts::TAU;
            let (gx, gy) = if group_count > 1 {
                (ga.cos() * group_radius, ga.sin() * group_radius)
            } else {
                (0.0, 0.0)
            };
            let r = 14.0 * (k as f32 + 0.5).sqrt();
            let a = k as f32 * golden;
            [gx + r * a.cos(), gy + r * a.sin()]
        })
        .collect()
}

pub fn energy(vel: &[[f32; 2]]) -> f32 {
    if vel.is_empty() {
        return 0.0;
    }
    vel.iter()
        .map(|v| (v[0] * v[0] + v[1] * v[1]).sqrt())
        .sum::<f32>()
        / vel.len() as f32
}

/// A layout that can be stepped incrementally (the app animates it; the CLI runs it to convergence).
pub trait Engine: Send {
    fn step(&mut self, iterations: u32) -> anyhow::Result<()>;
    fn positions(&mut self) -> anyhow::Result<Vec<[f32; 2]>>;
    fn energy(&mut self) -> anyhow::Result<f32>;
    fn backend(&self) -> &'static str;
    fn adapter(&self) -> String;
}

/// Below this many nodes the GPU's device setup costs more than the whole CPU run
/// (measured on an M1 Pro: ~100ms setup vs ~85ms for 300 CPU iterations at 700 nodes).
pub const GPU_MIN_NODES: usize = 800;

/// Pick a backend. `Auto` uses the CPU for small graphs, otherwise tries the GPU
/// first and falls back to the CPU.
pub fn engine(input: &Input, params: Params, backend: Backend) -> anyhow::Result<Box<dyn Engine>> {
    match backend {
        Backend::Cpu => Ok(Box::new(cpu::CpuEngine::new(input, params))),
        Backend::Gpu => Ok(Box::new(gpu::GpuEngine::new(input, params)?)),
        Backend::Auto if input.n() < GPU_MIN_NODES => {
            Ok(Box::new(cpu::CpuEngine::new(input, params)))
        }
        Backend::Auto => match gpu::GpuEngine::new(input, params) {
            Ok(e) => Ok(Box::new(e)),
            Err(err) => {
                tracing::warn!(error = %err, "GPU layout unavailable, falling back to CPU");
                Ok(Box::new(cpu::CpuEngine::new(input, params)))
            }
        },
    }
}

/// Run a layout to completion and report on it.
pub fn layout(
    input: &Input,
    params: Params,
    backend: Backend,
    iterations: u32,
) -> anyhow::Result<(Vec<[f32; 2]>, Report)> {
    let start = std::time::Instant::now();
    let mut eng = engine(input, params, backend)?;
    let _span = tracing::info_span!(
        "layout",
        backend = eng.backend(),
        nodes = input.n(),
        iterations
    )
    .entered();
    eng.step(iterations)?;
    let positions = eng.positions()?;
    let report = Report {
        backend: eng.backend(),
        adapter: eng.adapter(),
        nodes: input.n(),
        edges: input.edges.len(),
        iterations,
        ms: start.elapsed().as_secs_f64() * 1000.0,
        energy: eng.energy()?,
    };
    tracing::info!(ms = report.ms, energy = report.energy, "layout done");
    Ok((positions, report))
}

#[derive(Debug, Clone, Serialize)]
pub struct GpuInfo {
    pub available: bool,
    pub adapter: String,
    pub backend: String,
    pub device_type: String,
}

pub fn gpu_info() -> GpuInfo {
    gpu::probe()
}
