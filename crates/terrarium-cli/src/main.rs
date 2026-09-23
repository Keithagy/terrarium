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
use terrarium_core::atlas::{self, Atlas};
use terrarium_core::{Graph, NodeKind, ScanOptions, cache, discovery, query};

const DESCRIPTION: &str = "Scan multi-language repositories, draw them as C4 diagrams (context, containers, components, code) with every relationship backed by evidence, trace data across languages, and drive the Terrarium app";

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
    /// End-to-end request paths: entry point → calls → every boundary crossed → sinks (db, fs, queue).
    Traces {
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Only traces that reach a language, e.g. `go`.
        #[arg(long)]
        lang: Option<String>,
    },
    /// One trace as a call tree, starting at a node (id or path). Examples:
    /// `terrarium trace web/src/app.ts#main`, `terrarium trace fetchUsers --full`.
    Trace {
        node: String,
        /// Show every step instead of the first 60.
        #[arg(long)]
        full: bool,
    },
    /// Contracts between callers and handlers (HTTP routes, IPC commands, queue topics), gaps first.
    Endpoints {
        /// Only endpoints with no callers or no handler in the repository.
        #[arg(long)]
        gaps: bool,
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
    /// The atlas: the system in context, its containers, their components, and every
    /// relationship with the code that backs it. Examples: `terrarium atlas`,
    /// `terrarium atlas --level components --container api`, `terrarium atlas --dsl`.
    Atlas {
        /// Which diagram: context, containers, or components (of one container).
        #[arg(long, value_enum, default_value = "containers")]
        level: LevelArg,
        /// Container (id, package or name) for `--level components`.
        #[arg(long)]
        container: Option<String>,
        /// Print the whole atlas as Structurizr DSL instead.
        #[arg(long)]
        dsl: bool,
        /// One journey (id or name) as a sequence: its participants and messages at `--level`.
        #[arg(long)]
        journey: Option<String>,
        /// With `--journey`: print a Mermaid sequence diagram instead.
        #[arg(long)]
        mermaid: bool,
    },
    /// Have Claude narrate one journey again, with a note, and save it. One agent; nothing
    /// else in the atlas moves. The journey is an id, a name, or an entry (`web/src/app.ts#main`);
    /// a name nobody has heard of is followed from the code alone. Spends money.
    /// Examples: `terrarium narrate "Sign up" --note "show the welcome email"`.
    Narrate {
        journey: String,
        /// What to pay attention to, in your words.
        #[arg(long, default_value = "")]
        note: String,
        #[arg(long)]
        model: Option<String>,
    },
    /// Have Claude discover the atlas: a surveyor reads the manifests and entry points, a scout
    /// proposes the key flows (unless `--flow` names them), one agent per container groups its
    /// code into components, one agent per journey narrates a path across the system, an
    /// editor writes the guide, and the engine checks every relationship against the code. Saves the atlas for the app. Spends money.
    /// Examples: `terrarium discover`, `terrarium discover --model claude-opus-5-5`, `terrarium discover --reset`.
    Discover {
        /// Model for the agents (default claude-opus-5-5, or TERRARIUM_DISCOVERY_MODEL).
        #[arg(long)]
        model: Option<String>,
        /// Agents running at once.
        #[arg(long, default_value_t = 4)]
        parallel: usize,
        /// Spending cap per agent, in US dollars.
        #[arg(long, default_value_t = 2.0)]
        budget: f64,
        /// Journeys to narrate, at most (when the scout proposes them).
        #[arg(long, default_value_t = 4)]
        journeys: usize,
        /// A flow to narrate, instead of letting the scout propose them: an entry (`web/src/app.ts#main`),
        /// a name (`Sign up`), or both (`Sign up @ api/main.py#post_user`), with an optional note
        /// after ` :: `. Repeat for more.
        /// Example: `--flow "web/src/app.ts#main :: watch the users list" --flow "Nightly cleanup"`.
        #[arg(long)]
        flow: Vec<String>,
        /// Forget the saved atlas and use the engine's.
        #[arg(long)]
        reset: bool,
    },
    /// Have Claude scout the key flows: one agent reads where flows begin (routes, commands,
    /// jobs, handlers, UI actions) and proposes the journeys worth narrating, each matched to a
    /// trace the scanner found, or left for the narrator to find in the code. Saves nothing.
    /// Spends a little money. Pass the ones you keep to `terrarium discover --flow`.
    /// Examples: `terrarium propose`, `terrarium propose --journeys 6`.
    Propose {
        #[arg(long)]
        model: Option<String>,
        /// Flows to propose, at most.
        #[arg(long, default_value_t = 4)]
        journeys: usize,
    },
    /// Check the toolchain, Claude Code, cache and app bridge.
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
    /// Full UI state: repo, level, focus, selection, journey, panels, atlas summary.
    State,
    /// Select an atlas element (container, component, person, external) by id or name, or a file by path.
    Select { element: String },
    /// Search in the app's search box.
    Search { query: String },
    /// Go to a diagram level: `context`, `containers`, `components` (of a container) or `code` (of a component).
    Level {
        #[arg(value_enum)]
        level: LevelArg,
        /// Container or component to focus (id or name).
        #[arg(long)]
        focus: Option<String>,
    },
    /// Play a journey on the diagrams (id or name), optionally at a 1-based step; `--stop` clears it.
    Journey {
        journey: Option<String>,
        #[arg(long)]
        step: Option<usize>,
        /// Show it as a sequence diagram (`--map` goes back to the C4 map).
        #[arg(long)]
        sequence: bool,
        #[arg(long)]
        map: bool,
        #[arg(long)]
        stop: bool,
    },
    /// Put a journey into the app's atlas from a JSON file (the shape `terrarium app atlas` prints
    /// under `journeys`), replacing one with the same id; the engine checks it and saves it.
    JourneySave {
        #[arg(long)]
        file: PathBuf,
    },
    /// Remove a journey (id or name) from the app's atlas.
    JourneyDelete { journey: String },
    /// Have Claude narrate one journey again in the app, with a note (blocks; spends money).
    Narrate {
        journey: String,
        #[arg(long, default_value = "")]
        note: String,
        #[arg(long)]
        model: Option<String>,
    },
    /// The atlas the app is showing (or `--dsl` for Structurizr DSL).
    Atlas {
        #[arg(long)]
        dsl: bool,
    },
    /// Have Claude discover the atlas in the app (blocks until done; spends money).
    Discover {
        #[arg(long)]
        model: Option<String>,
        /// A flow to narrate (entry or name, ` :: note` optional); repeat for more. See `terrarium discover --help`.
        #[arg(long)]
        flow: Vec<String>,
        /// Forget the saved atlas and use the engine's.
        #[arg(long)]
        reset: bool,
    },
    /// Have Claude scout the key flows in the app (blocks; spends a little money). An open plan
    /// sheet fills with the proposals.
    Propose {
        #[arg(long)]
        model: Option<String>,
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
    /// Span timings (scan, build, design, IPC) aggregated since launch.
    Profile,
    /// Evaluate JavaScript inside the webview; the result must be JSON-serialisable.
    Eval { js: String },
    /// Semantic snapshot of the UI (panels, interactive elements, texts).
    Ui,
    /// Click an element by its data-testid.
    Click { testid: String },
    /// Type into an element by its data-testid.
    Type { testid: String, text: String },
    /// Context level, no selection, no journey, fit.
    Reset,
    /// Quit the app.
    Quit,
}

#[derive(Clone, Copy, ValueEnum, Debug)]
enum LevelArg {
    Context,
    Containers,
    Components,
    Code,
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
        Some(Cmd::Traces { limit, lang }) => {
            let g = load_graph(&root)?;
            let mut traces = query::traces(&g);
            if let Some(l) = &lang {
                traces.retain(|t| t.langs.iter().any(|x| x.label() == l.as_str()));
            }
            if traces.is_empty() {
                return Ok(json!({
                    "traces": format!("0 traces cross a boundary{}", lang.map(|l| format!(" into {l}")).unwrap_or_default()),
                    "help": ["Run `terrarium endpoints` to see routes and calls that did not pair up"]
                }));
            }
            let total = traces.len();
            let rows: Vec<Value> = traces.iter().take(limit).map(|t| json!({
                "entry": t.entry_path,
                "hops": t.hops,
                "langs": t.langs.iter().map(|l| l.label()).collect::<Vec<_>>().join(">"),
                "via": t.via.join(" "),
            })).collect();
            let mut help = vec!["Run `terrarium trace <entry>` for the call tree".to_string()];
            if total > limit {
                help.push(format!("Run `terrarium traces --limit {total}` for all"));
            }
            Ok(json!({ "count": format!("{} of {total} total", rows.len()), "traces": rows, "help": help }))
        }
        Some(Cmd::Trace { node, full }) => {
            let g = load_graph(&root)?;
            let id = resolve_node(&g, &node)?;
            let Some(t) = query::trace_from(&g, id) else {
                return Ok(json!({
                    "trace": format!("{} crosses no boundary", g.node(id).path),
                    "help": ["Run `terrarium traces` to list entry points that do"]
                }));
            };
            let total = t.steps.len();
            let shown = if full { total } else { total.min(60) };
            let steps: Vec<Value> = t.steps.iter().take(shown).map(|s| json!({
                "depth": s.depth,
                "at": match s.line { Some(l) => format!("{}:{l}", s.path), None => s.path.clone() },
                "via": match (&s.via, &s.label) {
                    (Some(terrarium_core::EdgeKind::Flow), Some(l)) => l.clone(),
                    (None, _) => "entry".into(),
                    _ => if s.repeat { "call (seen above)".into() } else { "call".into() },
                },
                "sinks": s.sinks.join(" "),
            })).collect();
            let mut v = json!({
                "entry": t.entry_path,
                "hops": t.hops,
                "langs": t.langs.iter().map(|l| l.label()).collect::<Vec<_>>().join(">"),
                "lanes": t.lanes.join(" "),
                "sinks": if t.sinks.is_empty() { "none".to_string() } else { t.sinks.join(" ") },
                "count": format!("{shown} of {total} steps"),
                "steps": steps,
            });
            if shown < total {
                v["help"] = json!([format!("Run `terrarium trace {} --full` for all {total} steps", t.entry_path)]);
            } else if t.truncated {
                v["help"] = json!(["Trace stopped at 300 steps or depth 24; run `terrarium trace <deeper node>` to continue"]);
            }
            Ok(v)
        }
        Some(Cmd::Endpoints { gaps, limit }) => {
            let g = load_graph(&root)?;
            let all = query::endpoints(&g);
            let gap_count = all.iter().filter(|e| e.status != "ok").count();
            let eps: Vec<&query::Endpoint> = all.iter().filter(|e| !gaps || e.status != "ok").collect();
            if eps.is_empty() {
                return Ok(json!({ "endpoints": if gaps { "0 gaps: every route and call pairs up" } else { "0 endpoints found" } }));
            }
            let short = |v: &[query::EndpointRef]| v.iter().map(|r| r.path.clone()).collect::<Vec<_>>().join(" ");
            let total = eps.len();
            let rows: Vec<Value> = eps.iter().take(limit).map(|e| json!({
                "key": e.key,
                "status": e.status,
                "handlers": short(&e.handlers),
                "callers": short(&e.callers),
            })).collect();
            let mut help = vec!["Run `terrarium trace <caller>` to follow a call end to end".to_string()];
            if total > limit {
                help.push(format!("Run `terrarium endpoints --limit {total}` for all"));
            }
            Ok(json!({
                "count": format!("{} of {total} total, {gap_count} gaps", rows.len()),
                "endpoints": rows,
                "help": help,
            }))
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
        Some(Cmd::Atlas { level, container, dsl, journey, mermaid }) => {
            let g = load_graph(&root)?;
            let (a, stale) = atlas::for_graph(&g, cache::load_atlas(Path::new(&g.root)));
            if dsl {
                return Ok(json!({ "dsl": atlas::to_dsl(&a) }));
            }
            if let Some(q) = journey {
                let j = find_journey(&a, &q)?;
                let (lvl, focus) = level_focus(&a, level, container.as_deref())?;
                if mermaid {
                    return Ok(json!({ "mermaid": atlas::to_mermaid(&a, j, lvl, focus.as_deref()) }));
                }
                return journey_value(&a, j, lvl, focus.as_deref());
            }
            atlas_value(&a, stale.as_deref(), level, container.as_deref(), true)
        }
        Some(Cmd::Discover { model, parallel, budget, journeys, flow, reset }) => {
            let g = load_graph(&root)?;
            let groot = Path::new(&g.root).to_path_buf();
            if reset {
                let had = cache::clear_atlas(&groot)?;
                let a = atlas::engine_atlas(&g);
                let mut v = atlas_value(&a, None, LevelArg::Containers, None, false)?;
                v["discover"] = json!(if had { "reset to the engine's atlas" } else { "already the engine's atlas (no-op)" });
                return Ok(v);
            }
            let saved = cache::load_atlas(&groot);
            let keep: Vec<atlas::Journey> = saved.as_ref().map(|a| a.journeys.iter().filter(|j| j.source == "user").cloned().collect()).unwrap_or_default();
            let mut opts = discovery::Options { parallel, budget_usd: budget, max_journeys: journeys, flows: flow.iter().map(|f| parse_flow(f)).collect(), keep, ..discovery::Options::default() };
            if let Some(m) = model {
                opts.model = m;
            }
            let runner = discovery::claude_runner(&groot, &opts);
            // Progress is for people watching; agents read the result on stdout.
            let progress = |p: discovery::Progress| {
                if !matches!(p, discovery::Progress::Verified { .. }) {
                    eprintln!("{}", serde_json::to_string(&p).unwrap_or_default());
                }
            };
            let (a, run) = discovery::discover(&g, &opts, &runner, &progress)?;
            cache::store_atlas(&groot, &a)?;
            let mut v = json!({ "run": run_value(&run) });
            let summary = atlas_value(&a, None, LevelArg::Containers, None, false)?;
            for (k, val) in summary.as_object().unwrap() {
                v[k] = val.clone();
            }
            Ok(v)
        }
        Some(Cmd::Propose { model, journeys }) => {
            let g = load_graph(&root)?;
            let groot = Path::new(&g.root).to_path_buf();
            let (a, _) = atlas::for_graph(&g, cache::load_atlas(&groot));
            let mut opts = discovery::Options { max_journeys: journeys, ..discovery::Options::default() };
            if let Some(m) = model {
                opts.model = m;
            }
            let runner = discovery::claude_runner(&groot, &opts);
            let progress = |p: discovery::Progress| {
                if !matches!(p, discovery::Progress::Proposed { .. }) {
                    eprintln!("{}", serde_json::to_string(&p).unwrap_or_default());
                }
            };
            let (proposals, run) = discovery::propose_flows(&g, &a, &opts, &runner, &progress)?;
            let mut v = proposals_value(&a, &proposals);
            v["run"] = run_value(&run);
            Ok(v)
        }
        Some(Cmd::Narrate { journey, note, model }) => {
            let g = load_graph(&root)?;
            let groot = Path::new(&g.root).to_path_buf();
            let (a, _) = atlas::for_graph(&g, cache::load_atlas(&groot));
            let mut opts = discovery::Options::default();
            if let Some(m) = model {
                opts.model = m;
            }
            let runner = discovery::claude_runner(&groot, &opts);
            let progress = |p: discovery::Progress| {
                if !matches!(p, discovery::Progress::JourneyDone { .. }) {
                    eprintln!("{}", serde_json::to_string(&p).unwrap_or_default());
                }
            };
            let (b, run) = discovery::narrate_one(&g, &a, &journey, &note, &opts, &runner, &progress)?;
            cache::store_atlas(&groot, &b)?;
            let id = b.journeys.iter().find(|j| j.id == journey || j.name.eq_ignore_ascii_case(&journey) || j.entry == journey).map(|j| j.id.clone()).unwrap_or_else(|| atlas::journey_id(if journey.contains('#') { &journey } else { "" }, &journey));
            let j = find_journey(&b, &id)?;
            let mut v = journey_value(&b, j, "containers", None)?;
            v["run"] = run_value(&run);
            Ok(v)
        }
        Some(Cmd::Doctor) => doctor(&root),
        Some(Cmd::App(a)) => app(a.cmd),
        Some(Cmd::Setup { target, global }) => setup::install(target, global),
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

fn element_name(a: &Atlas, id: &str) -> String {
    for c in &a.containers {
        if c.id == id {
            return c.name.clone();
        }
        for k in &c.components {
            if k.id == id {
                return format!("{} / {}", c.name, k.name);
            }
        }
    }
    a.people.iter().find(|p| p.id == id).map(|p| p.name.clone()).or_else(|| a.externals.iter().find(|x| x.id == id).map(|x| x.name.clone())).or_else(|| a.journeys.iter().find(|j| j.id == id).map(|j| j.name.clone())).unwrap_or_else(|| id.to_string())
}

fn rel_row(a: &Atlas, r: &atlas::Relationship) -> Value {
    json!({
        "from": element_name(a, &r.from),
        "to": element_name(a, &r.to),
        "label": r.label,
        "technology": r.technology,
        "source": r.source,
        "evidence": r.evidence.len(),
    })
}

fn run_value(run: &discovery::Run) -> Value {
    json!({
        "model": run.model,
        "cost_usd": (run.cost_usd * 100.0).round() / 100.0,
        "secs": run.secs.round(),
        "agents": run.agents.iter().map(|r| json!({ "role": r.role, "target": r.target.clone().unwrap_or_default(), "ok": r.ok, "reads": r.reads, "cost_usd": (r.cost_usd * 100.0).round() / 100.0, "secs": r.secs.round(), "error": r.error.clone().unwrap_or_default() })).collect::<Vec<_>>(),
    })
}

/// `entry-or-name [:: note]`, or `name @ entry [:: note]`, as a flow request.
fn parse_flow(s: &str) -> discovery::FlowRequest {
    let (head, note) = s.split_once(" :: ").map(|(h, n)| (h.trim(), n.trim())).unwrap_or((s.trim(), ""));
    let note = note.to_string();
    if let Some((name, entry)) = head.split_once(" @ ") {
        discovery::FlowRequest { entry: entry.trim().into(), name: name.trim().into(), note, ..Default::default() }
    } else if head.contains('#') {
        discovery::FlowRequest { entry: head.into(), note, ..Default::default() }
    } else {
        discovery::FlowRequest { name: head.into(), note, ..Default::default() }
    }
}

/// The scout's proposals as rows, with the `--flow` that keeps each one.
fn proposals_value(a: &Atlas, proposals: &[discovery::Proposal]) -> Value {
    let flag = |p: &discovery::Proposal| if p.entry.is_empty() { format!("--flow \"{}\"", p.name) } else { format!("--flow \"{} @ {}\"", p.name, p.entry) };
    let mut help: Vec<String> = Vec::new();
    if !proposals.is_empty() {
        help.push(format!("Run `terrarium discover {}` to narrate these (spends money); drop or reword any flow, add ` :: <note>` to steer one", proposals.iter().map(flag).collect::<Vec<_>>().join(" ")));
    }
    help.push("Run `terrarium discover` to let the scout pick again inside a full discovery (spends money)".into());
    json!({
        "proposed": format!("{} {}", proposals.len(), if proposals.len() == 1 { "flow" } else { "flows" }),
        "flows": proposals.iter().map(|p| json!({
            "name": p.name,
            "entry": if p.entry.is_empty() { "(the narrator finds it)".to_string() } else { p.entry.clone() },
            "matched": p.matched,
            "hops": p.hops,
            "containers": p.containers.iter().map(|c| element_name(a, c)).collect::<Vec<_>>().join(", "),
            "why": p.why,
        })).collect::<Vec<_>>(),
        "help": help,
    })
}

fn find_journey<'a>(a: &'a Atlas, q: &str) -> Result<&'a atlas::Journey> {
    let ql = q.to_lowercase();
    a.journeys
        .iter()
        .find(|j| j.id == q || j.name.to_lowercase() == ql || j.entry == q || j.id == format!("j:{}", atlas::slug(q)))
        .ok_or_else(|| anyhow!("no journey `{q}`; run `terrarium atlas` to list them"))
}

/// The level name and focus container for a journey projection.
fn level_focus(a: &Atlas, level: LevelArg, container: Option<&str>) -> Result<(&'static str, Option<String>)> {
    Ok(match level {
        LevelArg::Context => ("context", None),
        LevelArg::Containers => ("containers", None),
        LevelArg::Components | LevelArg::Code => {
            let shown: Vec<&atlas::Container> = a.containers.iter().filter(|c| !c.hidden).collect();
            let c = match container {
                Some(q) => shown.iter().find(|c| c.id == q || c.package == q || c.name.eq_ignore_ascii_case(q) || c.id == format!("c:{}", atlas::slug(q))).copied().ok_or_else(|| anyhow!("no container `{q}`; run `terrarium atlas` to list them"))?,
                None => *shown.first().ok_or_else(|| anyhow!("the atlas has no containers"))?,
            };
            ("components", Some(c.id.clone()))
        }
    })
}

/// One journey as a sequence at one level: participants, then messages in order.
fn journey_value(a: &Atlas, j: &atlas::Journey, level: &str, focus: Option<&str>) -> Result<Value> {
    let p = atlas::project(a, j, level, focus);
    Ok(json!({
        "journey": { "id": j.id, "name": j.name, "summary": j.summary, "entry": j.entry, "source": j.source, "note": j.note, "why": j.why, "messages": j.messages.len(), "claimed": j.messages.iter().filter(|m| m.source == "claimed").count() },
        "level": p.level,
        "focus": focus.map(|f| element_name(a, f)),
        "participants": p.participants.iter().map(|id| json!({ "id": id, "name": element_name(a, id) })).collect::<Vec<_>>(),
        "messages": p.messages.iter().map(|m| json!({ "n": m.n, "from": element_name(a, &m.from), "to": element_name(a, &m.to), "kind": m.kind, "label": m.label, "source": m.source, "by": m.by, "caption": m.caption })).collect::<Vec<_>>(),
        "help": [
            "Run `terrarium atlas --journey <id> --level components --container <name>` to open one container's components",
            "Run `terrarium atlas --journey <id> --mermaid` for a Mermaid sequence diagram",
            "Run `terrarium narrate <id> --note \"...\"` to have Claude narrate it again (spends money)"
        ],
    }))
}

/// One diagram of the atlas, as the CLI prints it.
fn atlas_value(a: &Atlas, stale: Option<&str>, level: LevelArg, container: Option<&str>, with_help: bool) -> Result<Value> {
    let shown: Vec<&atlas::Container> = a.containers.iter().filter(|c| !c.hidden).collect();
    let mut v = json!({
        "atlas": {
            "system": a.system.name,
            "source": match &a.model { Some(m) => format!("{} ({m})", a.source), None => a.source.clone() },
            "purpose": a.system.purpose,
            "summary": a.system.summary,
            "containers": shown.len(),
            "components": shown.iter().map(|c| c.components.len()).sum::<usize>(),
            "people": a.people.len(),
            "externals": a.externals.len(),
            "journeys": a.journeys.len(),
            "check": format!("{} relationships backed by code, {} from the survey, {} claimed", a.report.backed, a.report.survey, a.report.claimed),
        },
    });
    match level {
        LevelArg::Context => {
            v["people"] = json!(a.people.iter().map(|p| json!({ "name": p.name, "description": p.description })).collect::<Vec<_>>());
            v["externals"] = json!(a.externals.iter().map(|x| json!({ "name": x.name, "kind": x.kind, "description": x.description })).collect::<Vec<_>>());
            v["relationships"] = json!(a.relationships.iter().filter(|r| r.level == "container" && (r.from.starts_with("p:") || r.to.starts_with("x:") || r.from.starts_with("x:"))).map(|r| rel_row(a, r)).collect::<Vec<_>>());
        }
        LevelArg::Containers => {
            v["containers"] = json!(shown.iter().map(|c| json!({ "id": c.id, "name": c.name, "kind": c.kind, "technology": c.technology, "components": c.components.len(), "description": c.description })).collect::<Vec<_>>());
            v["relationships"] = json!(a.relationships.iter().filter(|r| r.level == "container").map(|r| rel_row(a, r)).collect::<Vec<_>>());
        }
        LevelArg::Components | LevelArg::Code => {
            let c = match container {
                Some(q) => shown.iter().find(|c| c.id == q || c.package == q || c.name.eq_ignore_ascii_case(q) || c.id == format!("c:{}", atlas::slug(q))).copied().ok_or_else(|| anyhow!("no container `{q}`; run `terrarium atlas` to list them"))?,
                None => *shown.first().ok_or_else(|| anyhow!("the atlas has no containers"))?,
            };
            v["container"] = json!({ "id": c.id, "name": c.name, "kind": c.kind, "technology": c.technology, "description": c.description, "responsibilities": c.responsibilities });
            v["components"] = json!(c.components.iter().map(|k| json!({ "id": k.id, "name": k.name, "files": k.files.len(), "description": k.description, "paths": k.files.join(" ") })).collect::<Vec<_>>());
            v["relationships"] = json!(a.relationships.iter().filter(|r| r.level == "component" && (r.from.starts_with(&c.id) || r.to.starts_with(&c.id))).map(|r| rel_row(a, r)).collect::<Vec<_>>());
        }
    }
    if !a.journeys.is_empty() {
        v["journeys"] = json!(a.journeys.iter().map(|j| json!({ "id": j.id, "name": j.name, "messages": j.messages.len(), "steps": j.steps.len(), "source": j.source, "entry": j.entry })).collect::<Vec<_>>());
    }
    if !a.guide.callouts.is_empty() {
        v["callouts"] = json!(a.guide.callouts.iter().map(|c| json!({ "title": c.title, "detail": c.detail })).collect::<Vec<_>>());
    }
    if !a.report.notes.is_empty() {
        v["notes"] = json!(a.report.notes);
    }
    if let Some(st) = stale {
        v["stale"] = json!(st);
    }
    if with_help {
        let mut help = vec!["Run `terrarium atlas --level context` for people and outside systems".to_string(), "Run `terrarium atlas --level components --container <name>` for one container's components".to_string()];
        if a.source == "engine" {
            help.push("Run `terrarium discover` to have Claude read the code and name every part (spends money)".into());
            help.push("Run `terrarium propose` to see which flows Claude would narrate first (spends a little)".into());
        } else {
            help.push("Run `terrarium discover --reset` to go back to the engine's atlas".into());
        }
        help.push("Run `terrarium atlas --dsl` for Structurizr DSL".into());
        v["help"] = json!(help);
    }
    Ok(v)
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
        help.push("Run `terrarium atlas` for the C4 diagrams: containers, components, relationships".to_string());
        help.push("Run `terrarium discover` to have Claude name every part (spends money)".to_string());
        help.push("Run `terrarium traces` for end-to-end paths across languages".to_string());
        help.push("Run `terrarium endpoints --gaps` for routes nobody calls and calls nobody serves".to_string());
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
    help.push("Run `terrarium doctor` to check Claude Code and the toolchain".to_string());
    v["help"] = json!(help);
    Ok(v)
}

fn doctor(root: &Path) -> Result<Value> {
    let claude = discovery::find_claude();
    let claude_version = std::process::Command::new(&claude)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
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
        "claude": match &claude_version {
            Some(v) => json!({ "available": true, "bin": claude.to_string_lossy(), "version": v, "discovery_model": discovery::Options::default().model }),
            None => json!({ "available": false, "bin": claude.to_string_lossy(), "detail": "`terrarium discover` needs Claude Code; install it or set TERRARIUM_CLAUDE" }),
        },
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
        AppCmd::Select { element } => b.post("/select", json!({ "element": element })),
        AppCmd::Search { query } => b.post("/search", json!({ "q": query })),
        AppCmd::Level { level, focus } => b.post("/level", json!({ "level": format!("{level:?}").to_lowercase(), "focus": focus })),
        AppCmd::Journey { journey, step, sequence, map, stop } => b.post("/journey", if stop { json!({}) } else { json!({ "journey": journey, "step": step, "view": if sequence { Some("sequence") } else if map { Some("map") } else { None } }) }),
        AppCmd::JourneySave { file } => {
            let text = std::fs::read_to_string(&file).with_context(|| format!("cannot read {}", file.display()))?;
            let j: Value = serde_json::from_str(&text).with_context(|| format!("{} is not JSON", file.display()))?;
            b.post("/journey/save", json!({ "journey": j }))
        }
        AppCmd::JourneyDelete { journey } => b.post("/journey/delete", json!({ "journey": journey })),
        AppCmd::Narrate { journey, note, model } => b.post_long("/journey/narrate", json!({ "journey": journey, "note": note, "model": model })),
        AppCmd::Atlas { dsl } => {
            if dsl {
                Ok(json!({ "dsl": String::from_utf8_lossy(&b.get_bytes("/atlas/dsl")?).to_string() }))
            } else {
                let v = b.get("/atlas")?;
                let a: Atlas = serde_json::from_value(v["atlas"].clone()).context("the app returned an atlas this CLI cannot read")?;
                atlas_value(&a, v["stale"].as_str(), LevelArg::Containers, None, false)
            }
        }
        AppCmd::Discover { model, flow, reset } => {
            if reset {
                b.post("/discover/reset", json!({}))
            } else {
                b.post_long("/discover", json!({ "model": model, "flows": flow.iter().map(|f| parse_flow(f)).collect::<Vec<_>>() }))
            }
        }
        AppCmd::Propose { model } => {
            let v = b.post_long("/discover/propose", json!({ "model": model }))?;
            let a: Atlas = serde_json::from_value(b.get("/atlas")?["atlas"].clone()).context("the app returned an atlas this CLI cannot read")?;
            let proposals: Vec<discovery::Proposal> = serde_json::from_value(v["proposals"].clone()).context("the app returned proposals this CLI cannot read")?;
            let mut out = proposals_value(&a, &proposals);
            out["run"] = v["run"].clone();
            out["plan"] = v["plan"].clone();
            Ok(out)
        }
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
