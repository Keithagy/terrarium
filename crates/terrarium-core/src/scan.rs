//! Repository scanner: walks files, extracts facts per language in parallel,
//! resolves imports and calls into graph edges, and derives cross-boundary flows.

use crate::lang::{self, FileFacts};
use crate::model::*;
use crate::tags::{self, Binding};
use anyhow::{Context, Result};
use ignore::WalkBuilder;
use once_cell::sync::Lazy;
use rayon::prelude::*;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tracing::{debug, info, info_span, warn};

#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Skip files larger than this (bytes).
    pub max_file_bytes: u64,
    /// Extra directory names to skip.
    pub skip_dirs: Vec<String>,
    /// Include external packages (dependencies) as nodes.
    pub include_external: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            max_file_bytes: 2 * 1024 * 1024,
            skip_dirs: vec![
                "node_modules",
                "target",
                "dist",
                "build",
                "vendor",
                ".venv",
                "venv",
                "__pycache__",
                ".git",
                ".next",
                "out",
                "coverage",
                ".turbo",
                ".cache",
                "site-packages",
                "third_party",
                "gen",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            include_external: true,
        }
    }
}

#[derive(Debug, Clone)]
struct PackageInfo {
    dir: PathBuf, // repo-relative ("" for root)
    name: String,
    kind: &'static str, // cargo | npm | go | python | root
    go_module: Option<String>,
}

struct FileEntry {
    rel: String,
    lang: Lang,
    facts: FileFacts,
    src_env: Vec<(u32, String)>, // (line, ENV_NAME) found by text scan
}

static ENV_MEMBER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?:process\.env\.([A-Z][A-Z0-9_]+)|import\.meta\.env\.([A-Z][A-Z0-9_]+)|os\.environ\[\s*["']([A-Z][A-Z0-9_]+)["']\s*\]|process\.env\[\s*["']([A-Z][A-Z0-9_]+)["']\s*\])"#).unwrap()
});

pub fn scan(root: &Path, opts: &ScanOptions) -> Result<Graph> {
    let started = Instant::now();
    let root = root
        .canonicalize()
        .with_context(|| format!("cannot open {}", root.display()))?;
    let _span = info_span!("scan", root = %root.display()).entered();

    // 1. Walk --------------------------------------------------------------
    let (files, manifests) = {
        let _s = info_span!("walk").entered();
        walk_files(&root, opts)
    };
    info!(
        files = files.len(),
        manifests = manifests.len(),
        "walked repository"
    );

    // 2. Packages ------------------------------------------------------------
    let mut packages = detect_packages(&root, &manifests);
    if !packages.iter().any(|p| p.dir.as_os_str().is_empty()) {
        packages.push(PackageInfo {
            dir: PathBuf::new(),
            name: root
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "repo".into()),
            kind: "root",
            go_module: None,
        });
    }
    // deepest dir first for nearest-ancestor lookups
    packages.sort_by_key(|p| std::cmp::Reverse(p.dir.components().count()));

    // 3. Parse (parallel) -----------------------------------------------------
    let entries: Vec<FileEntry> = {
        let _s = info_span!("parse", files = files.len()).entered();
        files
            .par_iter()
            .filter_map(|rel| {
                let abs = root.join(rel);
                let bytes = std::fs::read(&abs).ok()?;
                if bytes.len() as u64 > opts.max_file_bytes {
                    debug!(file = %rel, "skipping large file");
                    return None;
                }
                if looks_minified(&bytes) {
                    debug!(file = %rel, "skipping minified file");
                    return None;
                }
                let lang = Lang::from_path(rel);
                let facts = lang::extract(lang, &bytes, rel)?;
                let text = String::from_utf8_lossy(&bytes);
                let mut src_env = Vec::new();
                for (i, l) in text.lines().enumerate() {
                    for cap in ENV_MEMBER.captures_iter(l) {
                        let name = cap
                            .get(1)
                            .or(cap.get(2))
                            .or(cap.get(3))
                            .or(cap.get(4))
                            .map(|m| m.as_str().to_string());
                        if let Some(n) = name {
                            src_env.push((i as u32 + 1, n));
                        }
                    }
                }
                Some(FileEntry {
                    rel: rel.clone(),
                    lang,
                    facts,
                    src_env,
                })
            })
            .collect()
    };
    info!(parsed = entries.len(), "parsed files");

    // 4. Build nodes ----------------------------------------------------------
    let mut b = Builder::new(&root);
    let repo_id = b.push(Node {
        id: 0,
        kind: NodeKind::Repo,
        name: root
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default(),
        path: String::new(),
        lang: Lang::Other,
        parent: None,
        loc: 0,
        symbol_kind: None,
        span: None,
        tags: vec![],
        external: false,
    });

    // packages, shallowest first so parents exist
    let mut pkg_ids: HashMap<PathBuf, NodeId> = HashMap::new();
    let mut pkgs_sorted = packages.clone();
    pkgs_sorted.sort_by_key(|p| p.dir.components().count());
    for p in &pkgs_sorted {
        let parent = nearest_package(&packages, p.dir.parent().unwrap_or(Path::new("")))
            .and_then(|pp| pkg_ids.get(&pp.dir).copied())
            .unwrap_or(repo_id);
        let id = b.push(Node {
            id: 0,
            kind: NodeKind::Package,
            name: p.name.clone(),
            path: p.dir.to_string_lossy().to_string(),
            lang: match p.kind {
                "cargo" => Lang::Rust,
                "npm" => Lang::TypeScript,
                "go" => Lang::Go,
                "python" => Lang::Python,
                _ => Lang::Other,
            },
            parent: Some(parent),
            loc: 0,
            symbol_kind: None,
            span: None,
            tags: vec![p.kind.to_string()],
            external: false,
        });
        b.edge(parent, id, EdgeKind::Contains, None);
        pkg_ids.insert(p.dir.clone(), id);
    }

    // files and symbols
    let mut file_ids: HashMap<String, NodeId> = HashMap::new();
    // per file: def index -> symbol id
    let mut def_ids: Vec<Vec<NodeId>> = Vec::with_capacity(entries.len());
    for e in &entries {
        let rel_path = Path::new(&e.rel);
        let pkg = nearest_package(&packages, rel_path.parent().unwrap_or(Path::new("")))
            .and_then(|p| pkg_ids.get(&p.dir).copied())
            .unwrap_or(repo_id);
        let fid = b.push(Node {
            id: 0,
            kind: NodeKind::File,
            name: rel_path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default(),
            path: e.rel.clone(),
            lang: e.lang,
            parent: Some(pkg),
            loc: e.facts.loc,
            symbol_kind: None,
            span: None,
            tags: vec![],
            external: false,
        });
        b.edge(pkg, fid, EdgeKind::Contains, None);
        file_ids.insert(e.rel.clone(), fid);
        let mut ids = Vec::with_capacity(e.facts.defs.len());
        for d in &e.facts.defs {
            let parent = d.parent.map(|p| ids[p]).unwrap_or(fid);
            let sid = b.push(Node {
                id: 0,
                kind: NodeKind::Symbol,
                name: d.name.clone(),
                path: format!("{}#{}", e.rel, d.name),
                lang: e.lang,
                parent: Some(parent),
                loc: d.end_line.saturating_sub(d.start_line) + 1,
                symbol_kind: Some(d.kind),
                span: Some((d.start_line, d.end_line)),
                tags: vec![],
                external: false,
            });
            b.edge(parent, sid, EdgeKind::Contains, None);
            ids.push(sid);
        }
        def_ids.push(ids);
    }

    // 5. Tags + bindings ------------------------------------------------------
    let mut bindings: Vec<(Binding, usize /*file idx*/)> = Vec::new();
    for (fi, e) in entries.iter().enumerate() {
        let fid = file_ids[&e.rel];
        for (di, d) in e.facts.defs.iter().enumerate() {
            let ct = tags::classify_def(d);
            let sid = def_ids[fi][di];
            b.add_tags(sid, &ct.tags);
            for mut bd in ct.bindings {
                bd.def = Some(di);
                bindings.push((bd, fi));
            }
        }
        for c in &e.facts.calls {
            let ct = tags::classify_call(c, e.lang);
            if ct.tags.is_empty() && ct.bindings.is_empty() {
                continue;
            }
            let owner = c.def.map(|d| def_ids[fi][d]).unwrap_or(fid);
            b.add_tags(owner, &ct.tags);
            for bd in ct.bindings {
                bindings.push((bd, fi));
            }
        }
        for (line, name) in &e.src_env {
            let owner = e
                .facts
                .def_of_line(*line)
                .map(|d| def_ids[fi][d])
                .unwrap_or(fid);
            b.add_tags(owner, &["env".to_string(), format!("env:{name}")]);
        }
    }

    // 6. Imports --------------------------------------------------------------
    let file_set: HashSet<&str> = entries.iter().map(|e| e.rel.as_str()).collect();
    let mut go_dirs: HashMap<String, Vec<String>> = HashMap::new();
    for e in &entries {
        if e.lang == Lang::Go && !e.rel.ends_with("_test.go") {
            let dir = Path::new(&e.rel)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            go_dirs.entry(dir).or_default().push(e.rel.clone());
        }
    }
    // file idx -> list of (imported file idx or external id, imported names)
    let mut import_targets: Vec<Vec<(NodeId, Vec<String>)>> = vec![Vec::new(); entries.len()];
    {
        let _s = info_span!("resolve_imports").entered();
        for (fi, e) in entries.iter().enumerate() {
            let fid = file_ids[&e.rel];
            for imp in &e.facts.imports {
                // Python `from pkg import sub` may name submodules rather than symbols:
                // resolve each name as a module first and keep the rest for the package.
                let mut names = imp.names.clone();
                if e.lang == Lang::Python && !names.is_empty() {
                    names.retain(|n| {
                        let sub = format!("{}.{n}", imp.module);
                        match resolve_import(
                            &root, &e.rel, e.lang, &sub, &packages, &file_set, &go_dirs,
                        ) {
                            Resolved::Files(targets) => {
                                for t in targets {
                                    if let Some(&tid) = file_ids.get(&t)
                                        && tid != fid
                                    {
                                        b.edge(fid, tid, EdgeKind::Imports, Some(n.clone()));
                                        import_targets[fi].push((tid, vec![n.clone()]));
                                    }
                                }
                                false
                            }
                            _ => true,
                        }
                    });
                    if names.is_empty() {
                        continue;
                    }
                }
                let imp = &lang::Import {
                    module: imp.module.clone(),
                    names,
                    line: imp.line,
                };
                let resolved = resolve_import(
                    &root,
                    &e.rel,
                    e.lang,
                    &imp.module,
                    &packages,
                    &file_set,
                    &go_dirs,
                );
                match resolved {
                    Resolved::Files(targets) => {
                        for t in targets {
                            if let Some(&tid) = file_ids.get(&t)
                                && tid != fid
                            {
                                let label = if imp.names.is_empty() {
                                    None
                                } else {
                                    Some(imp.names.join(", "))
                                };
                                b.edge(fid, tid, EdgeKind::Imports, label);
                                import_targets[fi].push((tid, imp.names.clone()));
                            }
                        }
                    }
                    Resolved::External(name) => {
                        if opts.include_external {
                            let xid = b.external(&name, e.lang, repo_id);
                            b.edge(
                                fid,
                                xid,
                                EdgeKind::Imports,
                                if imp.names.is_empty() {
                                    None
                                } else {
                                    Some(imp.names.join(", "))
                                },
                            );
                        }
                    }
                    Resolved::None => {}
                }
            }
        }
    }

    // 7. Calls ----------------------------------------------------------------
    let mut unresolved = 0u32;
    {
        let _s = info_span!("resolve_calls").entered();
        // symbol lookup per file: leaf name -> ids ; full name -> id
        let mut by_file: HashMap<NodeId, HashMap<String, Vec<NodeId>>> = HashMap::new();
        for (fi, e) in entries.iter().enumerate() {
            let fid = file_ids[&e.rel];
            let map = by_file.entry(fid).or_default();
            for (di, d) in e.facts.defs.iter().enumerate() {
                let sid = def_ids[fi][di];
                map.entry(d.name.clone()).or_default().push(sid);
                let leaf = d
                    .name
                    .rsplit(['.', ':'])
                    .next()
                    .unwrap_or(&d.name)
                    .to_string();
                if leaf != d.name {
                    map.entry(leaf).or_default().push(sid);
                }
            }
        }
        for (fi, e) in entries.iter().enumerate() {
            let fid = file_ids[&e.rel];
            for c in &e.facts.calls {
                if is_builtin(&c.callee, e.lang) {
                    continue;
                }
                let from = c.def.map(|d| def_ids[fi][d]).unwrap_or(fid);
                let callee = c.callee.trim_end_matches('!');
                let leaf = c.leaf.trim_end_matches('!');
                let mut target: Option<NodeId> = None;
                // (a) same file: full name, then `X.leaf` / `X::leaf` method forms, then leaf
                if let Some(map) = by_file.get(&fid) {
                    target = map.get(callee).and_then(|v| v.first().copied());
                    if target.is_none() {
                        let stripped = callee
                            .trim_start_matches("self.")
                            .trim_start_matches("this.")
                            .trim_start_matches("Self::");
                        target = map.get(stripped).and_then(|v| v.first().copied());
                    }
                    if target.is_none() {
                        target = map.get(leaf).and_then(|v| v.first().copied());
                    }
                }
                // (b) imported files: prefer imports that name the callee's head or leaf
                if target.is_none() {
                    let head = callee.split(['.', ':']).next().unwrap_or(callee);
                    let mut fallback: Option<NodeId> = None;
                    for (tid, names) in &import_targets[fi] {
                        let Some(map) = by_file.get(tid) else {
                            continue;
                        };
                        let named = names.iter().any(|n| n == head || n == leaf);
                        let hit = map
                            .get(leaf)
                            .or_else(|| map.get(callee))
                            .and_then(|v| v.first().copied());
                        if let Some(h) = hit {
                            if named || names.is_empty() {
                                target = Some(h);
                                break;
                            }
                            fallback.get_or_insert(h);
                        }
                    }
                    if target.is_none() {
                        target = fallback;
                    }
                }
                match target {
                    Some(t) if t != from => {
                        b.edge(from, t, EdgeKind::Calls, Some(leaf.to_string()))
                    }
                    Some(_) => {}
                    None => unresolved += 1,
                }
            }
        }
    }

    // 8. Flows ----------------------------------------------------------------
    {
        let _s = info_span!("derive_flows").entered();
        // resolve binding handlers to symbol ids
        let mut servers: Vec<(String, NodeId)> = Vec::new();
        for (bd, fi) in &bindings {
            let e = &entries[*fi];
            let fid = file_ids[&e.rel];
            let handler_id =
                bd.handler
                    .as_ref()
                    .and_then(|h| {
                        let h = h.trim_start_matches("Self::").trim_start_matches("self.");
                        e.facts.defs.iter().position(|d| {
                            d.name == h || d.name.rsplit(['.', ':']).next() == Some(h)
                        })
                    })
                    .or(bd.def)
                    .map(|di| def_ids[*fi][di])
                    .unwrap_or(fid);
            b.add_tags(
                handler_id,
                &["http-server".to_string(), bd.key.clone()]
                    .into_iter()
                    .filter(|t| !bd.key.starts_with("ipc") || t != "http-server")
                    .collect::<Vec<_>>(),
            );
            if bd.key.starts_with("ipc:") {
                b.add_tags(handler_id, &["ipc-server".to_string()]);
            }
            servers.push((bd.key.clone(), handler_id));
        }
        servers.sort();
        servers.dedup();
        // clients
        let client_nodes: Vec<(NodeId, Vec<String>)> = b
            .nodes
            .iter()
            .filter(|n| {
                n.tags
                    .iter()
                    .any(|t| t == "http-client" || t == "ipc-client")
            })
            .map(|n| (n.id, n.tags.clone()))
            .collect();
        for (cid, ctags) in client_nodes {
            for t in &ctags {
                if let Some(route) = t.strip_prefix("route:") {
                    for (key, sid) in &servers {
                        if let Some(sroute) = key.strip_prefix("route:")
                            && *sid != cid
                            && tags::routes_match(route, sroute)
                        {
                            b.edge(cid, *sid, EdgeKind::Flow, Some(format!("http {sroute}")));
                        }
                    }
                } else if t.starts_with("ipc:") {
                    for (key, sid) in &servers {
                        if key == t && *sid != cid {
                            b.edge(
                                cid,
                                *sid,
                                EdgeKind::Flow,
                                Some(format!("ipc {}", t.trim_start_matches("ipc:"))),
                            );
                        }
                    }
                }
            }
        }
        // queues: producers -> consumers sharing a topic
        let topic_nodes: Vec<(NodeId, String, bool)> = b
            .nodes
            .iter()
            .filter(|n| n.tags.iter().any(|t| t == "queue"))
            .flat_map(|n| {
                let is_producer = n.tags.iter().any(|t| t == "queue-producer");
                n.tags
                    .iter()
                    .filter_map(|t| {
                        t.strip_prefix("topic:")
                            .map(|x| (n.id, x.to_string(), is_producer))
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        for (pid, ptopic, pprod) in &topic_nodes {
            if !pprod {
                continue;
            }
            for (cid, ctopic, cprod) in &topic_nodes {
                if !cprod && ctopic == ptopic && cid != pid {
                    b.edge(*pid, *cid, EdgeKind::Flow, Some(format!("queue {ptopic}")));
                }
            }
        }
    }

    // 9. Stats + finish -------------------------------------------------------
    let mut graph = b.finish(unresolved);
    graph.stats.scan_ms = started.elapsed().as_millis() as u64;
    graph.scanned_at = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    info!(
        nodes = graph.nodes.len(),
        edges = graph.edges.len(),
        ms = graph.stats.scan_ms,
        "scan complete"
    );
    Ok(graph)
}

// ---- builder -------------------------------------------------------------

struct Builder {
    root: String,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    edge_index: HashMap<(NodeId, NodeId, EdgeKind, Option<String>), usize>,
    /// One shared node per (name, language): every crate that imports `serde` points at the same node.
    externals: HashMap<(String, Lang), NodeId>,
}

impl Builder {
    fn new(root: &Path) -> Self {
        Self {
            root: root.to_string_lossy().to_string(),
            nodes: vec![],
            edges: vec![],
            edge_index: HashMap::new(),
            externals: HashMap::new(),
        }
    }
    fn push(&mut self, mut n: Node) -> NodeId {
        n.id = self.nodes.len() as NodeId;
        self.nodes.push(n);
        self.nodes.len() as NodeId - 1
    }
    fn edge(&mut self, from: NodeId, to: NodeId, kind: EdgeKind, label: Option<String>) {
        let key = (from, to, kind, label.clone());
        if let Some(&i) = self.edge_index.get(&key) {
            self.edges[i].weight += 1;
            return;
        }
        let id = self.edges.len() as EdgeId;
        self.edges.push(Edge {
            id,
            from,
            to,
            kind,
            label,
            weight: 1,
        });
        self.edge_index.insert(key, id as usize);
    }
    fn add_tags(&mut self, id: NodeId, tags: &[String]) {
        let n = &mut self.nodes[id as usize];
        for t in tags {
            if !n.tags.contains(t) {
                n.tags.push(t.clone());
            }
        }
    }
    fn external(&mut self, name: &str, lang: Lang, parent: NodeId) -> NodeId {
        let key = (name.to_string(), lang);
        if let Some(&id) = self.externals.get(&key) {
            return id;
        }
        let id = self.push(Node {
            id: 0,
            kind: NodeKind::Package,
            name: name.to_string(),
            path: format!("external:{name}"),
            lang,
            parent: Some(parent),
            loc: 0,
            symbol_kind: None,
            span: None,
            tags: vec!["external".into()],
            external: true,
        });
        self.edge(parent, id, EdgeKind::Contains, None);
        self.externals.insert(key, id);
        id
    }
    fn finish(mut self, unresolved: u32) -> Graph {
        // propagate loc up from files to packages
        let file_locs: Vec<(Option<NodeId>, u32)> = self
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::File)
            .map(|n| (n.parent, n.loc))
            .collect();
        for (parent, loc) in file_locs {
            let mut cur = parent;
            while let Some(p) = cur {
                self.nodes[p as usize].loc += loc;
                cur = self.nodes[p as usize].parent;
            }
        }
        let mut stats = Stats::default();
        let mut by_lang: HashMap<&'static str, LangStat> = HashMap::new();
        for n in &self.nodes {
            match n.kind {
                NodeKind::File => {
                    stats.files += 1;
                    stats.loc += n.loc as u64;
                    let s = by_lang.entry(n.lang.label()).or_insert_with(|| LangStat {
                        lang: n.lang.label().into(),
                        ..Default::default()
                    });
                    s.files += 1;
                    s.loc += n.loc as u64;
                }
                NodeKind::Symbol => stats.symbols += 1,
                NodeKind::Package => {
                    if n.external {
                        stats.external_packages += 1
                    } else {
                        stats.packages += 1
                    }
                }
                NodeKind::Repo => {}
            }
        }
        for e in &self.edges {
            match e.kind {
                EdgeKind::Imports => stats.imports += e.weight,
                EdgeKind::Calls => stats.calls += e.weight,
                EdgeKind::Flow => stats.flows += 1,
                EdgeKind::Contains => {}
            }
        }
        stats.unresolved_calls = unresolved;
        let mut langs: Vec<LangStat> = by_lang.into_values().collect();
        langs.sort_by_key(|l| std::cmp::Reverse(l.loc));
        stats.by_lang = langs;
        Graph {
            schema: SCHEMA_VERSION,
            root: self.root,
            scanned_at: String::new(),
            nodes: self.nodes,
            edges: self.edges,
            stats,
        }
    }
}

// ---- walking -------------------------------------------------------------

fn walk_files(root: &Path, opts: &ScanOptions) -> (Vec<String>, Vec<String>) {
    let mut files = Vec::new();
    let mut manifests = Vec::new();
    let skip: HashSet<String> = opts.skip_dirs.iter().cloned().collect();
    let walker = WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .follow_links(false)
        .filter_entry(move |e| {
            let name = e.file_name().to_string_lossy();
            !(e.file_type().map(|t| t.is_dir()).unwrap_or(false) && skip.contains(name.as_ref()))
        })
        .build();
    for entry in walker.flatten() {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let rel = match entry.path().strip_prefix(root) {
            Ok(r) => r.to_string_lossy().to_string(),
            Err(_) => continue,
        };
        let name = entry.file_name().to_string_lossy();
        match name.as_ref() {
            "Cargo.toml" | "package.json" | "go.mod" | "pyproject.toml" | "setup.py"
            | "requirements.txt" => manifests.push(rel.clone()),
            _ => {}
        }
        if Lang::from_path(&rel) != Lang::Other && !name.ends_with(".d.ts") {
            files.push(rel);
        }
    }
    files.sort();
    manifests.sort();
    (files, manifests)
}

fn looks_minified(bytes: &[u8]) -> bool {
    if bytes.len() < 4096 {
        return false;
    }
    let lines = bytes.iter().filter(|b| **b == b'\n').count().max(1);
    bytes.len() / lines > 400
}

fn detect_packages(root: &Path, manifests: &[String]) -> Vec<PackageInfo> {
    let mut seen: HashMap<PathBuf, PackageInfo> = HashMap::new();
    for m in manifests {
        let p = Path::new(m);
        let dir = p.parent().unwrap_or(Path::new("")).to_path_buf();
        let file = p.file_name().unwrap().to_string_lossy().to_string();
        let content = std::fs::read_to_string(root.join(m)).unwrap_or_default();
        let fallback = dir
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| {
                root.file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default()
            });
        let (kind, name, go_module): (&'static str, String, Option<String>) = match file.as_str() {
            "Cargo.toml" => {
                if !content.contains("[package]") {
                    // workspace-only manifest: not a package
                    continue;
                }
                ("cargo", toml_name(&content).unwrap_or(fallback), None)
            }
            "package.json" => ("npm", json_name(&content).unwrap_or(fallback), None),
            "go.mod" => {
                let module = content.lines().find_map(|l| {
                    l.trim()
                        .strip_prefix("module ")
                        .map(|s| s.trim().to_string())
                });
                let name = module
                    .as_ref()
                    .and_then(|m| m.rsplit('/').next().map(String::from))
                    .unwrap_or(fallback);
                ("go", name, module)
            }
            "pyproject.toml" => ("python", toml_name(&content).unwrap_or(fallback), None),
            "setup.py" | "requirements.txt" => ("python", fallback, None),
            _ => continue,
        };
        // Prefer the more specific manifest when several sit in one directory.
        let rank = |k: &str| match k {
            "cargo" => 4,
            "go" => 3,
            "python" => 2,
            "npm" => 1,
            _ => 0,
        };
        match seen.get(&dir) {
            Some(existing) if rank(existing.kind) >= rank(kind) => {}
            _ => {
                seen.insert(
                    dir.clone(),
                    PackageInfo {
                        dir,
                        name,
                        kind,
                        go_module,
                    },
                );
            }
        }
    }
    seen.into_values().collect()
}

fn toml_name(content: &str) -> Option<String> {
    let mut in_pkg = false;
    for l in content.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            in_pkg = t == "[package]" || t == "[project]" || t == "[tool.poetry]";
            continue;
        }
        if in_pkg && let Some(rest) = t.strip_prefix("name") {
            let rest = rest.trim().strip_prefix('=')?.trim();
            return Some(rest.trim_matches(|c| c == '"' || c == '\'').to_string());
        }
    }
    None
}

fn json_name(content: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(content).ok()?;
    v.get("name")?.as_str().map(String::from)
}

fn nearest_package<'a>(packages: &'a [PackageInfo], dir: &Path) -> Option<&'a PackageInfo> {
    // packages sorted deepest-first
    packages.iter().find(|p| dir.starts_with(&p.dir))
}

// ---- import resolution ---------------------------------------------------

enum Resolved {
    Files(Vec<String>),
    External(String),
    None,
}

fn exists(file_set: &HashSet<&str>, p: &Path) -> Option<String> {
    let s = normalize(p);
    file_set.contains(s.as_str()).then_some(s)
}

fn normalize(p: &Path) -> String {
    let mut out: Vec<String> = Vec::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            std::path::Component::Normal(s) => out.push(s.to_string_lossy().to_string()),
            _ => {}
        }
    }
    out.join("/")
}

fn resolve_import(
    root: &Path,
    file: &str,
    lang: Lang,
    module: &str,
    packages: &[PackageInfo],
    file_set: &HashSet<&str>,
    go_dirs: &HashMap<String, Vec<String>>,
) -> Resolved {
    let dir = Path::new(file)
        .parent()
        .unwrap_or(Path::new(""))
        .to_path_buf();
    let pkg = nearest_package(packages, &dir);
    match lang {
        Lang::TypeScript | Lang::JavaScript => {
            let exts = ["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts"];
            let try_base = |base: PathBuf| -> Option<String> {
                if let Some(f) = exists(file_set, &base)
                    && !base.is_dir()
                {
                    return Some(f);
                }
                let stem = base.to_string_lossy().to_string();
                let stem_noext = stem
                    .strip_suffix(".js")
                    .or(stem.strip_suffix(".jsx"))
                    .or(stem.strip_suffix(".mjs"))
                    .unwrap_or(&stem)
                    .to_string();
                for e in exts {
                    if let Some(f) = exists(file_set, Path::new(&format!("{stem_noext}.{e}"))) {
                        return Some(f);
                    }
                }
                for e in exts {
                    if let Some(f) = exists(file_set, Path::new(&format!("{stem}/index.{e}"))) {
                        return Some(f);
                    }
                }
                None
            };
            if module.starts_with('.') {
                return match try_base(dir.join(module)) {
                    Some(f) => Resolved::Files(vec![f]),
                    None => Resolved::None,
                };
            }
            if let Some(rest) = module
                .strip_prefix("@/")
                .or(module.strip_prefix("~/"))
                .or(module.strip_prefix("src/"))
            {
                let pkgdir = pkg.map(|p| p.dir.clone()).unwrap_or_default();
                for base in [
                    pkgdir.join("src").join(rest),
                    pkgdir.join(rest),
                    Path::new("src").join(rest),
                ] {
                    if let Some(f) = try_base(base) {
                        return Resolved::Files(vec![f]);
                    }
                }
                return Resolved::None;
            }
            // workspace package by name?
            let head = if module.starts_with('@') {
                module.splitn(3, '/').take(2).collect::<Vec<_>>().join("/")
            } else {
                module.split('/').next().unwrap_or(module).to_string()
            };
            if let Some(p) = packages.iter().find(|p| p.kind == "npm" && p.name == head) {
                for base in [
                    p.dir.join("src/index"),
                    p.dir.join("index"),
                    p.dir.join("src/main"),
                ] {
                    if let Some(f) = try_base(base) {
                        return Resolved::Files(vec![f]);
                    }
                }
            }
            Resolved::External(head)
        }
        Lang::Python => {
            let dots = module.chars().take_while(|c| *c == '.').count();
            let rest = &module[dots..];
            let parts: Vec<&str> = rest.split('.').filter(|s| !s.is_empty()).collect();
            let mut bases: Vec<PathBuf> = Vec::new();
            if dots > 0 {
                let mut base = dir.clone();
                for _ in 1..dots {
                    base = base.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                }
                bases.push(base);
            } else {
                // package roots: repo root, package dir, src/, and every ancestor of the file's dir
                bases.push(PathBuf::new());
                if let Some(p) = pkg {
                    bases.push(p.dir.clone());
                    bases.push(p.dir.join("src"));
                }
                bases.push(PathBuf::from("src"));
                let mut d = Some(dir.clone());
                while let Some(cur) = d {
                    bases.push(cur.clone());
                    d = cur.parent().map(|p| p.to_path_buf());
                    if cur.as_os_str().is_empty() {
                        break;
                    }
                }
            }
            for base in bases {
                // longest first: a/b/c.py, a/b/c/__init__.py, then a/b.py (c is a symbol)
                for take in (1..=parts.len()).rev() {
                    let mut p = base.clone();
                    for s in &parts[..take] {
                        p = p.join(s);
                    }
                    if let Some(f) = exists(file_set, &p.with_extension("py")) {
                        return Resolved::Files(vec![f]);
                    }
                    if let Some(f) = exists(file_set, &p.join("__init__.py")) {
                        return Resolved::Files(vec![f]);
                    }
                }
                if parts.is_empty()
                    && dots > 0
                    && let Some(f) = exists(file_set, &base.join("__init__.py"))
                {
                    return Resolved::Files(vec![f]);
                }
            }
            if dots > 0 {
                return Resolved::None;
            }
            Resolved::External(
                parts
                    .first()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| module.to_string()),
            )
        }
        Lang::Rust => {
            let file_name = Path::new(file)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let is_root_like = matches!(file_name.as_str(), "mod.rs" | "lib.rs" | "main.rs")
                || dir.ends_with("bin");
            // module directory of this file: `foo.rs` owns `foo/`, `mod.rs`/`lib.rs`/`main.rs` own their dir
            let mod_dir = if is_root_like {
                dir.clone()
            } else {
                dir.join(file_name.trim_end_matches(".rs"))
            };
            if let Some(m) = module.strip_prefix("mod:") {
                for cand in [
                    mod_dir.join(format!("{m}.rs")),
                    mod_dir.join(m).join("mod.rs"),
                ] {
                    if let Some(f) = exists(file_set, &cand) {
                        return Resolved::Files(vec![f]);
                    }
                }
                return Resolved::None;
            }
            let parts: Vec<&str> = module.split("::").collect();
            let (base, rest): (PathBuf, &[&str]) = match parts.first().copied() {
                Some("crate") => (crate_src_dir(root, pkg, &dir), &parts[1..]),
                Some("self") => (mod_dir.clone(), &parts[1..]),
                Some("super") => {
                    let mut b = mod_dir
                        .parent()
                        .map(|p| p.to_path_buf())
                        .unwrap_or_default();
                    let mut i = 1;
                    while parts.get(i) == Some(&"super") {
                        b = b.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                        i += 1;
                    }
                    (b, &parts[i..])
                }
                Some(head) => {
                    let norm = head.replace('-', "_");
                    if let Some(p) = packages
                        .iter()
                        .find(|p| p.kind == "cargo" && p.name.replace('-', "_") == norm)
                    {
                        (p.dir.join("src"), &parts[1..])
                    } else if matches!(head, "std" | "core" | "alloc") {
                        return Resolved::External("std".into());
                    } else {
                        return Resolved::External(head.to_string());
                    }
                }
                None => return Resolved::None,
            };
            // longest prefix that is a file
            for take in (0..=rest.len()).rev() {
                let mut p = base.clone();
                for s in &rest[..take] {
                    p = p.join(s);
                }
                if take > 0
                    && let Some(f) = exists(file_set, &p.with_extension("rs"))
                {
                    return Resolved::Files(vec![f]);
                }
                for name in ["mod.rs", "lib.rs", "main.rs"] {
                    if let Some(f) = exists(file_set, &p.join(name)) {
                        return Resolved::Files(vec![f]);
                    }
                }
            }
            Resolved::None
        }
        Lang::Go => {
            if let Some(p) = pkg
                && let Some(m) = &p.go_module
                && (module == m || module.starts_with(&format!("{m}/")))
            {
                let rest = module[m.len()..].trim_start_matches('/');
                let target_dir = normalize(&p.dir.join(rest));
                return match go_dirs.get(&target_dir) {
                    Some(files) => Resolved::Files(files.iter().take(8).cloned().collect()),
                    None => Resolved::None,
                };
            }
            // any other module in the repo (multi-module repos)
            for p in packages.iter().filter(|p| p.kind == "go") {
                if let Some(m) = &p.go_module
                    && module.starts_with(&format!("{m}/"))
                {
                    let rest = &module[m.len() + 1..];
                    if let Some(files) = go_dirs.get(&normalize(&p.dir.join(rest))) {
                        return Resolved::Files(files.iter().take(8).cloned().collect());
                    }
                }
            }
            let name = if module.contains('.') {
                module.splitn(4, '/').take(3).collect::<Vec<_>>().join("/")
            } else {
                module.to_string()
            };
            Resolved::External(name)
        }
        Lang::Other => Resolved::None,
    }
}

fn crate_src_dir(root: &Path, pkg: Option<&PackageInfo>, dir: &Path) -> PathBuf {
    if let Some(p) = pkg.filter(|p| p.kind == "cargo") {
        let src = p.dir.join("src");
        if root.join(&src).is_dir() {
            return src;
        }
        return p.dir.clone();
    }
    // walk up until a `src` directory
    let mut d = Some(dir.to_path_buf());
    while let Some(cur) = d {
        if cur.file_name().map(|n| n == "src").unwrap_or(false) {
            return cur;
        }
        d = cur.parent().map(|p| p.to_path_buf());
    }
    PathBuf::from("src")
}

fn is_builtin(callee: &str, lang: Lang) -> bool {
    let c = callee.trim_end_matches('!');
    match lang {
        Lang::Rust => {
            matches!(
                c,
                "println"
                    | "eprintln"
                    | "print"
                    | "eprint"
                    | "format"
                    | "vec"
                    | "assert"
                    | "assert_eq"
                    | "assert_ne"
                    | "debug_assert"
                    | "panic"
                    | "todo"
                    | "unimplemented"
                    | "unreachable"
                    | "matches"
                    | "write"
                    | "writeln"
                    | "Some"
                    | "Ok"
                    | "Err"
                    | "Box::new"
                    | "String::from"
                    | "Vec::new"
                    | "Vec::with_capacity"
                    | "HashMap::new"
                    | "HashSet::new"
                    | "Default::default"
                    | "drop"
                    | "dbg"
                    | "include_str"
                    | "include_bytes"
                    | "cfg"
                    | "concat"
                    | "stringify"
                    | "env"
                    | "option_env"
                    | "thread_local"
                    | "info"
                    | "warn"
                    | "error"
                    | "debug"
                    | "trace"
            ) || c.starts_with("Some(")
                || c.ends_with(".clone")
                || c.ends_with(".to_string")
                || c.ends_with(".unwrap")
                || c.ends_with(".into")
                || c.ends_with(".len")
                || c.ends_with(".iter")
                || c.ends_with(".collect")
                || c.ends_with(".map")
                || c.ends_with(".as_ref")
                || c.ends_with(".to_owned")
                || c.ends_with(".push")
                || c.ends_with(".get")
                || c.ends_with(".expect")
                || c.ends_with(".as_str")
                || c.ends_with(".is_empty")
                || c.ends_with(".unwrap_or")
                || c.ends_with(".unwrap_or_default")
                || c.ends_with(".to_vec")
                || c.ends_with(".contains")
                || c.ends_with(".join")
                || c.ends_with(".starts_with")
                || c.ends_with(".ends_with")
                || c.ends_with(".trim")
                || c.ends_with(".lock")
                || c.ends_with(".await")
                || c.ends_with(".and_then")
                || c.ends_with(".ok")
                || c.ends_with(".insert")
                || c.ends_with(".filter")
                || c.ends_with(".find")
                || c.ends_with(".sort")
                || c.ends_with(".cloned")
                || c.ends_with(".copied")
                || c.ends_with(".unwrap_or_else")
                || c.ends_with(".map_err")
                || c.ends_with(".ok_or")
                || c.ends_with(".unwrap_err")
                || c.ends_with(".entry")
        }
        Lang::TypeScript | Lang::JavaScript => {
            matches!(
                c,
                "console.log"
                    | "console.error"
                    | "console.warn"
                    | "console.info"
                    | "console.debug"
                    | "parseInt"
                    | "parseFloat"
                    | "String"
                    | "Number"
                    | "Boolean"
                    | "Array.isArray"
                    | "Object.keys"
                    | "Object.values"
                    | "Object.entries"
                    | "Object.assign"
                    | "Object.freeze"
                    | "JSON.parse"
                    | "JSON.stringify"
                    | "Math.max"
                    | "Math.min"
                    | "Math.floor"
                    | "Math.ceil"
                    | "Math.round"
                    | "Math.abs"
                    | "Math.sqrt"
                    | "Math.random"
                    | "Math.hypot"
                    | "setTimeout"
                    | "setInterval"
                    | "clearTimeout"
                    | "clearInterval"
                    | "requestAnimationFrame"
                    | "cancelAnimationFrame"
                    | "Promise.all"
                    | "Promise.resolve"
                    | "Promise.reject"
                    | "Error"
                    | "Date.now"
                    | "performance.now"
                    | "Symbol"
                    | "Map"
                    | "Set"
                    | "Array.from"
                    | "isNaN"
                    | "document.getElementById"
                    | "document.querySelector"
                    | "document.querySelectorAll"
                    | "document.createElement"
                    | "Number.isFinite"
                    | "structuredClone"
                    | "encodeURIComponent"
                    | "decodeURIComponent"
                    | "btoa"
                    | "atob"
            ) || c.ends_with(".push")
                || c.ends_with(".map")
                || c.ends_with(".filter")
                || c.ends_with(".forEach")
                || c.ends_with(".reduce")
                || c.ends_with(".find")
                || c.ends_with(".some")
                || c.ends_with(".every")
                || c.ends_with(".join")
                || c.ends_with(".split")
                || c.ends_with(".slice")
                || c.ends_with(".splice")
                || c.ends_with(".indexOf")
                || c.ends_with(".includes")
                || c.ends_with(".toString")
                || c.ends_with(".toFixed")
                || c.ends_with(".trim")
                || c.ends_with(".toLowerCase")
                || c.ends_with(".toUpperCase")
                || c.ends_with(".get")
                || c.ends_with(".set")
                || c.ends_with(".has")
                || c.ends_with(".delete")
                || c.ends_with(".add")
                || c.ends_with(".keys")
                || c.ends_with(".values")
                || c.ends_with(".entries")
                || c.ends_with(".sort")
                || c.ends_with(".concat")
                || c.ends_with(".addEventListener")
                || c.ends_with(".removeEventListener")
                || c.ends_with(".appendChild")
                || c.ends_with(".querySelector")
                || c.ends_with(".querySelectorAll")
                || c.ends_with(".setAttribute")
                || c.ends_with(".classList.add")
                || c.ends_with(".classList.remove")
                || c.ends_with(".classList.toggle")
                || c.ends_with(".then")
                || c.ends_with(".catch")
                || c.ends_with(".finally")
                || c.ends_with(".bind")
                || c.ends_with(".call")
                || c.ends_with(".apply")
                || c.ends_with(".startsWith")
                || c.ends_with(".endsWith")
                || c.ends_with(".replace")
                || c.ends_with(".padStart")
                || c.ends_with(".padEnd")
                || c.ends_with(".fill")
                || c.ends_with(".flat")
                || c.ends_with(".flatMap")
                || c.ends_with(".at")
                || c.ends_with(".pop")
                || c.ends_with(".shift")
                || c.ends_with(".unshift")
                || c.ends_with(".reverse")
                || c.ends_with(".localeCompare")
                || c.ends_with(".charCodeAt")
                || c.ends_with(".substring")
                || c.ends_with(".preventDefault")
                || c.ends_with(".stopPropagation")
                || c.ends_with(".focus")
                || c.ends_with(".blur")
                || c.ends_with(".remove")
                || c.ends_with(".getBoundingClientRect")
                || c.ends_with(".closest")
                || c.ends_with(".matches")
                || c.ends_with(".clear")
                || c.ends_with(".now")
                || c.starts_with("gl.")
                || c.starts_with("ctx.")
                || c.starts_with("Math.")
                || c.starts_with("console.")
        }
        Lang::Python => {
            matches!(
                c,
                "print"
                    | "len"
                    | "range"
                    | "str"
                    | "int"
                    | "float"
                    | "list"
                    | "dict"
                    | "set"
                    | "tuple"
                    | "isinstance"
                    | "enumerate"
                    | "zip"
                    | "map"
                    | "filter"
                    | "sorted"
                    | "reversed"
                    | "min"
                    | "max"
                    | "sum"
                    | "abs"
                    | "any"
                    | "all"
                    | "open"
                    | "iter"
                    | "next"
                    | "getattr"
                    | "setattr"
                    | "hasattr"
                    | "super"
                    | "type"
                    | "id"
                    | "hash"
                    | "repr"
                    | "format"
                    | "input"
                    | "round"
                    | "bool"
                    | "bytes"
                    | "ord"
                    | "chr"
                    | "vars"
                    | "dir"
                    | "property"
                    | "staticmethod"
                    | "classmethod"
                    | "Exception"
                    | "ValueError"
                    | "TypeError"
                    | "KeyError"
                    | "RuntimeError"
                    | "frozenset"
                    | "object"
                    | "dataclass"
                    | "field"
                    | "logging.getLogger"
                    | "logger.info"
                    | "logger.debug"
                    | "logger.warning"
                    | "logger.error"
                    | "logger.exception"
            ) || c.ends_with(".append")
                || c.ends_with(".extend")
                || c.ends_with(".get")
                || c.ends_with(".items")
                || c.ends_with(".keys")
                || c.ends_with(".values")
                || c.ends_with(".join")
                || c.ends_with(".split")
                || c.ends_with(".strip")
                || c.ends_with(".format")
                || c.ends_with(".update")
                || c.ends_with(".pop")
                || c.ends_with(".lower")
                || c.ends_with(".upper")
                || c.ends_with(".startswith")
                || c.ends_with(".endswith")
                || c.ends_with(".replace")
                || c.ends_with(".encode")
                || c.ends_with(".decode")
                || c.ends_with(".add")
                || c.ends_with(".remove")
                || c.ends_with(".sort")
                || c.ends_with(".copy")
                || c.ends_with(".setdefault")
                || c.ends_with(".index")
                || c.ends_with(".count")
                || c.ends_with(".isoformat")
                || c.ends_with(".strftime")
                || c.ends_with(".dict")
                || c.ends_with(".json")
                || c.ends_with(".model_dump")
        }
        Lang::Go => {
            matches!(
                c,
                "len"
                    | "cap"
                    | "make"
                    | "new"
                    | "append"
                    | "copy"
                    | "delete"
                    | "panic"
                    | "recover"
                    | "print"
                    | "println"
                    | "close"
                    | "string"
                    | "int"
                    | "int64"
                    | "float64"
                    | "byte"
                    | "error"
                    | "errors.New"
                    | "fmt.Println"
                    | "fmt.Printf"
                    | "fmt.Sprintf"
                    | "fmt.Errorf"
                    | "fmt.Fprintf"
                    | "fmt.Print"
                    | "log.Println"
                    | "log.Printf"
                    | "log.Fatal"
                    | "log.Fatalf"
                    | "log.Fatalln"
                    | "strings.Split"
                    | "strings.Join"
                    | "strings.TrimSpace"
                    | "strings.HasPrefix"
                    | "strings.HasSuffix"
                    | "strings.Contains"
                    | "strconv.Itoa"
                    | "strconv.Atoi"
                    | "time.Now"
                    | "time.Since"
                    | "context.Background"
                    | "context.WithCancel"
                    | "context.WithTimeout"
                    | "json.Marshal"
                    | "json.Unmarshal"
                    | "json.NewEncoder"
                    | "json.NewDecoder"
                    | "sync.WaitGroup"
                    | "defer"
            ) || c.ends_with(".Error")
                || c.ends_with(".String")
                || c.ends_with(".Lock")
                || c.ends_with(".Unlock")
                || c.ends_with(".Done")
                || c.ends_with(".Add")
                || c.ends_with(".Wait")
                || c.ends_with(".Close")
                || c.ends_with(".Write")
                || c.ends_with(".WriteHeader")
                || c.ends_with(".Header")
                || c.ends_with(".Encode")
                || c.ends_with(".Decode")
                || c.ends_with(".Err")
                || c.ends_with(".Context")
                || c.ends_with(".Printf")
                || c.ends_with(".Println")
        }
        Lang::Other => true,
    }
}

#[allow(dead_code)]
fn _unused(_: &Path) -> Option<()> {
    warn!("unused");
    None
}
