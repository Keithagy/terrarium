//! Read-only questions over a graph: neighbours, paths, boundaries, hotspots.

use crate::model::*;
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Clone, Serialize)]
pub struct Neighbour {
    pub id: NodeId,
    pub name: String,
    pub path: String,
    pub kind: NodeKind,
    pub edge: EdgeKind,
    pub direction: &'static str, // "in" | "out"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub weight: u32,
}

pub fn neighbours(g: &Graph, id: NodeId) -> Vec<Neighbour> {
    let mut out = Vec::new();
    for e in &g.edges {
        if e.kind == EdgeKind::Contains {
            continue;
        }
        if e.from == id {
            let n = g.node(e.to);
            out.push(Neighbour {
                id: n.id,
                name: n.name.clone(),
                path: n.path.clone(),
                kind: n.kind,
                edge: e.kind,
                direction: "out",
                label: e.label.clone(),
                weight: e.weight,
            });
        } else if e.to == id {
            let n = g.node(e.from);
            out.push(Neighbour {
                id: n.id,
                name: n.name.clone(),
                path: n.path.clone(),
                kind: n.kind,
                edge: e.kind,
                direction: "in",
                label: e.label.clone(),
                weight: e.weight,
            });
        }
    }
    out.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| a.path.cmp(&b.path)));
    out
}

/// Shortest path (by hop count) following non-containment edges in either direction.
pub fn path_between(g: &Graph, from: NodeId, to: NodeId) -> Option<Vec<NodeId>> {
    let mut adj: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for e in &g.edges {
        if e.kind == EdgeKind::Contains {
            continue;
        }
        adj.entry(e.from).or_default().push(e.to);
        adj.entry(e.to).or_default().push(e.from);
    }
    let mut prev: HashMap<NodeId, NodeId> = HashMap::new();
    let mut seen: HashSet<NodeId> = HashSet::from([from]);
    let mut q = VecDeque::from([from]);
    while let Some(cur) = q.pop_front() {
        if cur == to {
            let mut path = vec![to];
            let mut c = to;
            while let Some(&p) = prev.get(&c) {
                path.push(p);
                c = p;
            }
            path.reverse();
            return Some(path);
        }
        for &n in adj.get(&cur).map(|v| v.as_slice()).unwrap_or(&[]) {
            if seen.insert(n) {
                prev.insert(n, cur);
                q.push_back(n);
            }
        }
    }
    None
}

#[derive(Debug, Clone, Serialize)]
pub struct Boundary {
    pub id: NodeId,
    pub name: String,
    pub path: String,
    pub lang: Lang,
    pub tags: Vec<String>,
}

/// Every node touching the outside world, optionally filtered by tag prefix (`http`, `db`, `env:`...).
pub fn boundaries(g: &Graph, filter: Option<&str>) -> Vec<Boundary> {
    let interesting = [
        "http-server",
        "http-client",
        "db",
        "fs",
        "env",
        "ipc-server",
        "ipc-client",
        "queue",
        "process",
    ];
    g.nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Symbol || n.kind == NodeKind::File)
        .filter(|n| n.tags.iter().any(|t| interesting.contains(&t.as_str())))
        .filter(|n| {
            filter
                .map(|f| n.tags.iter().any(|t| t.starts_with(f)))
                .unwrap_or(true)
        })
        .map(|n| Boundary {
            id: n.id,
            name: n.name.clone(),
            path: n.path.clone(),
            lang: n.lang,
            tags: n.tags.clone(),
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct Hotspot {
    pub id: NodeId,
    pub name: String,
    pub path: String,
    pub kind: NodeKind,
    pub fan_in: u32,
    pub fan_out: u32,
    pub loc: u32,
}

/// Files ranked by fan-in + fan-out (the ones everything depends on, or that depend on everything).
pub fn hotspots(g: &Graph, level: NodeKind, limit: usize) -> Vec<Hotspot> {
    let v = g.view(level, None);
    let mut fan: HashMap<NodeId, (u32, u32)> = HashMap::new();
    for e in &v.edges {
        fan.entry(e.from).or_default().1 += e.weight;
        fan.entry(e.to).or_default().0 += e.weight;
    }
    let mut out: Vec<Hotspot> = v
        .nodes
        .iter()
        .filter(|n| !n.external)
        .map(|n| {
            let (fi, fo) = fan.get(&n.id).copied().unwrap_or_default();
            Hotspot {
                id: n.id,
                name: n.name.clone(),
                path: n.path.clone(),
                kind: n.kind,
                fan_in: fi,
                fan_out: fo,
                loc: n.loc,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        (b.fan_in + b.fan_out)
            .cmp(&(a.fan_in + a.fan_out))
            .then_with(|| a.path.cmp(&b.path))
    });
    out.truncate(limit);
    out
}

#[derive(Debug, Clone, Serialize)]
pub struct FlowRow {
    pub from: NodeId,
    pub from_path: String,
    pub from_lang: Lang,
    pub to: NodeId,
    pub to_path: String,
    pub to_lang: Lang,
    pub label: String,
}

pub fn flows(g: &Graph) -> Vec<FlowRow> {
    g.edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Flow)
        .map(|e| {
            let a = g.node(e.from);
            let b = g.node(e.to);
            FlowRow {
                from: a.id,
                from_path: a.path.clone(),
                from_lang: a.lang,
                to: b.id,
                to_path: b.path.clone(),
                to_lang: b.lang,
                label: e.label.clone().unwrap_or_default(),
            }
        })
        .collect()
}

/// Cycles between files (strongly connected components of size > 1).
pub fn cycles(g: &Graph, level: NodeKind) -> Vec<Vec<NodeId>> {
    let v = g.view(level, None);
    let ids: Vec<NodeId> = v.nodes.iter().map(|n| n.id).collect();
    let idx: HashMap<NodeId, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let mut adj: Vec<Vec<usize>> = vec![vec![]; ids.len()];
    for e in &v.edges {
        if let (Some(&a), Some(&b)) = (idx.get(&e.from), idx.get(&e.to)) {
            adj[a].push(b);
        }
    }
    // Tarjan
    struct T<'a> {
        adj: &'a [Vec<usize>],
        index: Vec<i64>,
        low: Vec<i64>,
        on: Vec<bool>,
        stack: Vec<usize>,
        next: i64,
        out: Vec<Vec<usize>>,
    }
    fn strong(t: &mut T, v: usize) {
        t.index[v] = t.next;
        t.low[v] = t.next;
        t.next += 1;
        t.stack.push(v);
        t.on[v] = true;
        for i in 0..t.adj[v].len() {
            let w = t.adj[v][i];
            if t.index[w] < 0 {
                strong(t, w);
                t.low[v] = t.low[v].min(t.low[w]);
            } else if t.on[w] {
                t.low[v] = t.low[v].min(t.index[w]);
            }
        }
        if t.low[v] == t.index[v] {
            let mut comp = vec![];
            loop {
                let w = t.stack.pop().unwrap();
                t.on[w] = false;
                comp.push(w);
                if w == v {
                    break;
                }
            }
            if comp.len() > 1 {
                t.out.push(comp);
            }
        }
    }
    let n = ids.len();
    let mut t = T {
        adj: &adj,
        index: vec![-1; n],
        low: vec![0; n],
        on: vec![false; n],
        stack: vec![],
        next: 0,
        out: vec![],
    };
    for v in 0..n {
        if t.index[v] < 0 {
            strong(&mut t, v);
        }
    }
    let mut out: Vec<Vec<NodeId>> = t
        .out
        .into_iter()
        .map(|c| c.into_iter().map(|i| ids[i]).collect())
        .collect();
    out.sort_by_key(|c| std::cmp::Reverse(c.len()));
    out
}

// ---- traces ------------------------------------------------------------------

/// Tags that mark where a trace comes to rest: the data leaves the code here.
const SINK_TAGS: [&str; 4] = ["db", "fs", "queue", "process"];
const TRACE_MAX_DEPTH: u32 = 24;
const TRACE_MAX_STEPS: usize = 300;

#[derive(Debug, Clone, Serialize)]
pub struct TraceStep {
    pub id: NodeId,
    pub name: String,
    pub path: String,
    pub lang: Lang,
    /// Package the step runs in; the trace view draws one lane per package.
    pub lane: String,
    pub depth: u32,
    /// Index of the calling step; `None` for the entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<usize>,
    /// How the parent reached this step: `calls` or `flow`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<EdgeKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sinks: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// Already expanded earlier in this trace; its calls are not repeated here.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub repeat: bool,
}

/// One end-to-end path through the code: an entry point, the calls under it, and
/// every boundary it crosses on the way to its sinks. Branches that never cross a
/// boundary or reach a sink are pruned, so what is left is how data moves.
#[derive(Debug, Clone, Serialize)]
pub struct Trace {
    pub entry: NodeId,
    pub entry_path: String,
    pub name: String,
    /// Number of flow edges crossed.
    pub hops: u32,
    /// Languages in the order the trace first reaches them.
    pub langs: Vec<Lang>,
    pub lanes: Vec<String>,
    /// Flow labels in order, deduplicated (`http /api/users`, `ipc scan_repo`).
    pub via: Vec<String>,
    pub sinks: Vec<String>,
    pub truncated: bool,
    pub steps: Vec<TraceStep>,
}

struct Hops {
    out: HashMap<NodeId, Vec<(NodeId, EdgeKind, Option<String>)>>,
    has_in: HashSet<NodeId>,
}

/// Calls and flows, minus a file calling its own symbols (module-level code), which
/// would otherwise make every file the entry of its own functions.
fn hop_graph(g: &Graph) -> Hops {
    let mut out: HashMap<NodeId, Vec<(NodeId, EdgeKind, Option<String>)>> = HashMap::new();
    let mut has_in = HashSet::new();
    for e in &g.edges {
        if e.kind != EdgeKind::Calls && e.kind != EdgeKind::Flow {
            continue;
        }
        if e.kind == EdgeKind::Calls && g.node(e.from).kind == NodeKind::File && g.node(e.to).parent == Some(e.from) {
            continue;
        }
        out.entry(e.from).or_default().push((e.to, e.kind, e.label.clone()));
        has_in.insert(e.to);
    }
    Hops { out, has_in }
}

fn sinks_of(n: &Node, hops: &Hops) -> Vec<String> {
    let mut s: Vec<String> = n
        .tags
        .iter()
        .filter(|t| SINK_TAGS.contains(&t.as_str()))
        .cloned()
        .collect();
    // A client call that pairs with nothing leaves the repository: an external API, or a typo.
    let crosses = hops.out.get(&n.id).is_some_and(|v| v.iter().any(|h| h.1 == EdgeKind::Flow));
    if !crosses && n.tags.iter().any(|t| t == "http-client" || t == "ipc-client") {
        s.extend(n.tags.iter().filter(|t| t.starts_with("route:") || t.starts_with("ipc:")).map(|t| format!("unmatched {t}")));
    }
    s
}

/// Whether a node, or anything it reaches, crosses a boundary or rests in a sink.
fn leads_somewhere(g: &Graph, hops: &Hops, id: NodeId, memo: &mut HashMap<NodeId, bool>) -> bool {
    if let Some(&v) = memo.get(&id) {
        return v;
    }
    memo.insert(id, false); // cycles count as "not yet"
    let n = g.node(id);
    let mut v = !sinks_of(n, hops).is_empty();
    for (to, kind, _) in hops.out.get(&id).map(|v| v.as_slice()).unwrap_or(&[]) {
        if *kind == EdgeKind::Flow || leads_somewhere(g, hops, *to, memo) {
            v = true;
        }
    }
    memo.insert(id, v);
    v
}

fn lane_of(g: &Graph, id: NodeId) -> String {
    g.ancestor_of_kind(id, NodeKind::Package)
        .map(|p| g.node(p).name.clone())
        .unwrap_or_default()
}

/// Every end-to-end trace that crosses at least one boundary, longest journeys first.
pub fn traces(g: &Graph) -> Vec<Trace> {
    let hops = hop_graph(g);
    let mut memo = HashMap::new();
    let mut entries: Vec<NodeId> = hops
        .out
        .keys()
        .copied()
        .filter(|id| !hops.has_in.contains(id))
        .collect();
    entries.sort();
    let mut out: Vec<Trace> = Vec::new();
    for id in entries {
        if leads_somewhere(g, &hops, id, &mut memo)
            && let Some(t) = build_trace(g, &hops, &mut memo, id)
        {
            out.push(t);
        }
    }
    out.sort_by(|a, b| {
        b.hops
            .cmp(&a.hops)
            .then_with(|| b.langs.len().cmp(&a.langs.len()))
            .then_with(|| b.steps.len().cmp(&a.steps.len()))
            .then_with(|| a.entry_path.cmp(&b.entry_path))
    });
    out
}

/// The trace starting at `entry`, whether or not `entry` is called by something else.
pub fn trace_from(g: &Graph, entry: NodeId) -> Option<Trace> {
    let hops = hop_graph(g);
    build_trace(g, &hops, &mut HashMap::new(), entry)
}

fn build_trace(g: &Graph, hops: &Hops, memo: &mut HashMap<NodeId, bool>, entry: NodeId) -> Option<Trace> {
    let mut steps: Vec<TraceStep> = Vec::new();
    let mut truncated = false;
    let mut on_path: Vec<NodeId> = Vec::new();
    let mut expanded: HashSet<NodeId> = HashSet::new();
    // Explicit stack of (node, parent step, depth, via, label) in pre-order.
    type Pending = (NodeId, Option<usize>, u32, Option<EdgeKind>, Option<String>);
    let mut stack: Vec<Pending> = vec![(entry, None, 0, None, None)];
    while let Some((id, parent, depth, via, label)) = stack.pop() {
        if steps.len() >= TRACE_MAX_STEPS {
            truncated = true;
            break;
        }
        // on_path mirrors the ancestors of the step being added
        on_path.truncate(depth as usize);
        let n = g.node(id);
        let idx = steps.len();
        let repeat = !expanded.insert(id) && hops.out.contains_key(&id);
        steps.push(TraceStep {
            id,
            name: n.name.clone(),
            path: n.path.clone(),
            lang: n.lang,
            lane: lane_of(g, id),
            depth,
            parent,
            via,
            label,
            sinks: sinks_of(n, hops),
            line: n.span.map(|s| s.0),
            repeat,
        });
        on_path.push(id);
        if repeat {
            continue;
        }
        if depth >= TRACE_MAX_DEPTH {
            truncated |= hops.out.contains_key(&id);
            continue;
        }
        let kids = hops.out.get(&id).map(|v| v.as_slice()).unwrap_or(&[]);
        for (to, kind, lbl) in kids.iter().rev() {
            if on_path.contains(to) {
                continue;
            }
            if *kind == EdgeKind::Flow || leads_somewhere(g, hops, *to, memo) {
                stack.push((*to, Some(idx), depth + 1, Some(*kind), lbl.clone()));
            }
        }
    }
    let flow_count = steps.iter().filter(|s| s.via == Some(EdgeKind::Flow)).count() as u32;
    if flow_count == 0 {
        return None;
    }
    let mut langs = Vec::new();
    let mut lanes = Vec::new();
    let mut via = Vec::new();
    let mut sinks = Vec::new();
    for s in &steps {
        if !langs.contains(&s.lang) {
            langs.push(s.lang);
        }
        if !lanes.contains(&s.lane) {
            lanes.push(s.lane.clone());
        }
        if s.via == Some(EdgeKind::Flow)
            && let Some(l) = &s.label
            && !via.contains(l)
        {
            via.push(l.clone());
        }
        for k in &s.sinks {
            if !sinks.contains(k) {
                sinks.push(k.clone());
            }
        }
    }
    let n = g.node(entry);
    Some(Trace {
        entry,
        entry_path: n.path.clone(),
        name: n.name.clone(),
        hops: flow_count,
        langs,
        lanes,
        via,
        sinks,
        truncated,
        steps,
    })
}

// ---- endpoints -------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct EndpointRef {
    pub id: NodeId,
    pub name: String,
    pub path: String,
    pub lang: Lang,
}

/// One contract between code that calls and code that answers: an HTTP route, an
/// IPC command or a queue topic, with both sides and whether they pair up.
#[derive(Debug, Clone, Serialize)]
pub struct Endpoint {
    /// `http /api/users`, `ipc scan_repo`, `queue emails`.
    pub key: String,
    pub kind: &'static str,
    /// `ok`, `no-callers` (served, nothing in the repo calls it) or
    /// `no-handler` (called, nothing in the repo serves it).
    pub status: &'static str,
    pub handlers: Vec<EndpointRef>,
    pub callers: Vec<EndpointRef>,
}

fn eref(g: &Graph, id: NodeId) -> EndpointRef {
    let n = g.node(id);
    EndpointRef {
        id,
        name: n.name.clone(),
        path: n.path.clone(),
        lang: n.lang,
    }
}

/// Every endpoint in the repository, gaps first.
pub fn endpoints(g: &Graph) -> Vec<Endpoint> {
    // key -> (handlers, callers)
    let mut map: HashMap<String, (Vec<NodeId>, Vec<NodeId>)> = HashMap::new();
    let push = |v: &mut Vec<NodeId>, id: NodeId| {
        if !v.contains(&id) {
            v.push(id);
        }
    };
    let mut flowing: HashSet<NodeId> = HashSet::new();
    for e in &g.edges {
        if e.kind != EdgeKind::Flow {
            continue;
        }
        let Some(l) = &e.label else { continue };
        let entry = map.entry(l.clone()).or_default();
        push(&mut entry.0, e.to);
        push(&mut entry.1, e.from);
        flowing.insert(e.from);
        flowing.insert(e.to);
    }
    let flow_keys: HashSet<String> = map.keys().cloned().collect();
    for n in &g.nodes {
        if n.kind != NodeKind::Symbol && n.kind != NodeKind::File {
            continue;
        }
        let has = |t: &str| n.tags.iter().any(|x| x == t);
        let server = has("http-server") || has("ipc-server");
        let client = has("http-client") || has("ipc-client");
        for t in &n.tags {
            let key = if let Some(r) = t.strip_prefix("route:") {
                format!("http {r}")
            } else if let Some(c) = t.strip_prefix("ipc:") {
                format!("ipc {c}")
            } else if let Some(q) = t.strip_prefix("topic:") {
                format!("queue {q}")
            } else {
                continue;
            };
            // Keys and nodes already paired by a flow are accounted for under the flow's
            // label; a route tag on the code that merely registers a handler is not a second handler.
            if flowing.contains(&n.id) || flow_keys.contains(&key) {
                continue;
            }
            let entry = map.entry(key).or_default();
            if t.starts_with("topic:") {
                if has("queue-producer") {
                    push(&mut entry.1, n.id);
                } else {
                    push(&mut entry.0, n.id);
                }
            } else if server {
                push(&mut entry.0, n.id);
            } else if client {
                push(&mut entry.1, n.id);
            }
        }
    }
    let mut out: Vec<Endpoint> = map
        .into_iter()
        .filter(|(_, (h, c))| !h.is_empty() || !c.is_empty())
        .map(|(key, (h, c))| {
            let kind = match key.split(' ').next() {
                Some("http") => "http",
                Some("ipc") => "ipc",
                _ => "queue",
            };
            let status = if h.is_empty() {
                "no-handler"
            } else if c.is_empty() {
                "no-callers"
            } else {
                "ok"
            };
            Endpoint {
                key,
                kind,
                status,
                handlers: h.into_iter().map(|id| eref(g, id)).collect(),
                callers: c.into_iter().map(|id| eref(g, id)).collect(),
            }
        })
        .collect();
    out.sort_by(|a, b| {
        (a.status == "ok")
            .cmp(&(b.status == "ok"))
            .then_with(|| a.key.cmp(&b.key))
    });
    out
}
