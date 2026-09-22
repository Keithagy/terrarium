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
