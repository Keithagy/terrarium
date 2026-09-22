//! `terrarium` — agent-friendly CLI.
//!
//! Output is TOON on stdout (or JSON with `--json`). Errors are structured and go
//! to stdout too. Exit codes: 0 success (including no-ops), 1 error, 2 usage.
//! Diagnostics (tracing) go to stderr and are off unless `TERRARIUM_LOG` is set.

mod bridge;
mod setup;
mod toon;

use anyhow::{Context, Result, anyhow};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use terrarium_core::{Graph, NodeKind, ScanOptions, cache, query};
use terrarium_layout::{Backend, Input, Params};

const DESCRIPTION: &str = "Scan multi-language repositories into an architecture + dataflow graph, query it, and drive the Terrarium app";

#[derive(Parser)]
#[command(name = "terrarium", version, about = DESCRIPTION, disable_help_subcommand = true)]
struct Cli {
    /// Emit JSON instead of TOON.
    #[arg(long, global = true)]
    json: bool,
    /// Repository path (defaults to the current directory; the nearest cached ancestor is used for queries).
    #[arg(long, global = true, value_name = "PATH")]
    repo: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Scan a repository and cache its graph.
    Scan {
        /// Repository path (default: current directory).
        path: Option<PathBuf>,
        /// Also write the graph JSON to this file.
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
        /// Do not create nodes for external dependencies.
        #[arg(long)]
        no_external: bool,
    },
    /// Summary statistics of the cached graph.
    Stats,
    /// List nodes (packages, files or symbols).
    Nodes {
        #[arg(long, value_enum, default_value = "file")]
        kind: KindArg,
        /// Filter by language (rust, typescript, javascript, python, go).
        #[arg(long)]
        lang: Option<String>,
        /// Filter by tag prefix (db, http-server, env:, route:/api ...).
        #[arg(long)]
        tag: Option<String>,
        /// Filter by path substring.
        #[arg(long)]
        path: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
        /// Extra fields to include: lang,loc,tags,parent,span,degree.
        #[arg(long, value_delimiter = ',')]
        fields: Vec<String>,
    },
    /// Fuzzy search nodes by name or path.
    Search {
        query: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Show one node with its neighbours. Accepts an id or a path (`src/a.rs`, `src/a.rs#main`).
    Show {
        node: String,
        /// Show all neighbours instead of the first 40.
        #[arg(long)]
        full: bool,
    },
    /// Cross-language / cross-boundary data flows (HTTP routes, IPC commands, queues).
    Flows {
        #[arg(long, default_value_t = 200)]
        limit: usize,
    },
    /// Nodes that touch the outside world (http, db, fs, env, ipc, queue, process).
    Boundaries {
        /// Tag prefix filter, e.g. `db`, `http`, `env:DATABASE_URL`.
        #[arg(long)]
        tag: Option<String>,
        #[arg(long, default_value_t = 200)]
        limit: usize,
    },
    /// Most connected nodes at a level.
    Hotspots {
        #[arg(long, value_enum, default_value = "file")]
        level: KindArg,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Dependency cycles between files or packages.
    Cycles {
        #[arg(long, value_enum, default_value = "file")]
        level: KindArg,
    },
    /// Shortest connection between two nodes (ids or paths).
    Path { from: String, to: String },
    /// Run the force-directed layout and report on it (GPU by default).
    Layout {
        #[arg(long, value_enum, default_value = "file")]
        level: KindArg,
        #[arg(long, default_value = "auto")]
        backend: String,
        #[arg(long, default_value_t = 300)]
        iterations: u32,
        /// Write positions JSON to this file.
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
    /// Check the toolchain, GPU, cache and app bridge.
    Doctor,
    /// Drive the running Terrarium app through its local agent bridge.
    App(AppArgs),
    /// Install a session hook so agents see repo state at start (Claude Code, Codex, OpenCode).
    Setup {
        #[arg(value_enum)]
        target: setup::Target,
        /// Install into the user-level config instead of the current project.
        #[arg(long)]
        global: bool,
    },
}

#[derive(Args)]
struct AppArgs {
    #[command(subcommand)]
    cmd: AppCmd,
}

#[derive(Subcommand)]
enum AppCmd {
    /// Is the app running? Prints health.
    Status,
    /// Launch the app (if not running) and wait for the bridge.
    Launch {
        /// Repository to open once the app is up.
        path: Option<PathBuf>,
        /// Path to the app binary (default: auto-detected).
        #[arg(long)]
        bin: Option<PathBuf>,
    },
    /// Scan and open a repository in the app.
    Open { path: PathBuf },
    /// Full UI state: repo, level, selection, camera, layout, filters, panels.
    State,
    /// Select a node by id or path.
    Select { node: String },
    /// Center the camera on a node and expand it.
    Focus { node: String },
    /// Switch hierarchy level.
    Level {
        #[arg(value_enum)]
        level: KindArg,
    },
    /// Search in the app's search box.
    Search { query: String },
    /// Set visible languages / edge kinds / tags (comma-separated; empty clears).
    Filter {
        #[arg(long, value_delimiter = ',')]
        langs: Option<Vec<String>>,
        #[arg(long, value_delimiter = ',')]
        edges: Option<Vec<String>>,
        #[arg(long)]
        tag: Option<String>,
    },
    /// Move the camera (`--x`, `--y`, `--zoom`) or fit everything (`--fit`).
    Camera {
        #[arg(long)]
        x: Option<f64>,
        #[arg(long)]
        y: Option<f64>,
        #[arg(long)]
        zoom: Option<f64>,
        #[arg(long)]
        fit: bool,
    },
    /// Run more layout iterations in the app.
    Layout {
        #[arg(long, default_value_t = 200)]
        iterations: u32,
        #[arg(long)]
        backend: Option<String>,
    },
    /// Save a PNG screenshot of the window.
    Screenshot {
        #[arg(long, default_value = "terrarium.png")]
        out: PathBuf,
    },
    /// Recent log events from the app (backend + frontend).
    Logs {
        #[arg(long, default_value_t = 100)]
        limit: usize,
        /// trace|debug|info|warn|error
        #[arg(long)]
        level: Option<String>,
        /// Only events after this sequence number.
        #[arg(long)]
        since: Option<u64>,
        /// Substring filter on message/target.
        #[arg(long)]
        grep: Option<String>,
    },
    /// Frame timing, memory, counters.
    Metrics,
    /// Span timings (scan, layout, IPC) aggregated since launch.
    Profile,
    /// Evaluate JavaScript inside the webview; the result must be JSON-serialisable.
    Eval { js: String },
    /// Semantic snapshot of the UI (panels, interactive elements, texts).
    Ui,
    /// Click an element by its data-testid.
    Click { testid: String },
    /// Type into an element by its data-testid.
    Type { testid: String, text: String },
    /// Clear selection, filters and camera.
    Reset,
    /// Quit the app.
    Quit,
}

#[derive(Clone, Copy, ValueEnum, Debug)]
enum KindArg {
    Package,
    File,
    Symbol,
}

impl From<KindArg> for NodeKind {
    fn from(k: KindArg) -> NodeKind {
        match k {
            KindArg::Package => NodeKind::Package,
            KindArg::File => NodeKind::File,
            KindArg::Symbol => NodeKind::Symbol,
        }
    }
}

fn main() {
    init_tracing();
    let cli = Cli::parse();
    let json = cli.json;
    let code = match run(cli) {
        Ok(v) => {
            emit(&v, json);
            0
        }
        Err(e) => {
            let msg = e.to_string();
            let mut v = json!({ "error": msg });
            if let Some(h) = help_for_error(&e) {
                v["help"] = json!(h);
            }
            emit(&v, json);
            1
        }
    };
    std::process::exit(code);
}

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_from_env("TERRARIUM_LOG").unwrap_or_else(|_| EnvFilter::new("off"));
    let _ = fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

fn emit(v: &Value, json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(v).unwrap());
    } else {
        print!("{}", toon::encode(v));
    }
}

fn help_for_error(e: &anyhow::Error) -> Option<String> {
    let s = e.to_string();
    if s.contains("no cached graph") {
        Some("Run `terrarium scan <path>` first".into())
    } else if s.contains("not reachable") {
        Some("Run `terrarium app launch` to start the app".into())
    } else if s.contains("no node") {
        Some("Run `terrarium search <name>` to find ids and paths".into())
    } else {
        None
    }
}

fn bin_path() -> String {
    let p = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "terrarium".into());
    match dirs::home_dir() {
        Some(h) => p.replacen(&h.to_string_lossy().to_string(), "~", 1),
        None => p,
    }
}

fn repo_root(cli_repo: &Option<PathBuf>) -> PathBuf {
    cli_repo
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn load_graph(root: &Path) -> Result<Graph> {
    let entry =
        cache::find_entry(root).ok_or_else(|| anyhow!("no cached graph for {}", root.display()))?;
    Graph::load(Path::new(&entry.file))
        .with_context(|| format!("cannot load cached graph {}", entry.file))
}

fn resolve_node(g: &Graph, s: &str) -> Result<u32> {
    if let Ok(id) = s.parse::<u32>() {
        if (id as usize) < g.nodes.len() {
            return Ok(id);
        }
        return Err(anyhow!(
            "no node with id {id} (graph has {} nodes)",
            g.nodes.len()
        ));
    }
    if let Some(n) = g.find_by_path(s) {
        return Ok(n.id);
    }
    // suffix match on path, then unique name match
    let mut hits: Vec<&terrarium_core::Node> =
        g.nodes.iter().filter(|n| n.path.ends_with(s)).collect();
    if hits.is_empty() {
        hits = g.nodes.iter().filter(|n| n.name == s).collect();
    }
    match hits.len() {
        1 => Ok(hits[0].id),
        0 => Err(anyhow!("no node matches `{s}`")),
        n => Err(anyhow!(
            "`{s}` is ambiguous ({n} matches): {}",
            hits.iter()
                .take(5)
                .map(|h| h.path.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn node_brief(g: &Graph, id: u32) -> Value {
    let n = g.node(id);
    json!({ "id": n.id, "path": n.path, "kind": n.kind, "lang": n.lang })
}

fn run(cli: Cli) -> Result<Value> {
    let root = repo_root(&cli.repo);
    match cli.cmd {
        None => home(&root),
        Some(Cmd::Scan {
            path,
            out,
            no_external,
        }) => {
            let path = path.unwrap_or(root);
            let opts = ScanOptions {
                include_external: !no_external,
                ..Default::default()
            };
            let g = terrarium_core::scan(&path, &opts)?;
            let cached = cache::store(&g)?;
            if let Some(o) = &out {
                g.save(o)?;
            }
            let mut v = stats_value(&g);
            v["cached"] = json!(cached.to_string_lossy());
            if let Some(o) = out {
                v["out"] = json!(o.to_string_lossy());
            }
            v["help"] = json!([
                "Run `terrarium flows` to see cross-language data flows",
                "Run `terrarium hotspots` for the most connected files",
                "Run `terrarium app open <path>` to view it in the app",
            ]);
            Ok(v)
        }
        Some(Cmd::Stats) => {
            let g = load_graph(&root)?;
            Ok(stats_value(&g))
        }
        Some(Cmd::Nodes {
            kind,
            lang,
            tag,
            path,
            limit,
            fields,
        }) => {
            let g = load_graph(&root)?;
            let kind: NodeKind = kind.into();
            let all: Vec<&terrarium_core::Node> = g
                .nodes
                .iter()
                .filter(|n| n.kind == kind)
                .filter(|n| lang.as_ref().map(|l| n.lang.label() == l).unwrap_or(true))
                .filter(|n| {
                    tag.as_ref()
                        .map(|t| n.tags.iter().any(|x| x.starts_with(t)))
                        .unwrap_or(true)
                })
                .filter(|n| {
                    path.as_ref()
                        .map(|p| n.path.contains(p.as_str()))
                        .unwrap_or(true)
                })
                .collect();
            let total = all.len();
            let rows: Vec<Value> = all
                .iter()
                .take(limit)
                .map(|n| {
                    let mut o = json!({ "id": n.id, "path": n.path, "lang": n.lang });
                    for f in &fields {
                        match f.as_str() {
                            "loc" => o["loc"] = json!(n.loc),
                            "tags" => o["tags"] = json!(n.tags.join(" ")),
                            "parent" => o["parent"] = json!(n.parent),
                            "span" => o["span"] = json!(n.span.map(|(a, b)| format!("{a}-{b}"))),
                            "name" => o["name"] = json!(n.name),
                            "external" => o["external"] = json!(n.external),
                            _ => {}
                        }
                    }
                    o
                })
                .collect();
            if total == 0 {
                return Ok(
                    json!({ "nodes": format!("0 {} nodes match", kind_label(kind)), "help": ["Run `terrarium nodes --kind file` to list files"] }),
                );
            }
            let mut v =
                json!({ "count": format!("{} of {total} total", rows.len()), "nodes": rows });
            let mut help = vec!["Run `terrarium show <id>` for neighbours and tags".to_string()];
            if total > limit {
                help.push(format!(
                    "Run `terrarium nodes --kind {} --limit {total}` for all",
                    kind_label(kind)
                ));
            }
            v["help"] = json!(help);
            Ok(v)
        }
        Some(Cmd::Search { query, limit }) => {
            let g = load_graph(&root)?;
            let hits = g.search(&query, limit);
            if hits.is_empty() {
                return Ok(json!({ "results": format!("0 nodes match `{query}`") }));
            }
            Ok(json!({
                "results": hits.iter().map(|n| json!({ "id": n.id, "path": n.path, "kind": n.kind, "lang": n.lang })).collect::<Vec<_>>(),
                "help": ["Run `terrarium show <id>` for details"]
            }))
        }
        Some(Cmd::Show { node, full }) => {
            let g = load_graph(&root)?;
            let id = resolve_node(&g, &node)?;
            let n = g.node(id);
            let neighbours = query::neighbours(&g, id);
            let total = neighbours.len();
            let shown: Vec<Value> = neighbours
                .iter()
                .take(if full { usize::MAX } else { 40 })
                .map(|nb| json!({ "dir": nb.direction, "edge": nb.edge, "id": nb.id, "path": nb.path, "label": nb.label.clone().unwrap_or_default(), "w": nb.weight }))
                .collect();
            let children: Vec<Value> = g
                .children(id)
                .map(|c| json!({ "id": c.id, "name": c.name, "kind": c.kind }))
                .collect();
            let mut v = json!({
                "node": {
                    "id": n.id, "kind": n.kind, "name": n.name, "path": n.path, "lang": n.lang,
                    "parent": n.parent, "loc": n.loc, "symbol_kind": n.symbol_kind, "span": n.span.map(|(a, b)| format!("{a}-{b}")),
                    "tags": n.tags, "external": n.external,
                    "children": children.len(),
                },
                "neighbours": shown,
            });
            if !children.is_empty() && children.len() <= 60 {
                v["children"] = json!(children);
            }
            if total > shown.len() {
                v["help"] = json!([format!(
                    "Run `terrarium show {id} --full` for all {total} neighbours"
                )]);
            }
            Ok(v)
        }
        Some(Cmd::Flows { limit }) => {
            let g = load_graph(&root)?;
            let flows = query::flows(&g);
            if flows.is_empty() {
                return Ok(
                    json!({ "flows": "0 cross-boundary flows found", "help": ["Run `terrarium boundaries` to see http/ipc/queue call sites that did not pair up"] }),
                );
            }
            let total = flows.len();
            let rows: Vec<Value> = flows.iter().take(limit).map(|f| json!({ "from": f.from_path, "from_lang": f.from_lang, "to": f.to_path, "to_lang": f.to_lang, "via": f.label })).collect();
            let mut v =
                json!({ "count": format!("{} of {total} total", rows.len()), "flows": rows });
            if total > limit {
                v["help"] = json!([format!("Run `terrarium flows --limit {total}` for all")]);
            }
            Ok(v)
        }
        Some(Cmd::Boundaries { tag, limit }) => {
            let g = load_graph(&root)?;
            let b = query::boundaries(&g, tag.as_deref());
            if b.is_empty() {
                return Ok(
                    json!({ "boundaries": format!("0 boundary nodes{}", tag.map(|t| format!(" with tag `{t}`")).unwrap_or_default()) }),
                );
            }
            let total = b.len();
            let rows: Vec<Value> = b.iter().take(limit).map(|x| json!({ "id": x.id, "path": x.path, "lang": x.lang, "tags": x.tags.join(" ") })).collect();
            Ok(json!({ "count": format!("{} of {total} total", rows.len()), "boundaries": rows }))
        }
        Some(Cmd::Hotspots { level, limit }) => {
            let g = load_graph(&root)?;
            let h = query::hotspots(&g, level.into(), limit);
            Ok(
                json!({ "hotspots": h.iter().map(|x| json!({ "id": x.id, "path": x.path, "in": x.fan_in, "out": x.fan_out, "loc": x.loc })).collect::<Vec<_>>() }),
            )
        }
        Some(Cmd::Cycles { level }) => {
            let g = load_graph(&root)?;
            let c = query::cycles(&g, level.into());
            if c.is_empty() {
                return Ok(
                    json!({ "cycles": format!("0 cycles at {} level", kind_label(level.into())) }),
                );
            }
            Ok(
                json!({ "count": c.len(), "cycles": c.iter().map(|cy| json!({ "size": cy.len(), "members": cy.iter().map(|id| g.node(*id).path.clone()).collect::<Vec<_>>().join(" ") })).collect::<Vec<_>>() }),
            )
        }
        Some(Cmd::Path { from, to }) => {
            let g = load_graph(&root)?;
            let (a, b) = (resolve_node(&g, &from)?, resolve_node(&g, &to)?);
            match query::path_between(&g, a, b) {
                Some(p) => Ok(
                    json!({ "hops": p.len() - 1, "path": p.iter().map(|id| node_brief(&g, *id)).collect::<Vec<_>>() }),
                ),
                None => Ok(
                    json!({ "path": format!("no connection between {} and {}", g.node(a).path, g.node(b).path) }),
                ),
            }
        }
        Some(Cmd::Layout {
            level,
            backend,
            iterations,
            out,
        }) => {
            let g = load_graph(&root)?;
            let backend: Backend = backend.parse().map_err(|e: String| anyhow!(e))?;
            let view = g.view(level.into(), None);
            let input = layout_input(&view);
            let (positions, report) =
                terrarium_layout::layout(&input, Params::default(), backend, iterations)?;
            if let Some(o) = &out {
                let rows: Vec<Value> = view
                    .nodes
                    .iter()
                    .zip(&positions)
                    .map(|(n, p)| json!({ "id": n.id, "x": p[0], "y": p[1] }))
                    .collect();
                std::fs::write(o, serde_json::to_vec(&rows)?)?;
            }
            let mut v = serde_json::to_value(&report)?;
            if let Some(o) = out {
                v["out"] = json!(o.to_string_lossy());
            }
            Ok(v)
        }
        Some(Cmd::Doctor) => doctor(&root),
        Some(Cmd::App(a)) => app(a.cmd),
        Some(Cmd::Setup { target, global }) => setup::install(target, global),
    }
}

pub fn layout_input(view: &terrarium_core::ViewGraph) -> Input {
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
        positions: vec![],
        edges,
        groups,
        mass,
    }
}

fn kind_label(k: NodeKind) -> &'static str {
    match k {
        NodeKind::Repo => "repo",
        NodeKind::Package => "package",
        NodeKind::File => "file",
        NodeKind::Symbol => "symbol",
    }
}

fn stats_value(g: &Graph) -> Value {
    let s = &g.stats;
    json!({
        "repo": g.root,
        "scanned_at": g.scanned_at,
        "files": s.files, "symbols": s.symbols, "packages": s.packages, "external_packages": s.external_packages,
        "imports": s.imports, "calls": s.calls, "unresolved_calls": s.unresolved_calls, "flows": s.flows, "loc": s.loc,
        "scan_ms": s.scan_ms,
        "by_lang": s.by_lang.iter().map(|l| json!({ "lang": l.lang, "files": l.files, "loc": l.loc })).collect::<Vec<_>>(),
    })
}

fn home(root: &Path) -> Result<Value> {
    let mut v = json!({ "bin": bin_path(), "description": DESCRIPTION });
    match cache::find_entry(root) {
        Some(e) => {
            v["repo"] = json!({ "root": e.root, "scanned_at": e.scanned_at, "files": e.files, "symbols": e.symbols, "flows": e.flows });
        }
        None => {
            v["repo"] = json!(format!("not scanned: {}", root.display()));
        }
    }
    let b = bridge::Bridge::discover();
    v["app"] = match b.get("/health") {
        Ok(h) => {
            json!({ "running": true, "url": b.base(), "repo": h.get("repo").cloned().unwrap_or(Value::Null), "nodes": h.get("nodes").cloned().unwrap_or(Value::Null) })
        }
        Err(_) => json!({ "running": false }),
    };
    let idx = cache::load_index();
    if !idx.entries.is_empty() {
        v["recent"] = json!(
            idx.entries
                .iter()
                .take(5)
                .map(|e| json!({ "root": e.root, "files": e.files, "flows": e.flows }))
                .collect::<Vec<_>>()
        );
    }
    let mut help = Vec::new();
    if v["repo"].is_string() {
        help.push("Run `terrarium scan` to analyse this repository".to_string());
    } else {
        help.push("Run `terrarium flows` for cross-language data flows".to_string());
        help.push(
            "Run `terrarium hotspots` / `terrarium boundaries` / `terrarium cycles`".to_string(),
        );
        help.push("Run `terrarium show <path>` for one file or symbol".to_string());
    }
    if v["app"]["running"] == json!(false) {
        help.push("Run `terrarium app launch [path]` to open the app".to_string());
    } else {
        help.push(
            "Run `terrarium app state` / `terrarium app screenshot` to inspect the app".to_string(),
        );
    }
    help.push("Run `terrarium doctor` to check GPU and toolchain".to_string());
    v["help"] = json!(help);
    Ok(v)
}

fn doctor(root: &Path) -> Result<Value> {
    let gpu = terrarium_layout::gpu_info();
    let b = bridge::Bridge::discover();
    let app = match b.get("/health") {
        Ok(h) => {
            json!({ "running": true, "url": b.base(), "version": h.get("version").cloned().unwrap_or(Value::Null), "pid": h.get("pid").cloned().unwrap_or(Value::Null) })
        }
        Err(e) => json!({ "running": false, "url": b.base(), "detail": e.to_string() }),
    };
    let home = cache::home_dir();
    let idx = cache::load_index();
    Ok(json!({
        "bin": bin_path(),
        "version": env!("CARGO_PKG_VERSION"),
        "gpu": { "available": gpu.available, "adapter": gpu.adapter, "backend": gpu.backend, "device_type": gpu.device_type },
        "cache": { "dir": home.to_string_lossy(), "graphs": idx.entries.len(), "current_repo_cached": cache::find_entry(root).is_some() },
        "app": app,
        "languages": ["rust", "typescript", "javascript", "python", "go"],
    }))
}

fn app(cmd: AppCmd) -> Result<Value> {
    let b = bridge::Bridge::discover();
    match cmd {
        AppCmd::Status => match b.get("/health") {
            Ok(h) => Ok(h),
            Err(e) => Ok(
                json!({ "running": false, "error": e.to_string(), "help": ["Run `terrarium app launch` to start the app"] }),
            ),
        },
        AppCmd::Launch { path, bin } => {
            if b.is_up() {
                let mut v = json!({ "app": "already running (no-op)", "url": b.base() });
                if let Some(p) = path {
                    v["opened"] = b.post("/scan", json!({ "path": abs(&p)? }))?;
                }
                return Ok(v);
            }
            let exe = bin.or_else(find_app_binary).ok_or_else(|| anyhow!("app binary not found; build it with `cargo build -p terrarium-app` or pass --bin"))?;
            let mut cmd = std::process::Command::new(&exe);
            cmd.stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            if let Some(p) = &path {
                cmd.env("TERRARIUM_OPEN", abs(p)?);
            }
            let child = cmd
                .spawn()
                .with_context(|| format!("cannot start {}", exe.display()))?;
            let pid = child.id();
            drop(child);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while std::time::Instant::now() < deadline {
                let b2 = bridge::Bridge::discover();
                if b2.is_up() {
                    let mut v = json!({ "app": "started", "pid": pid, "url": b2.base(), "bin": exe.to_string_lossy() });
                    if path.is_some() {
                        // wait for the initial scan
                        for _ in 0..200 {
                            if let Ok(s) = b2.get("/state")
                                && s.get("repo").map(|r| !r.is_null()).unwrap_or(false)
                                && s["scanning"] == json!(false)
                            {
                                v["state"] = s;
                                break;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(200));
                        }
                    }
                    v["help"] = json!([
                        "Run `terrarium app state` to inspect the UI",
                        "Run `terrarium app screenshot --out shot.png` to see it"
                    ]);
                    return Ok(v);
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            Err(anyhow!(
                "app started (pid {pid}) but the bridge did not come up within 30s"
            ))
        }
        AppCmd::Open { path } => b.post("/scan", json!({ "path": abs(&path)? })),
        AppCmd::State => b.get("/state"),
        AppCmd::Select { node } => b.post("/select", json!({ "node": node })),
        AppCmd::Focus { node } => b.post("/focus", json!({ "node": node })),
        AppCmd::Level { level } => b.post("/level", json!({ "level": kind_label(level.into()) })),
        AppCmd::Search { query } => b.post("/search", json!({ "q": query })),
        AppCmd::Filter { langs, edges, tag } => b.post(
            "/filter",
            json!({ "langs": langs, "edges": edges, "tag": tag }),
        ),
        AppCmd::Camera { x, y, zoom, fit } => b.post(
            "/camera",
            json!({ "x": x, "y": y, "zoom": zoom, "fit": fit }),
        ),
        AppCmd::Layout {
            iterations,
            backend,
        } => b.post(
            "/layout",
            json!({ "iterations": iterations, "backend": backend }),
        ),
        AppCmd::Screenshot { out } => {
            let bytes = b.get_bytes("/screenshot")?;
            std::fs::write(&out, &bytes)?;
            Ok(json!({ "screenshot": out.to_string_lossy(), "bytes": bytes.len() }))
        }
        AppCmd::Logs {
            limit,
            level,
            since,
            grep,
        } => {
            let mut q = format!("/logs?limit={limit}");
            if let Some(l) = level {
                q.push_str(&format!("&level={l}"));
            }
            if let Some(s) = since {
                q.push_str(&format!("&since={s}"));
            }
            if let Some(g) = grep {
                q.push_str(&format!("&grep={}", urlencode(&g)));
            }
            b.get(&q)
        }
        AppCmd::Metrics => b.get("/metrics"),
        AppCmd::Profile => b.get("/profile"),
        AppCmd::Eval { js } => b.post("/eval", json!({ "js": js })),
        AppCmd::Ui => b.get("/ui"),
        AppCmd::Click { testid } => b.post("/ui/click", json!({ "testid": testid })),
        AppCmd::Type { testid, text } => {
            b.post("/ui/type", json!({ "testid": testid, "text": text }))
        }
        AppCmd::Reset => b.post("/reset", json!({})),
        AppCmd::Quit => match b.post("/quit", json!({})) {
            Ok(v) => Ok(v),
            Err(_) => Ok(json!({ "app": "quit" })),
        },
    }
}

fn abs(p: &Path) -> Result<String> {
    Ok(p.canonicalize()
        .with_context(|| format!("no such path {}", p.display()))?
        .to_string_lossy()
        .to_string())
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn find_app_binary() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("TERRARIUM_APP") {
        return Some(PathBuf::from(p));
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    for cand in [
        dir.join("terrarium-app"),
        dir.join("Terrarium.app/Contents/MacOS/terrarium-app"),
    ] {
        if cand.exists() {
            return Some(cand);
        }
    }
    // sibling profile dirs (running from target/debug while app built in release, or vice versa)
    let target = dir.parent()?;
    for prof in ["debug", "release"] {
        let cand = target.join(prof).join("terrarium-app");
        if cand.exists() {
            return Some(cand);
        }
        let bundle = target
            .join(prof)
            .join("bundle/macos/Terrarium.app/Contents/MacOS/terrarium-app");
        if bundle.exists() {
            return Some(bundle);
        }
    }
    let app = PathBuf::from("/Applications/Terrarium.app/Contents/MacOS/terrarium-app");
    app.exists().then_some(app)
}
