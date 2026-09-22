//! The architecture graph: nodes for packages, files and symbols; edges for
//! containment, imports, calls and cross-boundary data flows.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type NodeId = u32;
pub type EdgeId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Other,
}

impl Lang {
    pub fn from_path(path: &str) -> Lang {
        let ext = path.rsplit('.').next().unwrap_or("");
        match ext {
            "rs" => Lang::Rust,
            "ts" | "tsx" | "mts" | "cts" => Lang::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" => Lang::JavaScript,
            "py" | "pyi" => Lang::Python,
            "go" => Lang::Go,
            _ => Lang::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::TypeScript => "typescript",
            Lang::JavaScript => "javascript",
            Lang::Python => "python",
            Lang::Go => "go",
            Lang::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Repo,
    Package,
    File,
    Symbol,
}

impl NodeKind {
    pub fn depth(self) -> u8 {
        match self {
            NodeKind::Repo => 0,
            NodeKind::Package => 1,
            NodeKind::File => 2,
            NodeKind::Symbol => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Struct,
    Enum,
    Trait,
    Interface,
    Type,
    Const,
    Module,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeKind {
    /// Parent contains child (repo→package→file→symbol).
    Contains,
    /// File imports another file (or an external package).
    Imports,
    /// Symbol calls another symbol.
    Calls,
    /// Data crosses a boundary: HTTP route, IPC command, queue topic, shared env var.
    Flow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    pub name: String,
    /// Repo-relative path. Symbols use `path#name`.
    pub path: String,
    pub lang: Lang,
    pub parent: Option<NodeId>,
    /// Lines of code (files) or lines spanned (symbols).
    pub loc: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol_kind: Option<SymbolKind>,
    /// 1-based start/end line for symbols.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<(u32, u32)>,
    /// Boundary tags such as `http-server`, `http-client`, `db`, `fs`, `env:NAME`, `route:/api/x`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// True for packages that live outside the repository (dependencies).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub external: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: EdgeId,
    pub from: NodeId,
    pub to: NodeId,
    pub kind: EdgeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub weight: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Stats {
    pub files: u32,
    pub symbols: u32,
    pub packages: u32,
    pub external_packages: u32,
    pub imports: u32,
    pub calls: u32,
    pub flows: u32,
    pub unresolved_calls: u32,
    pub loc: u64,
    pub by_lang: Vec<LangStat>,
    pub scan_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LangStat {
    pub lang: String,
    pub files: u32,
    pub loc: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Graph {
    pub schema: u32,
    pub root: String,
    pub scanned_at: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub stats: Stats,
}

pub const SCHEMA_VERSION: u32 = 1;

impl Graph {
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    pub fn children(&self, id: NodeId) -> impl Iterator<Item = &Node> {
        self.nodes.iter().filter(move |n| n.parent == Some(id))
    }

    /// Walk up to the ancestor at the given kind (or self if already that kind).
    pub fn ancestor_of_kind(&self, id: NodeId, kind: NodeKind) -> Option<NodeId> {
        let mut cur = Some(id);
        while let Some(c) = cur {
            let n = self.node(c);
            if n.kind == kind {
                return Some(c);
            }
            if n.kind.depth() < kind.depth() {
                return None;
            }
            cur = n.parent;
        }
        None
    }

    pub fn find_by_path(&self, path: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.path == path)
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<&Node> {
        let q = query.to_lowercase();
        let mut hits: Vec<(&Node, i32)> = self
            .nodes
            .iter()
            .filter_map(|n| {
                let name = n.name.to_lowercase();
                let path = n.path.to_lowercase();
                let score = if name == q {
                    100
                } else if name.starts_with(&q) {
                    60
                } else if name.contains(&q) {
                    40
                } else if path.contains(&q) {
                    20
                } else {
                    return None;
                };
                Some((n, score - n.kind.depth() as i32))
            })
            .collect();
        hits.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.path.cmp(&b.0.path)));
        hits.into_iter().take(limit).map(|(n, _)| n).collect()
    }

    /// Collapse the graph to a single hierarchy level. Edges between descendants
    /// are rolled up onto their ancestors at that level and their weights summed.
    pub fn view(&self, level: NodeKind, focus: Option<NodeId>) -> ViewGraph {
        let mut keep: Vec<bool> = vec![false; self.nodes.len()];
        for n in &self.nodes {
            // External packages are leaves: show them at every level so imports of
            // dependencies stay visible next to the files that use them.
            if n.kind == level || (n.external && level.depth() >= NodeKind::Package.depth()) {
                keep[n.id as usize] = true;
            }
        }
        // Symbols of the focused file, or files of the focused package, get
        // expanded one level deeper so the user can zoom in without changing the level.
        if let Some(f) = focus {
            for n in &self.nodes {
                if n.parent == Some(f) && n.kind.depth() == level.depth() + 1 {
                    keep[n.id as usize] = true;
                    keep[f as usize] = false;
                }
            }
        }
        let lift = |id: NodeId| -> Option<NodeId> {
            let mut cur = Some(id);
            while let Some(c) = cur {
                if keep[c as usize] {
                    return Some(c);
                }
                cur = self.node(c).parent;
            }
            None
        };
        let mut agg: HashMap<(NodeId, NodeId, EdgeKind), (u32, Vec<String>)> = HashMap::new();
        for e in &self.edges {
            if e.kind == EdgeKind::Contains {
                continue;
            }
            let (Some(a), Some(b)) = (lift(e.from), lift(e.to)) else {
                continue;
            };
            if a == b {
                continue;
            }
            let entry = agg.entry((a, b, e.kind)).or_default();
            entry.0 += e.weight;
            if let Some(l) = &e.label
                && entry.1.len() < 6
                && !entry.1.contains(l)
            {
                entry.1.push(l.clone());
            }
        }
        let mut nodes: Vec<ViewNode> = self
            .nodes
            .iter()
            .filter(|n| keep[n.id as usize])
            .map(|n| ViewNode {
                id: n.id,
                name: n.name.clone(),
                path: n.path.clone(),
                kind: n.kind,
                lang: n.lang,
                loc: n.loc,
                // Dependencies are shared by every package, so they form one loose group of their own.
                group: if n.external {
                    0
                } else {
                    self.ancestor_of_kind(n.id, NodeKind::Package).unwrap_or(0)
                },
                group_name: if n.external {
                    "dependencies".to_string()
                } else {
                    self.ancestor_of_kind(n.id, NodeKind::Package)
                        .map(|p| self.node(p).name.clone())
                        .unwrap_or_default()
                },
                tags: n.tags.clone(),
                external: n.external,
                degree: 0,
            })
            .collect();
        let index: HashMap<NodeId, usize> =
            nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
        let mut edges: Vec<ViewEdge> = agg
            .into_iter()
            .map(|((from, to, kind), (weight, labels))| ViewEdge {
                from,
                to,
                kind,
                weight,
                labels,
            })
            .collect();
        edges.sort_by_key(|e| (e.from, e.to, e.kind as u8));
        for e in &edges {
            if let Some(&i) = index.get(&e.from) {
                nodes[i].degree += e.weight;
            }
            if let Some(&i) = index.get(&e.to) {
                nodes[i].degree += e.weight;
            }
        }
        ViewGraph {
            level,
            focus,
            nodes,
            edges,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewNode {
    pub id: NodeId,
    pub name: String,
    pub path: String,
    pub kind: NodeKind,
    pub lang: Lang,
    pub loc: u32,
    /// Package the node belongs to; used for cluster gravity in layout.
    pub group: NodeId,
    pub group_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub external: bool,
    pub degree: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewEdge {
    pub from: NodeId,
    pub to: NodeId,
    pub kind: EdgeKind,
    pub weight: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewGraph {
    pub level: NodeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<NodeId>,
    pub nodes: Vec<ViewNode>,
    pub edges: Vec<ViewEdge>,
}
