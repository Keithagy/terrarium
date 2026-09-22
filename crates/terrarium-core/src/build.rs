//! The build: the repository as a brick model with a step-by-step manual.
//!
//! Packages are districts on a baseplate, files are buildings, and a file's
//! symbols are its bricks, stacked in source order. Files go in dependency order,
//! so each step only adds pieces that rest on pieces already built. The manual is
//! then a reading order for the codebase.
//!
//! A [`Design`] says how the pieces are grouped and explained: sub-builds, steps,
//! titles and captions. The engine writes a plain one ([`engine_design`]); an agent
//! can write a richer one (see `designer`). Whoever wrote it, [`check`] verifies
//! every joint against the graph and [`repair`] moves any piece placed before
//! something it rests on. So a design can be as imaginative as it likes and still
//! hold together.

use crate::model::*;
use crate::query;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

pub const DESIGN_SCHEMA: u32 = 1;
/// Most files one engine step adds; cycles that must go in together may exceed it.
const STEP_FILES: usize = 3;
/// A building is at most this many bricks tall; extra symbols share bricks.
const MAX_LAYERS: usize = 14;

// ---- design ------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Design {
    pub schema: u32,
    /// `engine`, or the agent that wrote it (`claude`).
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The scan this design was made for; a newer scan makes it stale.
    pub scanned_at: String,
    pub title: String,
    pub summary: String,
    pub sub_builds: Vec<SubBuild>,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubBuild {
    pub id: String,
    pub name: String,
    pub blurb: String,
    /// The package this sub-build stands for.
    pub package: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Step {
    pub sub_build: String,
    pub title: String,
    pub caption: String,
    /// Repo-relative file paths added in this step.
    pub files: Vec<String>,
}

// ---- dependencies --------------------------------------------------------------

/// What the engine knows about the files it is building with.
pub struct Deps {
    /// Internal files, by node id, in path order.
    pub files: Vec<NodeId>,
    /// file -> files it rests on (imports it or calls into it).
    pub on: HashMap<NodeId, Vec<NodeId>>,
    /// Joint count per ordered file pair.
    pub joints: HashMap<(NodeId, NodeId), u32>,
    /// Strongly connected component id per file; files sharing one are interlocked.
    pub scc: HashMap<NodeId, usize>,
    pub scc_size: Vec<usize>,
    pub package: HashMap<NodeId, String>,
}

fn file_of(g: &Graph, id: NodeId) -> Option<NodeId> {
    g.ancestor_of_kind(id, NodeKind::File)
}

pub fn deps(g: &Graph) -> Deps {
    let mut files: Vec<NodeId> = g
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::File && !n.external)
        .map(|n| n.id)
        .collect();
    files.sort_by(|a, b| g.node(*a).path.cmp(&g.node(*b).path));
    let known: HashSet<NodeId> = files.iter().copied().collect();
    let mut joints: HashMap<(NodeId, NodeId), u32> = HashMap::new();
    for e in &g.edges {
        if e.kind != EdgeKind::Imports && e.kind != EdgeKind::Calls {
            continue;
        }
        let (Some(a), Some(b)) = (file_of(g, e.from), file_of(g, e.to)) else { continue };
        if a == b || !known.contains(&a) || !known.contains(&b) {
            continue;
        }
        *joints.entry((a, b)).or_default() += 1;
    }
    let mut on: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for &(a, b) in joints.keys() {
        on.entry(a).or_default().push(b);
    }
    for v in on.values_mut() {
        v.sort_by(|x, y| g.node(*x).path.cmp(&g.node(*y).path));
    }
    let (scc, scc_size) = components(&files, &on);
    let package = files
        .iter()
        .map(|&f| {
            let p = g
                .ancestor_of_kind(f, NodeKind::Package)
                .map(|p| g.node(p).name.clone())
                .unwrap_or_else(|| "root".to_string());
            (f, p)
        })
        .collect();
    Deps { files, on, joints, scc, scc_size, package }
}

/// Tarjan over the file dependency graph.
fn components(files: &[NodeId], on: &HashMap<NodeId, Vec<NodeId>>) -> (HashMap<NodeId, usize>, Vec<usize>) {
    struct T<'a> {
        on: &'a HashMap<NodeId, Vec<NodeId>>,
        index: HashMap<NodeId, usize>,
        low: HashMap<NodeId, usize>,
        stack: Vec<NodeId>,
        on_stack: HashSet<NodeId>,
        next: usize,
        comp: HashMap<NodeId, usize>,
        sizes: Vec<usize>,
    }
    fn visit(t: &mut T, v: NodeId) {
        t.index.insert(v, t.next);
        t.low.insert(v, t.next);
        t.next += 1;
        t.stack.push(v);
        t.on_stack.insert(v);
        let succ = t.on.get(&v).cloned().unwrap_or_default();
        for w in succ {
            if !t.index.contains_key(&w) {
                visit(t, w);
                let lw = t.low[&w];
                let lv = t.low.get_mut(&v).unwrap();
                *lv = (*lv).min(lw);
            } else if t.on_stack.contains(&w) {
                let iw = t.index[&w];
                let lv = t.low.get_mut(&v).unwrap();
                *lv = (*lv).min(iw);
            }
        }
        if t.low[&v] == t.index[&v] {
            let id = t.sizes.len();
            let mut size = 0;
            loop {
                let w = t.stack.pop().unwrap();
                t.on_stack.remove(&w);
                t.comp.insert(w, id);
                size += 1;
                if w == v {
                    break;
                }
            }
            t.sizes.push(size);
        }
    }
    let mut t = T {
        on,
        index: HashMap::new(),
        low: HashMap::new(),
        stack: vec![],
        on_stack: HashSet::new(),
        next: 0,
        comp: HashMap::new(),
        sizes: vec![],
    };
    for &f in files {
        if !t.index.contains_key(&f) {
            visit(&mut t, f);
        }
    }
    (t.comp, t.sizes)
}

/// Files in an order where everything a file rests on comes first. Interlocked
/// files come out adjacent. Among files that are ready, stay in the package being
/// built, then go by path, so districts rise one at a time.
pub fn build_order(g: &Graph, d: &Deps) -> Vec<NodeId> {
    let n = d.scc_size.len();
    let mut members: Vec<Vec<NodeId>> = vec![vec![]; n];
    for &f in &d.files {
        members[d.scc[&f]].push(f);
    }
    // component -> components it rests on
    let mut needs: Vec<HashSet<usize>> = vec![HashSet::new(); n];
    let mut feeds: Vec<HashSet<usize>> = vec![HashSet::new(); n];
    for (&a, bs) in &d.on {
        for &b in bs {
            let (ca, cb) = (d.scc[&a], d.scc[&b]);
            if ca != cb {
                needs[ca].insert(cb);
                feeds[cb].insert(ca);
            }
        }
    }
    let key = |c: usize| -> (String, String) {
        let f = members[c][0];
        (d.package[&f].clone(), g.node(f).path.clone())
    };
    let mut waiting: Vec<usize> = needs.iter().map(|s| s.len()).collect();
    let mut ready: Vec<usize> = (0..n).filter(|&c| waiting[c] == 0).collect();
    let mut out = Vec::with_capacity(d.files.len());
    let mut current_pkg: Option<String> = None;
    while !ready.is_empty() {
        ready.sort_by_key(|&c| {
            let (pkg, path) = key(c);
            (Some(&pkg) != current_pkg.as_ref(), pkg, path)
        });
        let c = ready.remove(0);
        current_pkg = Some(key(c).0);
        let mut m = members[c].clone();
        m.sort_by(|a, b| g.node(*a).path.cmp(&g.node(*b).path));
        out.extend(m);
        for &x in &feeds[c] {
            waiting[x] -= 1;
            if waiting[x] == 0 {
                ready.push(x);
            }
        }
    }
    out
}

fn base(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn dir(path: &str) -> &str {
    path.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn join_names(names: &[String]) -> String {
    match names.len() {
        0 => String::new(),
        1 => names[0].clone(),
        2 => format!("{} and {}", names[0], names[1]),
        n => format!("{}, and {}", names[..n - 1].join(", "), names[n - 1]),
    }
}

/// Symbols of a file, in source order.
fn symbols_of(g: &Graph, file: NodeId) -> Vec<&Node> {
    let mut s: Vec<&Node> = g
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Symbol && n.parent == Some(file))
        .collect();
    s.sort_by_key(|n| (n.span.map(|x| x.0).unwrap_or(0), n.id));
    s
}

/// The engine's own design: one sub-build per package, steps of a few files from
/// one directory in build order, and captions made from the graph.
pub fn engine_design(g: &Graph) -> Design {
    let d = deps(g);
    let order = build_order(g, &d);
    let mut sub_builds: Vec<SubBuild> = Vec::new();
    let mut seen_pkg: HashSet<String> = HashSet::new();
    for &f in &order {
        let pkg = &d.package[&f];
        if seen_pkg.insert(pkg.clone()) {
            let count = order.iter().filter(|x| &d.package[*x] == pkg).count();
            let lang = g.node(f).lang;
            sub_builds.push(SubBuild {
                id: slug(pkg),
                name: pkg.clone(),
                blurb: format!("{count} {} in {}", if count == 1 { "file" } else { "files" }, lang_name(lang)),
                package: pkg.clone(),
            });
        }
    }
    let mut steps: Vec<Step> = Vec::new();
    let mut cur: Vec<NodeId> = Vec::new();
    let flush = |cur: &mut Vec<NodeId>, steps: &mut Vec<Step>| {
        if cur.is_empty() {
            return;
        }
        let placed: HashSet<NodeId> = steps
            .iter()
            .flat_map(|s| s.files.iter())
            .filter_map(|p| g.find_by_path(p).map(|n| n.id))
            .collect();
        steps.push(engine_step(g, &d, cur, &placed));
        cur.clear();
    };
    let mut i = 0;
    while i < order.len() {
        let f = order[i];
        let comp = d.scc[&f];
        // an interlocked group goes in whole, in its own step
        if d.scc_size[comp] > 1 {
            flush(&mut cur, &mut steps);
            let mut group = vec![];
            while i < order.len() && d.scc[&order[i]] == comp {
                group.push(order[i]);
                i += 1;
            }
            flush(&mut group, &mut steps);
            continue;
        }
        // A file that rests on one in this step waits for the next, so every step
        // only builds on steps before it.
        let rests_on_cur = d.on.get(&f).is_some_and(|v| v.iter().any(|b| cur.contains(b)));
        if let Some(&last) = cur.last()
            && (cur.len() >= STEP_FILES
                || rests_on_cur
                || d.package[&last] != d.package[&f]
                || dir(&g.node(last).path) != dir(&g.node(f).path))
        {
            flush(&mut cur, &mut steps);
        }
        cur.push(f);
        i += 1;
    }
    flush(&mut cur, &mut steps);
    let flows = g.stats.flows;
    Design {
        schema: DESIGN_SCHEMA,
        source: "engine".into(),
        model: None,
        scanned_at: g.scanned_at.clone(),
        title: base(&g.root).to_string(),
        summary: format!(
            "{} files in {} {}, built in dependency order{}.",
            order.len(),
            sub_builds.len(),
            if sub_builds.len() == 1 { "sub-build" } else { "sub-builds" },
            if flows > 0 { format!(", with {flows} bridges where data crosses between languages") } else { String::new() }
        ),
        sub_builds,
        steps,
    }
}

fn lang_name(l: Lang) -> &'static str {
    match l {
        Lang::Rust => "Rust",
        Lang::TypeScript => "TypeScript",
        Lang::JavaScript => "JavaScript",
        Lang::Python => "Python",
        Lang::Go => "Go",
        Lang::Other => "other languages",
    }
}

fn engine_step(g: &Graph, d: &Deps, files: &[NodeId], placed: &HashSet<NodeId>) -> Step {
    let names: Vec<String> = files.iter().map(|f| base(&g.node(*f).path).to_string()).collect();
    let mut parts = Vec::new();
    for &f in files {
        let syms = symbols_of(g, f);
        let mut big: Vec<&&Node> = syms.iter().collect();
        big.sort_by(|a, b| b.loc.cmp(&a.loc).then(a.id.cmp(&b.id)));
        let top: Vec<String> = big.iter().take(3).map(|n| n.name.clone()).collect();
        let mut s = if top.is_empty() {
            format!("{} has no named parts", base(&g.node(f).path))
        } else {
            format!(
                "{} brings {} {} ({}{})",
                base(&g.node(f).path),
                syms.len(),
                if syms.len() == 1 { "part" } else { "parts" },
                top.join(", "),
                if syms.len() > 3 { ", …" } else { "" }
            )
        };
        let rests: Vec<String> = d
            .on
            .get(&f)
            .map(|v| v.iter().filter(|x| placed.contains(x)).map(|x| base(&g.node(*x).path).to_string()).collect())
            .unwrap_or_default();
        if !rests.is_empty() {
            let shown: Vec<String> = rests.iter().take(3).cloned().collect();
            s.push_str(&format!(" and rests on {}", join_names(&shown)));
            if rests.len() > 3 {
                s.push_str(&format!(" and {} more", rests.len() - 3));
            }
        }
        parts.push(s);
    }
    let interlocked = files.len() > 1 && files.iter().all(|f| d.scc[f] == d.scc[&files[0]]) && d.scc_size[d.scc[&files[0]]] > 1;
    let mut caption = format!("{}.", parts.join(". "));
    if interlocked {
        caption.push_str(" These files depend on each other, so they go in together.");
    }
    Step {
        sub_build: slug(&d.package[&files[0]]),
        title: format!("Add {}", join_names(&names)),
        caption,
        files: files.iter().map(|f| g.node(*f).path.clone()).collect(),
    }
}

// ---- check -----------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Weak {
    /// `early` (placed before a piece it rests on), `missing`, `duplicate`,
    /// `unknown` (no such file) or `sub-build` (step names an unknown sub-build).
    pub kind: &'static str,
    pub file: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub ok: bool,
    pub pieces: u32,
    pub files: u32,
    pub steps: u32,
    pub sub_builds: u32,
    /// Import and call edges between files that hold the model together.
    pub joints: u32,
    /// Cross-language flows: bridges between districts.
    pub bridges: u32,
    pub weak: Vec<Weak>,
    /// Groups of files that depend on each other and must go in together.
    pub interlocked: Vec<Vec<String>>,
    /// Files that rest on nothing and that nothing rests on.
    pub loose: Vec<String>,
    /// Calls the scanner could not resolve: studs that did not click.
    pub unresolved: u32,
    /// Endpoints with no caller or no handler inside the repository.
    pub gaps: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub repairs: Vec<String>,
}

/// Verify a design against the graph: every file placed exactly once, and never
/// before a file it rests on (unless the two are interlocked).
pub fn check(g: &Graph, design: &Design) -> Check {
    let d = deps(g);
    let mut weak = Vec::new();
    let subs: HashSet<&str> = design.sub_builds.iter().map(|s| s.id.as_str()).collect();
    let mut pos: HashMap<NodeId, usize> = HashMap::new();
    for (i, s) in design.steps.iter().enumerate() {
        if !subs.contains(s.sub_build.as_str()) {
            weak.push(Weak { kind: "sub-build", file: s.files.first().cloned().unwrap_or_default(), detail: format!("step {} names sub-build `{}`, which the design does not define", i + 1, s.sub_build) });
        }
        for p in &s.files {
            match g.find_by_path(p).filter(|n| n.kind == NodeKind::File && !n.external) {
                None => weak.push(Weak { kind: "unknown", file: p.clone(), detail: format!("step {} adds a file the repository does not have", i + 1) }),
                Some(n) => {
                    if pos.insert(n.id, i).is_some() {
                        weak.push(Weak { kind: "duplicate", file: p.clone(), detail: format!("added again in step {}", i + 1) });
                    }
                }
            }
        }
    }
    for &f in &d.files {
        let Some(&pf) = pos.get(&f) else {
            weak.push(Weak { kind: "missing", file: g.node(f).path.clone(), detail: "never added".into() });
            continue;
        };
        for &b in d.on.get(&f).map(|v| v.as_slice()).unwrap_or(&[]) {
            if d.scc[&b] == d.scc[&f] {
                continue;
            }
            if let Some(&pb) = pos.get(&b)
                && pb > pf
            {
                weak.push(Weak {
                    kind: "early",
                    file: g.node(f).path.clone(),
                    detail: format!("added in step {} but rests on {}, which arrives in step {}", pf + 1, g.node(b).path, pb + 1),
                });
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for &f in &d.files {
        if d.scc_size[d.scc[&f]] > 1 {
            groups.entry(d.scc[&f]).or_default().push(g.node(f).path.clone());
        }
    }
    let touched: HashSet<NodeId> = d.joints.keys().flat_map(|(a, b)| [*a, *b]).collect();
    let flow_files: HashSet<NodeId> = g
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Flow)
        .flat_map(|e| [file_of(g, e.from), file_of(g, e.to)])
        .flatten()
        .collect();
    let loose = d
        .files
        .iter()
        .filter(|f| !touched.contains(f) && !flow_files.contains(f))
        .map(|f| g.node(*f).path.clone())
        .collect();
    let pieces = d.files.iter().map(|&f| layer_count(g, f) as u32).sum();
    Check {
        ok: weak.is_empty(),
        pieces,
        files: d.files.len() as u32,
        steps: design.steps.len() as u32,
        sub_builds: design.sub_builds.len() as u32,
        joints: d.joints.values().sum(),
        bridges: g.stats.flows,
        weak,
        interlocked: groups.into_values().collect(),
        loose,
        unresolved: g.stats.unresolved_calls,
        gaps: query::endpoints(g).iter().filter(|e| e.status != "ok").count() as u32,
        repairs: vec![],
    }
}

/// Make a design hold together: drop unknown and duplicate files, add missing
/// ones, and move any file placed before something it rests on into a new step
/// right after its last support. Returns the repaired design and what changed.
pub fn repair(g: &Graph, design: &Design) -> (Design, Vec<String>) {
    let d = deps(g);
    let order = build_order(g, &d);
    let mut notes = Vec::new();
    let mut subs = design.sub_builds.clone();
    let sub_ids: HashSet<String> = subs.iter().map(|s| s.id.clone()).collect();
    // Every package needs a sub-build to hold its files.
    for pkg in order.iter().map(|f| d.package[f].clone()) {
        let id = slug(&pkg);
        if !subs.iter().any(|s| s.id == id || s.package == pkg) {
            notes.push(format!("added sub-build {pkg}"));
            subs.push(SubBuild { id, name: pkg.clone(), blurb: String::new(), package: pkg });
        }
    }
    let sub_for_pkg = |pkg: &str| -> String {
        subs.iter().find(|s| s.package == pkg).or_else(|| subs.iter().find(|s| s.id == slug(pkg))).map(|s| s.id.clone()).unwrap_or_else(|| slug(pkg))
    };
    // (order key, step)
    let mut steps: Vec<(f64, Step)> = Vec::new();
    let mut placed: HashSet<NodeId> = HashSet::new();
    for (i, s) in design.steps.iter().enumerate() {
        let mut files = Vec::new();
        for p in &s.files {
            match g.find_by_path(p).filter(|n| n.kind == NodeKind::File && !n.external) {
                None => notes.push(format!("dropped {p}: no such file")),
                Some(n) if !placed.insert(n.id) => notes.push(format!("dropped a second copy of {p}")),
                Some(_) => files.push(p.clone()),
            }
        }
        let mut s = s.clone();
        if !sub_ids.contains(&s.sub_build)
            && let Some(first) = files.first().and_then(|p| g.find_by_path(p))
        {
            s.sub_build = sub_for_pkg(&d.package[&first.id]);
        }
        s.files = files;
        steps.push((i as f64, s));
    }
    let mut key_of: HashMap<NodeId, f64> = HashMap::new();
    for (k, s) in &steps {
        for p in &s.files {
            if let Some(n) = g.find_by_path(p) {
                key_of.insert(n.id, *k);
            }
        }
    }
    let mut bump = 0.0;
    for &f in &order {
        let need = d
            .on
            .get(&f)
            .map(|v| v.iter().filter(|b| d.scc[*b] != d.scc[&f]).filter_map(|b| key_of.get(b)).fold(f64::NEG_INFINITY, |a, &b| a.max(b)))
            .unwrap_or(f64::NEG_INFINITY);
        let path = g.node(f).path.clone();
        let have = key_of.get(&f).copied();
        let ok = matches!(have, Some(h) if h >= need);
        if ok {
            continue;
        }
        // take it out of wherever it was
        if have.is_some() {
            for (_, s) in steps.iter_mut() {
                s.files.retain(|p| p != &path);
            }
        }
        bump += 1e-4;
        let key = if need.is_finite() { need + bump } else { -1.0 + bump };
        let sub = sub_for_pkg(&d.package[&f]);
        notes.push(match have {
            Some(_) => format!("moved {path} after the pieces it rests on"),
            None => format!("added {path}, which the design left out"),
        });
        // Interlocked partners already placed together stay where they are.
        if let Some((_, s)) = steps.iter_mut().find(|(k, s)| (*k - key).abs() < 1e-9 && s.sub_build == sub) {
            s.files.push(path.clone());
        } else {
            steps.push((key, Step { sub_build: sub, title: format!("Add {}", base(&path)), caption: String::new(), files: vec![path.clone()] }));
        }
        key_of.insert(f, key);
    }
    steps.retain(|(_, s)| !s.files.is_empty());
    steps.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = design.clone();
    out.sub_builds = subs;
    out.steps = steps.into_iter().map(|(_, s)| s).collect();
    (out, notes)
}

// ---- geometry ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct District {
    pub sub_build: String,
    pub name: String,
    pub lang: Lang,
    pub x: i32,
    pub z: i32,
    pub w: u32,
    pub d: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Building {
    pub id: NodeId,
    pub path: String,
    pub name: String,
    pub lang: Lang,
    pub district: usize,
    pub x: i32,
    pub z: i32,
    pub w: u32,
    pub d: u32,
    pub layers: u32,
    /// Zero-based manual step that adds it.
    pub step: u32,
    /// Starts a trace across a boundary: a landmark with a lamp on top.
    pub lamp: bool,
    /// Buildings this one rests on (it imports them or calls into them).
    pub rests_on: Vec<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Brick {
    pub building: usize,
    /// Symbols this brick stands for (one, unless the file has more than fit).
    pub nodes: Vec<NodeId>,
    pub name: String,
    pub layer: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<SymbolKind>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sinks: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Bridge {
    pub from: usize,
    pub to: usize,
    pub label: String,
    pub step: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct BuildModel {
    /// Baseplate size in studs (width, depth).
    pub studs: (u32, u32),
    pub districts: Vec<District>,
    pub buildings: Vec<Building>,
    pub bricks: Vec<Brick>,
    pub bridges: Vec<Bridge>,
}

fn footprint(loc: u32) -> (u32, u32) {
    let w = 2 + (loc >= 80) as u32 + (loc >= 250) as u32 + (loc >= 600) as u32;
    let d = 2 + (loc >= 150) as u32 + (loc >= 400) as u32;
    (w, d)
}

fn layer_count(g: &Graph, f: NodeId) -> usize {
    symbols_of(g, f).len().clamp(1, MAX_LAYERS)
}

/// Lay the design out on a baseplate. Needs a design that passes [`check`];
/// files a design leaves out are not built.
pub fn geometry(g: &Graph, design: &Design) -> BuildModel {
    let step_of: HashMap<&str, u32> = design
        .steps
        .iter()
        .enumerate()
        .flat_map(|(i, s)| s.files.iter().map(move |p| (p.as_str(), i as u32)))
        .collect();
    let entries: HashSet<NodeId> = query::traces(g).iter().filter_map(|t| file_of(g, t.entry)).collect();
    let mut districts: Vec<District> = Vec::new();
    let mut buildings: Vec<Building> = Vec::new();
    let mut bricks: Vec<Brick> = Vec::new();
    // Pack each district's buildings into rows, in build order.
    struct Packed {
        w: u32,
        d: u32,
        items: Vec<(NodeId, i32, i32, u32, u32)>,
    }
    let mut packed: Vec<(usize, Packed)> = Vec::new();
    for (si, sb) in design.sub_builds.iter().enumerate() {
        let mut files: Vec<(u32, NodeId)> = design
            .steps
            .iter()
            .enumerate()
            .filter(|(_, s)| s.sub_build == sb.id)
            .flat_map(|(i, s)| s.files.iter().filter_map(move |p| g.find_by_path(p).map(|n| (i as u32, n.id))))
            .collect();
        files.sort();
        if files.is_empty() {
            continue;
        }
        let area: u32 = files.iter().map(|(_, f)| { let (w, d) = footprint(g.node(*f).loc); (w + 1) * (d + 1) }).sum();
        let row_w = ((area as f64).sqrt() * 1.25).ceil().max(6.0) as u32;
        let (mut x, mut z, mut row_d, mut max_w) = (1u32, 1u32, 0u32, 0u32);
        let mut items = Vec::new();
        for (_, f) in &files {
            let (w, d) = footprint(g.node(*f).loc);
            if x > 1 && x + w > row_w {
                x = 1;
                z += row_d + 1;
                row_d = 0;
            }
            items.push((*f, x as i32, z as i32, w, d));
            x += w + 1;
            row_d = row_d.max(d);
            max_w = max_w.max(x);
        }
        packed.push((si, Packed { w: max_w, d: z + row_d + 1, items }));
    }
    // Pack districts onto the baseplate with a two-stud canal between them.
    const CANAL: u32 = 2;
    let total: u32 = packed.iter().map(|(_, p)| (p.w + CANAL) * (p.d + CANAL)).sum();
    let plate_w = ((total as f64).sqrt() * 1.15).ceil() as u32 + CANAL;
    let (mut x, mut z, mut row_d, mut max_x) = (CANAL, CANAL, 0u32, 0u32);
    for (si, p) in &packed {
        if x > CANAL && x + p.w + CANAL > plate_w {
            x = CANAL;
            z += row_d + CANAL;
            row_d = 0;
        }
        let sb = &design.sub_builds[*si];
        let lang = p.items.first().map(|i| g.node(i.0).lang).unwrap_or(Lang::Other);
        let di = districts.len();
        districts.push(District { sub_build: sb.id.clone(), name: sb.name.clone(), lang, x: x as i32, z: z as i32, w: p.w, d: p.d });
        for &(f, bx, bz, w, d) in &p.items {
            let n = g.node(f);
            let bi = buildings.len();
            let syms = symbols_of(g, f);
            let layers = syms.len().clamp(1, MAX_LAYERS);
            buildings.push(Building {
                id: f,
                path: n.path.clone(),
                name: n.name.clone(),
                lang: n.lang,
                district: di,
                x: x as i32 + bx,
                z: z as i32 + bz,
                w,
                d,
                layers: layers as u32,
                step: step_of.get(n.path.as_str()).copied().unwrap_or(0),
                lamp: entries.contains(&f),
                rests_on: vec![],
            });
            if syms.is_empty() {
                bricks.push(Brick { building: bi, nodes: vec![f], name: n.name.clone(), layer: 0, kind: None, sinks: sink_tags(n) });
                continue;
            }
            // Spread symbols over the layers; the first symbol sits at the bottom.
            for layer in 0..layers {
                let lo = layer * syms.len() / layers;
                let hi = ((layer + 1) * syms.len() / layers).max(lo + 1);
                let group = &syms[lo..hi];
                let name = if group.len() == 1 { group[0].name.clone() } else { format!("{} +{}", group[0].name, group.len() - 1) };
                let mut sinks: Vec<String> = group.iter().flat_map(|s| sink_tags(s)).collect();
                sinks.sort();
                sinks.dedup();
                bricks.push(Brick { building: bi, nodes: group.iter().map(|s| s.id).collect(), name, layer: layer as u32, kind: group[0].symbol_kind, sinks });
            }
        }
        x += p.w + CANAL;
        row_d = row_d.max(p.d);
        max_x = max_x.max(x);
    }
    let depth = z + row_d + CANAL;
    let d = deps(g);
    let index: HashMap<NodeId, usize> = buildings.iter().enumerate().map(|(i, b)| (b.id, i)).collect();
    for b in buildings.iter_mut() {
        b.rests_on = d.on.get(&b.id).map(|v| v.iter().filter_map(|x| index.get(x).copied()).collect()).unwrap_or_default();
    }
    // Round the plate up to an even number of studs, so it stays snug around the districts.
    let round = |v: u32| v.div_ceil(2) * 2;
    let brick_of: HashMap<NodeId, usize> = bricks.iter().enumerate().flat_map(|(i, b)| b.nodes.iter().map(move |n| (*n, i))).collect();
    let mut bridges = Vec::new();
    let mut seen = HashSet::new();
    for e in &g.edges {
        if e.kind != EdgeKind::Flow {
            continue;
        }
        let (Some(&a), Some(&b)) = (brick_of.get(&e.from), brick_of.get(&e.to)) else { continue };
        if !seen.insert((a, b)) {
            continue;
        }
        let step = buildings[bricks[a].building].step.max(buildings[bricks[b].building].step);
        bridges.push(Bridge { from: a, to: b, label: e.label.clone().unwrap_or_default(), step });
    }
    BuildModel { studs: (round(max_x), round(depth)), districts, buildings, bricks, bridges }
}

fn sink_tags(n: &Node) -> Vec<String> {
    n.tags.iter().filter(|t| ["db", "fs", "queue", "process"].contains(&t.as_str())).cloned().collect()
}

/// Everything the app and the CLI show for a build.
#[derive(Debug, Clone, Serialize)]
pub struct Build {
    pub design: Design,
    pub check: Check,
    pub model: BuildModel,
    /// Set when a saved agent design was made for an older scan and the engine's was used instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale: Option<String>,
}

/// Check, repair if needed, and lay out a design.
pub fn assemble(g: &Graph, design: Design) -> Build {
    let first = check(g, &design);
    let (design, repairs) = if first.ok { (design, vec![]) } else { repair(g, &design) };
    let mut report = check(g, &design);
    report.repairs = repairs;
    let model = geometry(g, &design);
    Build { design, check: report, model, stale: None }
}

/// The build for a graph: the saved design if there is one (checked and repaired
/// against the current scan), otherwise the engine's.
pub fn for_graph(g: &Graph, saved: Option<Design>) -> Build {
    match saved {
        Some(d) => {
            let stale = (d.scanned_at != g.scanned_at)
                .then(|| format!("{} design made for an earlier scan ({}); checked and repaired against this one", d.source, d.scanned_at));
            let mut b = assemble(g, d);
            b.stale = stale;
            b
        }
        None => assemble(g, engine_design(g)),
    }
}
