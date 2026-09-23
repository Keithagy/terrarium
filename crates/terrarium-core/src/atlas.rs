//! The atlas: a repository as a set of C4 diagrams that hold together.
//!
//! C4 reads a system at four levels: the system in its context (who uses it,
//! what it talks to), the containers it runs as (web app, service, worker,
//! database), the components inside each container, and the code. The engine
//! writes a plain atlas from the graph alone ([`engine_atlas`]): packages become
//! containers, directories become components, imports and calls become
//! relationships. Agents can write a richer one (see `discovery`), naming things
//! for what they do and explaining why. Whoever wrote it, [`verify`] checks every
//! element against the graph: a component only holds files that exist in its
//! container, and every relationship is either backed by code (with the edges as
//! evidence), declared by a survey (people, external systems) or marked as a
//! claim the code does not show. Nothing an agent invents can pass for fact.

use crate::model::*;
use crate::query;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

pub const ATLAS_SCHEMA: u32 = 1;

/// How many journeys the engine picks by default.
pub const ENGINE_JOURNEYS: usize = 5;

// ---- model ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Atlas {
    pub schema: u32,
    /// `engine`, or the agent that wrote it (`claude`).
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The scan this atlas was made for; a newer scan makes it stale.
    pub scanned_at: String,
    pub system: System,
    pub people: Vec<Person>,
    pub externals: Vec<External>,
    pub containers: Vec<Container>,
    pub relationships: Vec<Relationship>,
    pub journeys: Vec<Journey>,
    pub guide: Guide,
    #[serde(default)]
    pub report: Report,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct System {
    pub name: String,
    /// One sentence: what the software is for.
    pub purpose: String,
    /// A short paragraph a newcomer reads first.
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Person {
    /// `p:<slug>`
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct External {
    /// `x:<slug>`
    pub id: String,
    pub name: String,
    /// `database`, `queue`, `filesystem`, `service`, `system`.
    pub kind: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Container {
    /// `c:<slug>`
    pub id: String,
    /// The package this container stands for.
    pub package: String,
    pub name: String,
    /// `web`, `desktop`, `service`, `worker`, `cli`, `library`, `tooling`.
    pub kind: String,
    /// The language most of its files are in (`rust`, `typescript`…); the engine sets it.
    #[serde(default)]
    pub language: String,
    pub technology: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub responsibilities: Vec<String>,
    /// Tooling and empty packages are kept in the data but left off the diagrams.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    pub components: Vec<Component>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Component {
    /// `c:<container>/<slug>`
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub technology: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub responsibilities: Vec<String>,
    /// Repo-relative file paths.
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Relationship {
    pub from: String,
    pub to: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub technology: String,
    /// `code` (backed by edges in the graph, see `evidence`), `survey` (declared
    /// by the survey: people and external systems the code cannot show) or
    /// `claimed` (an agent said so; the code does not show it).
    pub source: String,
    /// `container` or `component`.
    pub level: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Evidence {
    pub from: String,
    pub to: String,
    /// `imports`, `calls`, `flow`, `tag`.
    pub via: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

/// One thing the system does end to end, as a numbered path across the diagrams.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Journey {
    /// `j:<slug of entry>`
    pub id: String,
    pub name: String,
    pub summary: String,
    /// Entry symbol path (`web/src/app.ts#main`).
    pub entry: String,
    pub steps: Vec<JourneyStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JourneyStep {
    /// Element ids (components, or an external for a sink).
    pub from: String,
    pub to: String,
    pub from_path: String,
    pub to_path: String,
    /// `calls`, or a flow label (`http /api/users`), or a sink (`db`).
    pub label: String,
    pub caption: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Guide {
    pub start_here: Vec<Pointer>,
    pub callouts: Vec<Callout>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pointer {
    pub element: String,
    pub why: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Callout {
    pub title: String,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<String>,
}

/// What the check found: how much of the atlas the code backs, and what was fixed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Report {
    pub backed: u32,
    pub survey: u32,
    pub claimed: u32,
    /// Files the agents did not place; the engine put them in an "Other files" component.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unplaced: Vec<String>,
    /// Repairs, in plain words.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

// ---- ids and names --------------------------------------------------------------------

pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let t = out.trim_matches('-').to_string();
    if t.is_empty() { "x".into() } else { t }
}

pub fn container_id(package: &str) -> String {
    format!("c:{}", slug(package))
}

pub fn component_id(container: &str, name: &str) -> String {
    format!("{container}/{}", slug(name))
}

pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "web" => "Web app",
        "desktop" => "Desktop app",
        "service" => "Service",
        "worker" => "Worker",
        "cli" => "Command line",
        "library" => "Library",
        "tooling" => "Tooling",
        _ => "Container",
    }
}

fn lang_name(l: Lang) -> &'static str {
    match l {
        Lang::Rust => "Rust",
        Lang::TypeScript => "TypeScript",
        Lang::JavaScript => "JavaScript",
        Lang::Python => "Python",
        Lang::Go => "Go",
        Lang::Other => "",
    }
}

/// The language most of a package's lines are in.
pub fn main_language(g: &Graph, files: &[NodeId]) -> String {
    let mut by: HashMap<Lang, u32> = HashMap::new();
    for f in files {
        let n = g.node(*f);
        *by.entry(n.lang).or_default() += n.loc.max(1);
    }
    by.into_iter().max_by_key(|(l, n)| (*n, l.label())).map(|(l, _)| l.label().to_string()).unwrap_or_default()
}

fn base(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn stem(path: &str) -> &str {
    let b = base(path);
    b.rsplit_once('.').map(|(s, _)| s).unwrap_or(b)
}

fn title_case(s: &str) -> String {
    let mut out = String::new();
    for (i, w) in s.split(['-', '_', ' ']).filter(|w| !w.is_empty()).enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let mut c = w.chars();
        if let Some(f) = c.next() {
            out.extend(f.to_uppercase());
            out.push_str(c.as_str());
        }
    }
    out
}

// ---- what the engine knows -----------------------------------------------------------

/// The graph as the atlas sees it: internal packages, their files, and the file-level edges.
pub struct Facts {
    /// Package name -> (package node, files in path order).
    pub packages: BTreeMap<String, (NodeId, Vec<NodeId>)>,
    /// file -> package name
    pub package_of: HashMap<NodeId, String>,
    /// Edges between internal files (imports, calls) and flows, with evidence.
    pub file_edges: Vec<FileEdge>,
    /// Package -> external dependency names, most imported first.
    pub deps: HashMap<String, Vec<String>>,
    /// Package -> boundary tags found in its files (`http-server`, `db`, `env:X`...).
    pub tags: HashMap<String, Vec<String>>,
}

pub struct FileEdge {
    pub from: NodeId,
    pub to: NodeId,
    pub kind: EdgeKind,
    pub evidence: Evidence,
}

fn file_of(g: &Graph, id: NodeId) -> Option<NodeId> {
    g.ancestor_of_kind(id, NodeKind::File)
}

pub fn facts(g: &Graph) -> Facts {
    let mut packages: BTreeMap<String, (NodeId, Vec<NodeId>)> = BTreeMap::new();
    for n in &g.nodes {
        if n.kind == NodeKind::Package && !n.external {
            packages.insert(n.name.clone(), (n.id, vec![]));
        }
    }
    let mut package_of = HashMap::new();
    let mut files: Vec<&Node> = g.nodes.iter().filter(|n| n.kind == NodeKind::File && !n.external).collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    for f in files {
        let pkg = g.ancestor_of_kind(f.id, NodeKind::Package).map(|p| g.node(p).name.clone()).unwrap_or_else(|| "root".into());
        packages.entry(pkg.clone()).or_insert((f.parent.unwrap_or(0), vec![])).1.push(f.id);
        package_of.insert(f.id, pkg);
    }
    let mut file_edges = Vec::new();
    let mut dep_count: HashMap<(String, String), u32> = HashMap::new();
    for e in &g.edges {
        match e.kind {
            EdgeKind::Contains => continue,
            EdgeKind::Imports => {
                if let Some(a) = file_of(g, e.from) {
                    let to = g.node(e.to);
                    if to.external || (to.kind == NodeKind::Package && to.external) {
                        if let Some(p) = package_of.get(&a) {
                            let name = to.name.trim_start_matches("external:").to_string();
                            *dep_count.entry((p.clone(), name)).or_default() += e.weight;
                        }
                        continue;
                    }
                }
            }
            _ => {}
        }
        let (Some(a), Some(b)) = (file_of(g, e.from), file_of(g, e.to)) else { continue };
        if a == b || !package_of.contains_key(&a) || !package_of.contains_key(&b) {
            continue;
        }
        let via = match e.kind {
            EdgeKind::Imports => "imports",
            EdgeKind::Calls => "calls",
            EdgeKind::Flow => "flow",
            EdgeKind::Contains => unreachable!(),
        };
        file_edges.push(FileEdge {
            from: a,
            to: b,
            kind: e.kind,
            evidence: Evidence { from: g.node(e.from).path.clone(), to: g.node(e.to).path.clone(), via: via.into(), label: e.label.clone(), line: g.node(e.from).span.map(|s| s.0) },
        });
    }
    let mut deps: HashMap<String, Vec<String>> = HashMap::new();
    let mut counted: Vec<((String, String), u32)> = dep_count.into_iter().collect();
    counted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for ((p, name), _) in counted {
        // `crate::commands` in Rust resolves to a sibling file, not a dependency.
        let sibling = packages.get(&p).is_some_and(|(_, files)| files.iter().any(|f| stem(&g.node(*f).path) == name));
        if !sibling {
            deps.entry(p).or_default().push(name);
        }
    }
    let mut tags: HashMap<String, Vec<String>> = HashMap::new();
    for n in &g.nodes {
        if n.external || n.tags.is_empty() {
            continue;
        }
        let Some(f) = file_of(g, n.id) else { continue };
        let Some(p) = package_of.get(&f) else { continue };
        let v = tags.entry(p.clone()).or_default();
        for t in &n.tags {
            if !v.contains(t) {
                v.push(t.clone());
            }
        }
    }
    Facts { packages, package_of, file_edges, deps, tags }
}

const STDLIB: [&str; 25] = [
    "os", "sys", "json", "re", "time", "typing", "pathlib", "sqlite3", "fs", "path", "encoding/json", "net/http", "database/sql", "fmt", "context", "errors", "strings", "io", "log", "std", "collections", "dataclasses", "subprocess", "http", "url",
];

/// `Python, FastAPI, Celery`: the language plus the notable dependencies.
pub fn technology_of(lang: Lang, deps: &[String]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let l = lang_name(lang);
    if !l.is_empty() {
        parts.push(l.to_string());
    }
    for d in deps.iter().filter(|d| !STDLIB.contains(&d.as_str()) && !d.starts_with("crate") && !d.starts_with('.')).take(3) {
        parts.push(title_case_dep(d));
    }
    parts.join(", ")
}

fn title_case_dep(d: &str) -> String {
    let short = d.rsplit('/').next().unwrap_or(d).trim_start_matches('@');
    match short {
        "fastapi" => "FastAPI".into(),
        "celery" => "Celery".into(),
        "tauri" | "api" if d.contains("tauri") => "Tauri".into(),
        "three" => "three.js".into(),
        "axum" => "Axum".into(),
        "react" => "React".into(),
        "vite" => "Vite".into(),
        "express" => "Express".into(),
        "flask" => "Flask".into(),
        "django" => "Django".into(),
        "sqlalchemy" => "SQLAlchemy".into(),
        "reqwest" => "reqwest".into(),
        other => other.to_string(),
    }
}

/// Guess what a package runs as from what its code touches.
pub fn kind_of(g: &Graph, files: &[NodeId], tags: &[String]) -> &'static str {
    if files.is_empty() {
        return "tooling";
    }
    let has = |t: &str| tags.iter().any(|x| x == t);
    let lang = g.node(files[0]).lang;
    let has_main = files.iter().any(|f| {
        let n = g.node(*f);
        matches!(base(&n.path), "main.rs" | "main.go" | "main.py" | "main.ts" | "index.ts" | "app.ts" | "main.js" | "index.js" | "__main__.py")
            || g.children(*f).any(|s| s.name == "main")
    });
    if has("ipc-server") {
        "desktop"
    } else if has("http-server") {
        "service"
    } else if has("queue") && !has("http-client") && !matches!(lang, Lang::TypeScript | Lang::JavaScript) {
        "worker"
    } else if matches!(lang, Lang::TypeScript | Lang::JavaScript) && (has("ipc-client") || has("http-client") || has_main) {
        "web"
    } else if has_main {
        "cli"
    } else {
        "library"
    }
}

// ---- the engine's atlas ---------------------------------------------------------------

/// Group a package's files into components by directory. `src` and `lib` fold
/// into their parent; a package whose files all sit together gets one component
/// per file when there are few, or one component when there are many.
pub fn engine_components(g: &Graph, cid: &str, pkg_dir: &str, files: &[NodeId]) -> Vec<Component> {
    let rel = |p: &str| -> String {
        let r = if pkg_dir.is_empty() { p } else { p.strip_prefix(pkg_dir).map(|s| s.trim_start_matches('/')).unwrap_or(p) };
        r.to_string()
    };
    let group_of = |p: &str| -> String {
        let r = rel(p);
        let mut dirs: Vec<&str> = r.split('/').collect();
        dirs.pop();
        let dirs: Vec<&str> = dirs.into_iter().filter(|d| !matches!(*d, "src" | "lib" | "pkg" | "internal" | "app")).collect();
        dirs.first().map(|s| s.to_string()).unwrap_or_default()
    };
    let mut groups: BTreeMap<String, Vec<NodeId>> = BTreeMap::new();
    for f in files {
        groups.entry(group_of(&g.node(*f).path)).or_default().push(*f);
    }
    let one_group = groups.len() == 1;
    let mut out = Vec::new();
    let mut used: HashSet<String> = HashSet::new();
    let describe = |ids: &[NodeId]| -> String {
        let syms: Vec<String> = ids.iter().flat_map(|f| g.children(*f).map(|s| s.name.clone())).take(4).collect();
        let n: usize = ids.iter().map(|f| g.children(*f).count()).sum();
        let files_n = ids.len();
        let mut s = format!("{files_n} {}", if files_n == 1 { "file" } else { "files" });
        if n > 0 {
            s.push_str(&format!(", {n} {}: {}", if n == 1 { "part" } else { "parts" }, syms.join(", ")));
            if n > 4 {
                s.push_str(", …");
            }
        }
        s
    };
    if one_group && files.len() <= 6 {
        for f in files {
            let n = g.node(*f);
            let mut name = title_case(stem(&n.path));
            let mut i = 2;
            while !used.insert(name.clone()) {
                name = format!("{} {i}", title_case(stem(&n.path)));
                i += 1;
            }
            out.push(Component { id: component_id(cid, &name), name, description: describe(&[*f]), technology: lang_name(n.lang).into(), responsibilities: vec![], files: vec![n.path.clone()] });
        }
        return out;
    }
    for (dir, ids) in groups {
        let mut name = if dir.is_empty() { "Core".to_string() } else { title_case(&dir) };
        let mut i = 2;
        while !used.insert(name.clone()) {
            name = format!("{} {i}", if dir.is_empty() { "Core".to_string() } else { title_case(&dir) });
            i += 1;
        }
        let lang = g.node(ids[0]).lang;
        out.push(Component { id: component_id(cid, &name), name, description: describe(&ids), technology: lang_name(lang).into(), responsibilities: vec![], files: ids.iter().map(|f| g.node(*f).path.clone()).collect() });
    }
    out
}

/// The atlas the engine writes from the graph alone.
pub fn engine_atlas(g: &Graph) -> Atlas {
    let f = facts(g);
    let mut containers: Vec<Container> = Vec::new();
    for (pkg, (pid, files)) in &f.packages {
        let tags = f.tags.get(pkg).cloned().unwrap_or_default();
        let kind = kind_of(g, files, &tags);
        let lang = files.first().map(|x| g.node(*x).lang).unwrap_or(Lang::Other);
        let deps = f.deps.get(pkg).cloned().unwrap_or_default();
        let cid = container_id(pkg);
        let pkg_dir = g.node(*pid).path.clone();
        let components = engine_components(g, &cid, &pkg_dir, files);
        let loc: u32 = files.iter().map(|x| g.node(*x).loc).sum();
        let description = if files.is_empty() {
            format!("The `{pkg}` package has no source files of its own.")
        } else {
            format!(
                "{} {} of {} in `{}`{}.",
                files.len(),
                if files.len() == 1 { "file" } else { "files" },
                if lang_name(lang).is_empty() { "code" } else { lang_name(lang) },
                if pkg_dir.is_empty() { "the repository root" } else { &pkg_dir },
                boundary_phrase(&tags)
            )
        };
        containers.push(Container {
            id: cid,
            package: pkg.clone(),
            name: title_case(pkg),
            kind: kind.into(),
            language: main_language(g, files),
            technology: technology_of(lang, &deps),
            description,
            responsibilities: vec![],
            hidden: kind == "tooling" || loc == 0,
            components,
        });
    }
    let mut atlas = Atlas {
        schema: ATLAS_SCHEMA,
        source: "engine".into(),
        model: None,
        scanned_at: g.scanned_at.clone(),
        system: System {
            name: title_case(base(&g.root)),
            purpose: format!("{} files in {} languages, scanned as {} packages.", g.stats.files, g.stats.by_lang.len(), f.packages.len()),
            summary: engine_summary(g, &containers),
        },
        people: vec![],
        externals: vec![],
        containers,
        relationships: vec![],
        journeys: vec![],
        guide: Guide::default(),
        report: Report::default(),
    };
    // A person, when there is something a person would use.
    if atlas.containers.iter().any(|c| matches!(c.kind.as_str(), "web" | "desktop" | "cli")) {
        atlas.people.push(Person { id: "p:user".into(), name: "User".into(), description: "Someone using the software.".into() });
    }
    verify_into(g, &f, &mut atlas, None);
    atlas.journeys = engine_journeys(g, &atlas, ENGINE_JOURNEYS);
    atlas.guide = engine_guide(g, &atlas);
    atlas
}

fn boundary_phrase(tags: &[String]) -> String {
    let has = |t: &str| tags.iter().any(|x| x == t);
    let mut bits = vec![];
    if has("http-server") {
        bits.push("serves HTTP routes");
    }
    if has("ipc-server") {
        bits.push("answers IPC commands");
    }
    if has("http-client") {
        bits.push("calls HTTP APIs");
    }
    if has("db") {
        bits.push("reads a database");
    }
    if has("queue") {
        bits.push("uses a queue");
    }
    if has("fs") {
        bits.push("touches the file system");
    }
    if bits.is_empty() { String::new() } else { format!("; {}", bits.join(", ")) }
}

fn engine_summary(g: &Graph, containers: &[Container]) -> String {
    let shown: Vec<String> = containers.iter().filter(|c| !c.hidden).map(|c| format!("{} ({})", c.name, kind_label(&c.kind).to_lowercase())).collect();
    format!(
        "The engine drew this atlas from the code alone: {} containers ({}) and {} places where data crosses between languages. Names come from folders and manifests. Discover with Claude to have agents read the code and say what each part is for.",
        shown.len(),
        shown.join(", "),
        g.stats.flows
    )
}

/// The engine's journeys: the longest traces, with captions made from the graph.
pub fn engine_journeys(g: &Graph, atlas: &Atlas, limit: usize) -> Vec<Journey> {
    let traces = query::traces(g);
    traces.iter().take(limit).filter_map(|t| journey_from_trace(atlas, t, None)).collect()
}

/// Turn a trace into journey steps over the atlas's components. Each step is a
/// boundary crossing (a flow), plus the first call from the entry and each sink.
pub fn journey_from_trace(atlas: &Atlas, t: &query::Trace, words: Option<(&str, &str, &HashMap<usize, String>)>) -> Option<Journey> {
    let comp_of = component_index(atlas);
    let elem = |path: &str| -> Option<String> {
        let file = path.split('#').next().unwrap_or(path);
        comp_of.get(file).cloned()
    };
    let mut steps: Vec<JourneyStep> = Vec::new();
    let mut seen: HashSet<(String, String, String)> = HashSet::new();
    for (i, s) in t.steps.iter().enumerate() {
        let Some(parent) = s.parent else { continue };
        let p = &t.steps[parent];
        let crossing = s.via == Some(EdgeKind::Flow);
        let (Some(from), Some(to)) = (elem(&p.path), elem(&s.path)) else { continue };
        // A call inside one component is not a step; a crossing or a change of component is.
        if (crossing || from != to) && seen.insert((from.clone(), to.clone(), if crossing { s.label.clone().unwrap_or_default() } else { String::new() })) {
            let label = if crossing { s.label.clone().unwrap_or_else(|| "flow".into()) } else { "calls".into() };
            let caption = words.and_then(|(_, _, caps)| caps.get(&i).cloned()).unwrap_or_else(|| engine_caption(p, s, crossing));
            steps.push(JourneyStep { from: from.clone(), to: to.clone(), from_path: p.path.clone(), to_path: s.path.clone(), label, caption });
        }
        // A sink is where the data comes to rest: point at the external it touches, once.
        if !s.sinks.is_empty()
            && let Some(x) = sink_external(atlas, &s.sinks)
            && seen.insert((to.clone(), x.id.clone(), "sink".into()))
        {
            let caption = words.and_then(|(_, _, caps)| caps.get(&i).cloned()).filter(|_| from == to).unwrap_or_else(|| format!("{} {} {}.", s.name, sink_verb(&s.sinks), x.name));
            steps.push(JourneyStep { from: to.clone(), to: x.id.clone(), from_path: s.path.clone(), to_path: x.name.clone(), label: s.sinks.join(", "), caption });
        }
    }
    if steps.is_empty() {
        return None;
    }
    let (name, summary) = match words {
        Some((n, s, _)) => (n.to_string(), s.to_string()),
        None => (
            format!("From {}", t.name),
            format!("Starts at {} and crosses {} {} through {}.", t.entry_path, t.hops, if t.hops == 1 { "boundary" } else { "boundaries" }, t.lanes.join(", ")),
        ),
    };
    Some(Journey { id: format!("j:{}", slug(&t.entry_path)), name, summary, entry: t.entry_path.clone(), steps })
}

fn sink_external<'a>(atlas: &'a Atlas, sinks: &[String]) -> Option<&'a External> {
    for s in sinks {
        let kind = match s.as_str() {
            "db" => "database",
            "queue" => "queue",
            "fs" => "filesystem",
            "process" => "process",
            _ => continue,
        };
        if let Some(x) = atlas.externals.iter().find(|x| x.kind == kind) {
            return Some(x);
        }
    }
    None
}

fn sink_verb(sinks: &[String]) -> &'static str {
    if sinks.iter().any(|s| s == "db") {
        "reads and writes"
    } else if sinks.iter().any(|s| s == "queue") {
        "puts work on"
    } else {
        "touches"
    }
}

fn engine_caption(p: &query::TraceStep, s: &query::TraceStep, crossing: bool) -> String {
    if crossing {
        format!("{} in {} reaches {} in {} over {}.", p.name, base(p.path.split('#').next().unwrap_or(&p.path)), s.name, base(s.path.split('#').next().unwrap_or(&s.path)), s.label.clone().unwrap_or_default())
    } else {
        format!("{} calls {}.", p.name, s.name)
    }
}

fn engine_guide(g: &Graph, atlas: &Atlas) -> Guide {
    let mut start_here = Vec::new();
    for kind in ["web", "desktop", "cli", "service", "worker"] {
        if let Some(c) = atlas.containers.iter().find(|c| c.kind == kind && !c.hidden) {
            start_here.push(Pointer { element: c.id.clone(), why: format!("The {} is where a user's action enters the system.", kind_label(kind).to_lowercase()) });
            break;
        }
    }
    if let Some(j) = atlas.journeys.first() {
        start_here.push(Pointer { element: j.id.clone(), why: "The longest journey: it crosses the most boundaries, so it shows the most of the system at once.".into() });
    }
    let mut callouts = Vec::new();
    for e in query::endpoints(g).into_iter().filter(|e| e.status != "ok").take(4) {
        let (title, detail) = if e.status == "no-handler" {
            (format!("{} is called but nothing serves it", e.key), format!("Called from {}. Either it is served outside this repository, or it is a gap.", e.callers.iter().map(|r| r.path.clone()).collect::<Vec<_>>().join(", ")))
        } else {
            (format!("{} is served but nothing calls it", e.key), format!("Handled in {}. It may be for an outside caller, or unused.", e.handlers.iter().map(|r| r.path.clone()).collect::<Vec<_>>().join(", ")))
        };
        let element = e.callers.first().or(e.handlers.first()).and_then(|r| component_index(atlas).get(r.path.split('#').next().unwrap_or(&r.path)).cloned());
        callouts.push(Callout { title, detail, element });
    }
    for cy in query::cycles(g, NodeKind::File).into_iter().take(2) {
        let names: Vec<String> = cy.iter().map(|id| g.node(*id).path.clone()).collect();
        callouts.push(Callout { title: format!("{} files depend on each other", cy.len()), detail: format!("{} form a cycle: none can be understood alone.", names.join(", ")), element: component_index(atlas).get(&names[0]).cloned() });
    }
    for h in query::hotspots(g, NodeKind::File, 1) {
        if h.fan_in + h.fan_out >= 3 {
            callouts.push(Callout { title: format!("{} is the most connected file", base(&h.path)), detail: format!("{} files rest on it and it rests on {}. Changes here reach far.", h.fan_in, h.fan_out), element: component_index(atlas).get(&h.path).cloned() });
        }
    }
    Guide { start_here, callouts }
}

/// file path -> component id
pub fn component_index(atlas: &Atlas) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for c in &atlas.containers {
        for k in &c.components {
            for f in &k.files {
                m.insert(f.clone(), k.id.clone());
            }
        }
    }
    m
}

fn container_of_component(id: &str) -> &str {
    id.rsplit_once('/').map(|(c, _)| c).unwrap_or(id)
}

// ---- verify --------------------------------------------------------------------------

/// Words an agent attached to a relationship it found, keyed by (from, to) element ids.
pub type Words = HashMap<(String, String), (String, String)>;

/// Check an atlas against the graph and repair it in place. `claims` are
/// relationships an agent asserted, keyed by element ids, with (label, technology).
pub fn verify(g: &Graph, atlas: &Atlas, claims: Option<&Words>) -> Atlas {
    let f = facts(g);
    let mut a = atlas.clone();
    verify_into(g, &f, &mut a, claims);
    a
}

fn verify_into(g: &Graph, f: &Facts, a: &mut Atlas, claims: Option<&Words>) {
    let mut notes: Vec<String> = Vec::new();
    let mut unplaced: Vec<String> = Vec::new();
    // Every package is a container, and only packages are.
    let mut seen_pkg: HashSet<String> = HashSet::new();
    a.containers.retain(|c| {
        let keep = f.packages.contains_key(&c.package) && seen_pkg.insert(c.package.clone());
        if !keep {
            notes.push(format!("Dropped container `{}`: no package `{}` in the scan.", c.name, c.package));
        }
        keep
    });
    for (pkg, (pid, files)) in &f.packages {
        if seen_pkg.contains(pkg) {
            continue;
        }
        let tags = f.tags.get(pkg).cloned().unwrap_or_default();
        let kind = kind_of(g, files, &tags);
        let lang = files.first().map(|x| g.node(*x).lang).unwrap_or(Lang::Other);
        let cid = container_id(pkg);
        let pkg_dir = g.node(*pid).path.clone();
        a.containers.push(Container {
            id: cid.clone(),
            package: pkg.clone(),
            name: title_case(pkg),
            kind: kind.into(),
            language: main_language(g, files),
            technology: technology_of(lang, &f.deps.get(pkg).cloned().unwrap_or_default()),
            description: format!("{} files the agents did not describe.", files.len()),
            responsibilities: vec![],
            hidden: files.is_empty(),
            components: engine_components(g, &cid, &pkg_dir, files),
        });
        if !files.is_empty() {
            notes.push(format!("Added container `{}` from the engine: the agents left it out.", pkg));
        }
    }
    // Components hold only their container's files, and all of them.
    for c in &mut a.containers {
        c.id = container_id(&c.package);
        let (_, files) = &f.packages[&c.package];
        c.language = main_language(g, files);
        let own: HashSet<&str> = files.iter().map(|id| g.node(*id).path.as_str()).collect();
        let mut placed: HashSet<String> = HashSet::new();
        let mut used_ids: HashSet<String> = HashSet::new();
        for k in &mut c.components {
            let mut id = component_id(&c.id, &k.name);
            let mut i = 2;
            while !used_ids.insert(id.clone()) {
                id = format!("{}-{i}", component_id(&c.id, &k.name));
                i += 1;
            }
            k.id = id;
            let before = k.files.len();
            k.files.retain(|p| own.contains(p.as_str()) && placed.insert(p.clone()));
            if k.files.len() < before {
                notes.push(format!("`{}`: dropped {} file(s) that are not in `{}` or were placed twice.", k.name, before - k.files.len(), c.package));
            }
        }
        c.components.retain(|k| !k.files.is_empty());
        let missing: Vec<String> = files.iter().map(|id| g.node(*id).path.clone()).filter(|p| !placed.contains(p)).collect();
        if !missing.is_empty() {
            if c.components.is_empty() {
                let (_, pid_files) = &f.packages[&c.package];
                c.components = engine_components(g, &c.id, &g.node(f.packages[&c.package].0).path, pid_files);
            } else {
                unplaced.extend(missing.iter().cloned());
                let id = component_id(&c.id, "Other files");
                c.components.push(Component { id, name: "Other files".into(), description: format!("{} files the agent did not place.", missing.len()), technology: String::new(), responsibilities: vec![], files: missing });
            }
        }
        if c.kind == "tooling" || files.is_empty() {
            c.hidden = true;
        }
    }
    // Relationships: the code decides which exist; agents may only add words, or claims.
    let comp_of = component_index(a);
    let mut backed: BTreeMap<(String, String), Relationship> = BTreeMap::new();
    for e in &f.file_edges {
        let (fa, fb) = (&g.node(e.from).path, &g.node(e.to).path);
        let (Some(ka), Some(kb)) = (comp_of.get(fa), comp_of.get(fb)) else { continue };
        let (ca, cb) = (container_of_component(ka).to_string(), container_of_component(kb).to_string());
        if ka != kb {
            let r = backed.entry((ka.clone(), kb.clone())).or_insert_with(|| Relationship { from: ka.clone(), to: kb.clone(), label: String::new(), technology: String::new(), source: "code".into(), level: "component".into(), evidence: vec![] });
            push_evidence(r, &e.evidence);
        }
        if ca != cb {
            let r = backed.entry((ca.clone(), cb.clone())).or_insert_with(|| Relationship { from: ca.clone(), to: cb.clone(), label: String::new(), technology: String::new(), source: "code".into(), level: "container".into(), evidence: vec![] });
            push_evidence(r, &e.evidence);
        }
    }
    // Externals the code shows: databases, queues, file systems, and HTTP calls nothing here serves.
    // Evidence is per file, so the component that touches the database gets its own arrow.
    let mut file_tags: HashMap<String, Vec<String>> = HashMap::new();
    for n in &g.nodes {
        if n.external || n.tags.is_empty() {
            continue;
        }
        let Some(fid) = file_of(g, n.id) else { continue };
        let v = file_tags.entry(g.node(fid).path.clone()).or_default();
        for t in &n.tags {
            if !v.contains(t) {
                v.push(t.clone());
            }
        }
    }
    let mut externals: Vec<External> = a.externals.clone();
    let gaps: Vec<query::Endpoint> = query::endpoints(g).into_iter().filter(|e| e.status == "no-handler").collect();
    for c in &a.containers {
        let (_, files) = &f.packages[&c.package];
        let paths: Vec<&str> = files.iter().map(|id| g.node(*id).path.as_str()).collect();
        let with_tag = |t: &str| -> Vec<(String, String)> { paths.iter().filter(|p| file_tags.get(**p).is_some_and(|v| v.iter().any(|x| x == t))).map(|p| (p.to_string(), t.to_string())).collect() };
        let env_named = |pred: &dyn Fn(&str) -> bool| -> Option<String> { paths.iter().flat_map(|p| file_tags.get(*p).into_iter().flatten()).filter_map(|t| t.strip_prefix("env:")).find(|e| pred(e)).map(String::from) };
        // (kind, name, verb, technology, files touching it)
        let mut touches: Vec<(String, String, String, String, Vec<(String, String)>)> = vec![];
        let db = with_tag("db");
        if !db.is_empty() {
            let name = env_named(&|e| e.contains("DATABASE") || e.contains("DB") || e.contains("PG") || e.contains("SQL")).map(|e| format!("Database ({e})")).unwrap_or_else(|| "Database".into());
            touches.push(("database".into(), name, "reads and writes".into(), "SQL".into(), db));
        }
        let queue = with_tag("queue");
        if !queue.is_empty() {
            let name = env_named(&|e| e.contains("REDIS") || e.contains("QUEUE") || e.contains("BROKER") || e.contains("AMQP") || e.contains("KAFKA")).map(|e| format!("Queue ({e})")).unwrap_or_else(|| "Queue".into());
            touches.push(("queue".into(), name, "puts work on".into(), "queue".into(), queue));
        }
        let fs = with_tag("fs");
        if !fs.is_empty() {
            touches.push(("filesystem".into(), "File system".into(), "reads and writes files on".into(), String::new(), fs));
        }
        for gap in &gaps {
            let callers: Vec<(String, String)> = gap
                .callers
                .iter()
                .filter_map(|r| file_of(g, r.id))
                .filter(|fid| f.package_of.get(fid).is_some_and(|p| p == &c.package))
                .map(|fid| (g.node(fid).path.clone(), gap.key.clone()))
                .collect();
            if !callers.is_empty() {
                touches.push(("service".into(), "Outside HTTP API".into(), format!("calls {}", gap.key.trim_start_matches("http ")), "HTTP".into(), callers));
            }
        }
        for (kind, name, verb, tech, touching) in touches {
            let x = match externals.iter().find(|x| x.name == name || (x.kind == kind && kind != "service")) {
                Some(x) => x.clone(),
                None => {
                    let x = External { id: format!("x:{}", slug(&name)), name: name.clone(), kind: kind.clone(), description: external_blurb(&kind) };
                    externals.push(x.clone());
                    x
                }
            };
            // container level
            let r = backed.entry((c.id.clone(), x.id.clone())).or_insert_with(|| Relationship { from: c.id.clone(), to: x.id.clone(), label: verb.clone(), technology: tech.clone(), source: "code".into(), level: "container".into(), evidence: vec![] });
            if r.label.is_empty() {
                r.label = verb.clone();
            }
            if kind == "service" && r.label != verb && !r.label.contains(&verb) {
                r.label = format!("{}, {}", r.label, verb);
            }
            for (path, tag) in &touching {
                push_evidence(r, &Evidence { from: path.clone(), to: x.name.clone(), via: "tag".into(), label: Some(tag.clone()), line: None });
            }
            // component level: whichever components hold the touching files
            for k in &c.components {
                let mine: Vec<&(String, String)> = touching.iter().filter(|(p, _)| k.files.contains(p)).collect();
                if mine.is_empty() {
                    continue;
                }
                let r = backed.entry((k.id.clone(), x.id.clone())).or_insert_with(|| Relationship { from: k.id.clone(), to: x.id.clone(), label: verb.clone(), technology: tech.clone(), source: "code".into(), level: "component".into(), evidence: vec![] });
                for (path, tag) in mine {
                    push_evidence(r, &Evidence { from: path.clone(), to: x.name.clone(), via: "tag".into(), label: Some(tag.clone()), line: None });
                }
            }
        }
    }
    let mut seen_x = HashSet::new();
    externals.retain(|x| seen_x.insert(x.id.clone()));
    a.externals = externals;
    // Engine labels for backed edges without words.
    for r in backed.values_mut() {
        if r.label.is_empty() {
            r.label = engine_label(r);
        }
        if r.technology.is_empty() && r.level == "container" {
            r.technology = tech_of_evidence(&r.evidence);
        }
    }
    // Agents' words: attach to backed relationships; everything else is survey or a claim.
    let ids: HashSet<String> = a
        .containers
        .iter()
        .flat_map(|c| std::iter::once(c.id.clone()).chain(c.components.iter().map(|k| k.id.clone())))
        .chain(a.people.iter().map(|p| p.id.clone()))
        .chain(a.externals.iter().map(|x| x.id.clone()))
        .collect();
    let mut extra: Vec<Relationship> = Vec::new();
    let mut kept_old: Vec<Relationship> = Vec::new();
    for r in a.relationships.drain(..) {
        if r.source == "survey" && ids.contains(&r.from) && ids.contains(&r.to) {
            kept_old.push(r);
        }
    }
    if let Some(claims) = claims {
        let mut sorted: Vec<(&(String, String), &(String, String))> = claims.iter().collect();
        sorted.sort();
        for ((from, to), (label, tech)) in sorted {
            if let Some(r) = backed.get_mut(&(from.clone(), to.clone())) {
                if !label.trim().is_empty() {
                    r.label = label.trim().to_string();
                }
                if !tech.trim().is_empty() {
                    r.technology = tech.trim().to_string();
                }
                continue;
            }
            if !ids.contains(from) || !ids.contains(to) {
                notes.push(format!("Dropped a relationship from `{from}` to `{to}`: no such element."));
                continue;
            }
            let is_person = from.starts_with("p:");
            let is_external = to.starts_with("x:") || from.starts_with("x:");
            let level = if from.contains('/') || to.contains('/') { "component" } else { "container" };
            extra.push(Relationship { from: from.clone(), to: to.clone(), label: label.trim().to_string(), technology: tech.trim().to_string(), source: if is_person || is_external { "survey".into() } else { "claimed".into() }, level: level.into(), evidence: vec![] });
        }
    }
    let mut rels: Vec<Relationship> = backed.into_values().collect();
    for r in kept_old.into_iter().chain(extra) {
        if !rels.iter().any(|x| x.from == r.from && x.to == r.to) {
            rels.push(r);
        }
    }
    // A person with nothing to use gets the engine's guess: whatever a person can run.
    for p in &a.people {
        if !rels.iter().any(|r| r.from == p.id) {
            for c in a.containers.iter().filter(|c| !c.hidden && matches!(c.kind.as_str(), "web" | "desktop" | "cli")) {
                rels.push(Relationship { from: p.id.clone(), to: c.id.clone(), label: "uses".into(), technology: String::new(), source: "survey".into(), level: "container".into(), evidence: vec![] });
            }
        }
    }
    rels.sort_by(|x, y| x.level.cmp(&y.level).then_with(|| x.from.cmp(&y.from)).then_with(|| x.to.cmp(&y.to)));
    a.relationships = rels;
    // Journeys and the guide must point at things that exist.
    let comp_of = component_index(a);
    for j in &mut a.journeys {
        j.steps.retain(|s| ids.contains(&s.from) && ids.contains(&s.to));
    }
    a.journeys.retain(|j| !j.steps.is_empty());
    a.guide.start_here.retain(|p| ids.contains(&p.element) || a.journeys.iter().any(|j| j.id == p.element));
    for c in &mut a.guide.callouts {
        if let Some(e) = &c.element
            && !ids.contains(e)
        {
            c.element = comp_of.get(e).cloned();
        }
    }
    let _ = &comp_of;
    a.report = Report {
        backed: a.relationships.iter().filter(|r| r.source == "code").count() as u32,
        survey: a.relationships.iter().filter(|r| r.source == "survey").count() as u32,
        claimed: a.relationships.iter().filter(|r| r.source == "claimed").count() as u32,
        unplaced,
        notes,
    };
}

fn push_evidence(r: &mut Relationship, e: &Evidence) {
    if r.evidence.len() < 12 && !r.evidence.iter().any(|x| x.from == e.from && x.to == e.to && x.via == e.via) {
        r.evidence.push(e.clone());
    }
}

fn external_blurb(kind: &str) -> String {
    match kind {
        "database" => "Where the data lives.",
        "queue" => "Work handed off to be done later.",
        "filesystem" => "Files on the machine.",
        "service" => "An HTTP API nothing in this repository serves.",
        _ => "Outside this repository.",
    }
    .into()
}

fn engine_label(r: &Relationship) -> String {
    let flows: Vec<String> = r.evidence.iter().filter(|e| e.via == "flow").filter_map(|e| e.label.clone()).collect();
    if !flows.is_empty() {
        let mut uniq: Vec<String> = Vec::new();
        for f in flows {
            if !uniq.contains(&f) {
                uniq.push(f);
            }
        }
        let shown: Vec<String> = uniq.iter().take(3).cloned().collect();
        return format!("{}{}", shown.join(", "), if uniq.len() > 3 { format!(" and {} more", uniq.len() - 3) } else { String::new() });
    }
    let calls = r.evidence.iter().filter(|e| e.via == "calls").count();
    let imports = r.evidence.iter().filter(|e| e.via == "imports").count();
    match (calls, imports) {
        (0, _) => "imports".into(),
        (_, 0) => "calls".into(),
        _ => "imports and calls".into(),
    }
}

fn tech_of_evidence(ev: &[Evidence]) -> String {
    let mut kinds: Vec<&str> = Vec::new();
    for e in ev {
        let k = match e.label.as_deref().and_then(|l| l.split(' ').next()) {
            Some("http") => "HTTP",
            Some("ipc") => "IPC",
            Some("queue") => "queue",
            _ => continue,
        };
        if !kinds.contains(&k) {
            kinds.push(k);
        }
    }
    kinds.join(", ")
}

/// Everything about one container's code the discovery agents need to read it well.
pub fn container_brief(g: &Graph, f: &Facts, c: &Container) -> String {
    let (_, files) = &f.packages[&c.package];
    let mut out = String::new();
    for (i, id) in files.iter().enumerate() {
        let n = g.node(*id);
        let syms: Vec<String> = g.children(*id).map(|s| s.name.clone()).take(14).collect();
        let tags: Vec<&str> = n.tags.iter().map(|t| t.as_str()).chain(g.children(*id).flat_map(|s| s.tags.iter().map(|t| t.as_str()))).collect();
        let mut uniq: Vec<&str> = Vec::new();
        for t in tags {
            if !uniq.contains(&t) {
                uniq.push(t);
            }
        }
        out.push_str(&format!("{}. {} ({} lines)\n", i + 1, n.path, n.loc));
        if !syms.is_empty() {
            out.push_str(&format!("   parts: {}\n", syms.join(", ")));
        }
        if !uniq.is_empty() {
            out.push_str(&format!("   touches: {}\n", uniq.join(" ")));
        }
    }
    let mut inside: Vec<String> = Vec::new();
    let mut outward: Vec<String> = Vec::new();
    let mut inward: Vec<String> = Vec::new();
    for e in &f.file_edges {
        let (pa, pb) = (&f.package_of[&e.from], &f.package_of[&e.to]);
        let line = format!("{} {} {}{}", g.node(e.from).path, e.evidence.via, g.node(e.to).path, e.evidence.label.as_ref().map(|l| format!(" ({l})")).unwrap_or_default());
        if pa == &c.package && pb == &c.package {
            if !inside.contains(&line) {
                inside.push(line);
            }
        } else if pa == &c.package {
            if !outward.contains(&line) {
                outward.push(line);
            }
        } else if pb == &c.package && !inward.contains(&line) {
            inward.push(line);
        }
    }
    let list = |v: &[String]| if v.is_empty() { "none\n".to_string() } else { v.iter().take(60).map(|l| format!("- {l}\n")).collect() };
    out.push_str(&format!("\nDependencies inside this container:\n{}", list(&inside)));
    out.push_str(&format!("\nWhat it reaches in other containers:\n{}", list(&outward)));
    out.push_str(&format!("\nWhat reaches it from other containers:\n{}", list(&inward)));
    out
}

// ---- export ---------------------------------------------------------------------------

fn dsl_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn dsl_id(id: &str) -> String {
    let mut out = String::new();
    for c in id.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// The atlas as Structurizr DSL, so it can be checked into the repository or
/// opened in any C4 tool.
pub fn to_dsl(a: &Atlas) -> String {
    let mut s = String::new();
    s.push_str(&format!("workspace {} {} {{\n\n", dsl_str(&a.system.name), dsl_str(&a.system.purpose)));
    s.push_str("    model {\n");
    for p in &a.people {
        s.push_str(&format!("        {} = person {} {}\n", dsl_id(&p.id), dsl_str(&p.name), dsl_str(&p.description)));
    }
    s.push_str(&format!("        {} = softwareSystem {} {} {{\n", dsl_id("s"), dsl_str(&a.system.name), dsl_str(&a.system.purpose)));
    for c in a.containers.iter().filter(|c| !c.hidden) {
        s.push_str(&format!("            {} = container {} {} {} {{\n", dsl_id(&c.id), dsl_str(&c.name), dsl_str(&c.description), dsl_str(&c.technology)));
        for k in &c.components {
            s.push_str(&format!("                {} = component {} {} {}\n", dsl_id(&k.id), dsl_str(&k.name), dsl_str(&k.description), dsl_str(&k.technology)));
        }
        s.push_str("            }\n");
    }
    s.push_str("        }\n");
    for x in &a.externals {
        s.push_str(&format!("        {} = softwareSystem {} {} {{\n            tags {}\n        }}\n", dsl_id(&x.id), dsl_str(&x.name), dsl_str(&x.description), dsl_str(&x.kind)));
    }
    s.push('\n');
    for r in &a.relationships {
        s.push_str(&format!("        {} -> {} {} {}{}\n", dsl_id(&r.from), dsl_id(&r.to), dsl_str(&r.label), dsl_str(&r.technology), if r.source == "claimed" { " \"claimed\"" } else { "" }));
    }
    s.push_str("    }\n\n    views {\n");
    s.push_str("        systemContext s {\n            include *\n            autolayout tb\n        }\n");
    s.push_str("        container s {\n            include *\n            autolayout tb\n        }\n");
    for c in a.containers.iter().filter(|c| !c.hidden) {
        s.push_str(&format!("        component {} {{\n            include *\n            autolayout tb\n        }}\n", dsl_id(&c.id)));
    }
    for j in &a.journeys {
        s.push_str(&format!("        dynamic s {} {} {{\n", dsl_id(&j.id), dsl_str(&j.name)));
        for st in &j.steps {
            s.push_str(&format!("            {} -> {} {}\n", dsl_id(container_of_component(&st.from)), dsl_id(container_of_component(&st.to)), dsl_str(&st.caption)));
        }
        s.push_str("            autolayout tb\n        }\n");
    }
    s.push_str("        styles {\n            element \"Person\" { shape person }\n            element \"database\" { shape cylinder }\n            element \"queue\" { shape pipe }\n        }\n    }\n}\n");
    s
}

/// The atlas for a graph: the saved agent atlas if there is one (checked against
/// the current scan), otherwise the engine's. The second value says why a saved
/// atlas was made for an older scan.
pub fn for_graph(g: &Graph, saved: Option<Atlas>) -> (Atlas, Option<String>) {
    match saved {
        Some(a) => {
            let stale = (a.scanned_at != g.scanned_at).then(|| format!("{} atlas made for an earlier scan ({}); checked against this one", a.source, a.scanned_at));
            let mut v = verify(g, &a, None);
            if v.journeys.is_empty() {
                v.journeys = engine_journeys(g, &v, ENGINE_JOURNEYS);
            }
            (v, stale)
        }
        None => (engine_atlas(g), None),
    }
}
