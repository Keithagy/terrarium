use std::path::PathBuf;
use terrarium_core::{EdgeKind, NodeKind, ScanOptions, query, scan};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/polyglot")
}

fn has_flow(g: &terrarium_core::Graph, from: &str, to: &str, label: &str) -> bool {
    g.edges.iter().any(|e| {
        e.kind == EdgeKind::Flow
            && g.node(e.from).path == from
            && g.node(e.to).path == to
            && e.label.as_deref() == Some(label)
    })
}

#[test]
fn scans_polyglot_fixture() {
    let g = scan(&fixture(), &ScanOptions::default()).unwrap();
    assert_eq!(
        g.stats.files,
        12,
        "files: {:?}",
        g.nodes
            .iter()
            .filter(|n| n.kind == NodeKind::File)
            .map(|n| &n.path)
            .collect::<Vec<_>>()
    );
    let langs: Vec<&str> = g.stats.by_lang.iter().map(|l| l.lang.as_str()).collect();
    for l in ["python", "typescript", "go", "rust"] {
        assert!(langs.contains(&l), "missing {l} in {langs:?}");
    }
    // packages detected from manifests
    let pkgs: Vec<&str> = g
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Package && !n.external)
        .map(|n| n.name.as_str())
        .collect();
    for p in [
        "polyglot-root",
        "polyglot-api",
        "polyglot-web",
        "worker",
        "polyglot-native",
    ] {
        assert!(pkgs.contains(&p), "missing package {p} in {pkgs:?}");
    }
}

#[test]
fn resolves_imports_per_language() {
    let g = scan(&fixture(), &ScanOptions::default()).unwrap();
    let imports: Vec<(String, String)> = g
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Imports)
        .map(|e| (g.node(e.from).path.clone(), g.node(e.to).path.clone()))
        .collect();
    let expect = [
        ("api/main.py", "api/services/users.py"),
        ("api/main.py", "api/services/billing.py"),
        ("web/src/app.ts", "web/src/api.ts"),
        ("web/src/app.ts", "web/src/view.ts"),
        ("web/src/api.ts", "external:@tauri-apps/api"),
        ("worker/main.go", "worker/store/store.go"),
        ("native/src/lib.rs", "native/src/commands.rs"),
        ("native/src/lib.rs", "native/src/jobs.rs"),
        ("native/src/commands.rs", "native/src/jobs.rs"),
    ];
    for (a, b) in expect {
        assert!(
            imports.iter().any(|(x, y)| x == a && y == b),
            "missing import {a} -> {b}\nhave: {imports:#?}"
        );
    }
}

#[test]
fn derives_cross_language_flows() {
    let g = scan(&fixture(), &ScanOptions::default()).unwrap();
    let flows = query::flows(&g);
    assert!(
        has_flow(
            &g,
            "web/src/api.ts#fetchUsers",
            "api/main.py#get_users",
            "http /api/users"
        ),
        "flows: {flows:#?}"
    );
    assert!(
        has_flow(
            &g,
            "web/src/api.ts#fetchUsers",
            "api/main.py#post_user",
            "http /api/users"
        ),
        "flows: {flows:#?}"
    );
    assert!(
        has_flow(
            &g,
            "web/src/api.ts#fetchUser",
            "api/main.py#get_user",
            "http /api/users/*"
        ),
        "flows: {flows:#?}"
    );
    assert!(
        has_flow(
            &g,
            "web/src/api.ts#scanRepo",
            "native/src/commands.rs#scan_repo",
            "ipc scan_repo"
        ),
        "flows: {flows:#?}"
    );
    assert!(
        has_flow(
            &g,
            "native/src/jobs.rs#Job::fetch_all",
            "worker/main.go#handleJobs",
            "http /api/jobs"
        ),
        "flows: {flows:#?}"
    );
}

#[test]
fn tags_boundaries() {
    let g = scan(&fixture(), &ScanOptions::default()).unwrap();
    let tags = |p: &str| {
        g.find_by_path(p)
            .map(|n| n.tags.clone())
            .unwrap_or_else(|| panic!("no node {p}"))
    };
    assert!(tags("api/services/users.py#list_users").contains(&"db".to_string()));
    assert!(tags("worker/store/store.go#ListJobs").contains(&"db".to_string()));
    assert!(tags("worker/store/store.go#open").contains(&"env:DATABASE_URL".to_string()));
    assert!(tags("api/main.py").contains(&"env:DATABASE_URL".to_string()));
    assert!(tags("native/src/lib.rs#run").contains(&"env:HOME".to_string()));
    assert!(tags("native/src/commands.rs#scan_repo").contains(&"fs".to_string()));
    assert!(tags("web/src/app.ts#main").contains(&"env:VITE_REPO_PATH".to_string()));
}

#[test]
fn resolves_calls_and_views() {
    let g = scan(&fixture(), &ScanOptions::default()).unwrap();
    let calls: Vec<(String, String)> = g
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Calls)
        .map(|e| (g.node(e.from).path.clone(), g.node(e.to).path.clone()))
        .collect();
    for (a, b) in [
        ("api/main.py#get_users", "api/services/users.py#list_users"),
        (
            "api/main.py#post_user",
            "api/services/billing.py#enqueue_welcome",
        ),
        ("web/src/app.ts#main", "web/src/api.ts#fetchUsers"),
        (
            "worker/main.go#handleJobs",
            "worker/store/store.go#ListJobs",
        ),
        (
            "native/src/jobs.rs#sync_jobs",
            "native/src/jobs.rs#Job::fetch_all",
        ),
        (
            "native/src/commands.rs#scan_repo",
            "native/src/jobs.rs#sync_jobs",
        ),
    ] {
        assert!(
            calls.iter().any(|(x, y)| x == a && y == b),
            "missing call {a} -> {b}\nhave: {calls:#?}"
        );
    }
    let v = g.view(NodeKind::File, None);
    assert!(
        v.nodes
            .iter()
            .all(|n| n.kind == NodeKind::File || n.external)
    );
    assert!(v.edges.iter().any(|e| e.kind == EdgeKind::Flow));
    let pv = g.view(NodeKind::Package, None);
    assert!(
        pv.edges.iter().any(|e| e.kind == EdgeKind::Flow),
        "package view should keep flows: {:?}",
        pv.edges
    );
    let hs = query::hotspots(&g, NodeKind::File, 3);
    assert_eq!(hs.len(), 3);
}

#[test]
fn one_external_node_per_dependency() {
    let g = scan(&fixture(), &ScanOptions::default()).unwrap();
    let mut seen = std::collections::HashSet::new();
    for n in g.nodes.iter().filter(|n| n.external) {
        assert!(
            seen.insert((n.name.clone(), n.lang)),
            "duplicate external node {} ({:?})",
            n.name,
            n.lang
        );
        assert_eq!(
            n.parent,
            Some(0),
            "externals hang off the repo, not the importing package"
        );
    }
    // Python's `os` and Go's `os` are different libraries and stay apart.
    let os: Vec<_> = g
        .nodes
        .iter()
        .filter(|n| n.external && n.name == "os")
        .collect();
    assert_eq!(os.len(), 2, "{os:?}");
    let v = g.view(NodeKind::File, None);
    assert!(
        v.nodes
            .iter()
            .filter(|n| n.external)
            .all(|n| n.group_name == "dependencies")
    );
}

#[test]
fn traces_stitch_calls_and_flows_across_languages() {
    let g = scan(&fixture(), &ScanOptions::default()).unwrap();
    let traces = query::traces(&g);
    let by_entry = |p: &str| traces.iter().find(|t| t.entry_path == p);

    // web main → scanRepo ─ipc→ rust scan_repo → sync_jobs → fetch_all ─http→ go handleJobs → ListJobs → open (db)
    let t = by_entry("web/src/app.ts#main").expect("trace from app.ts#main");
    assert_eq!(t.hops, 4, "{:#?}", t.steps.iter().map(|s| &s.path).collect::<Vec<_>>());
    assert_eq!(
        t.langs,
        vec![
            terrarium_core::Lang::TypeScript,
            terrarium_core::Lang::Python,
            terrarium_core::Lang::Rust,
            terrarium_core::Lang::Go
        ]
    );
    let paths: Vec<&str> = t.steps.iter().map(|s| s.path.as_str()).collect();
    assert!(!paths.contains(&"web/src/view.ts#render"), "branches without a flow or sink are pruned");
    let open = t.steps.iter().find(|s| s.path == "worker/store/store.go#open").expect("reaches the go store");
    assert!(open.sinks.contains(&"db".to_string()));
    let handle = t.steps.iter().find(|s| s.path == "worker/main.go#handleJobs").unwrap();
    assert_eq!(handle.via, Some(EdgeKind::Flow));
    assert_eq!(handle.label.as_deref(), Some("http /api/jobs"));
    // parents always come before children, so the list reads as a tree top to bottom
    for (i, s) in t.steps.iter().enumerate() {
        if let Some(p) = s.parent {
            assert!(p < i);
            assert_eq!(t.steps[p].depth + 1, s.depth);
        }
    }

    // an HTTP client nobody calls is its own entry
    assert!(by_entry("web/src/api.ts#fetchUser").is_some());
    // the rust entry point reaches go
    assert_eq!(by_entry("native/src/lib.rs#run").map(|t| t.hops), Some(1));
    // callees are never entries
    assert!(by_entry("web/src/api.ts#fetchUsers").is_none());
    assert!(traces.iter().all(|t| t.hops > 0));
}

#[test]
fn endpoints_report_callers_and_gaps() {
    let g = scan(&fixture(), &ScanOptions::default()).unwrap();
    let eps = query::endpoints(&g);
    let find = |k: &str| eps.iter().find(|e| e.key == k).unwrap_or_else(|| panic!("no endpoint {k}: {:?}", eps.iter().map(|e| &e.key).collect::<Vec<_>>()));
    let users = find("http /api/users");
    assert_eq!(users.status, "ok");
    assert_eq!(users.handlers.len(), 2, "GET and POST handlers share the route");
    assert_eq!(users.callers.len(), 1);
    let jobs = find("http /api/jobs");
    assert_eq!(jobs.handlers.iter().map(|h| h.path.as_str()).collect::<Vec<_>>(), vec!["worker/main.go#handleJobs"]);
    assert_eq!(find("ipc scan_repo").status, "ok");
    // fetchUser has no caller inside the repo, but the route itself is served
    assert_eq!(find("http /api/users/*").status, "ok");
    // a route nothing in the repo calls, and a call nothing in the repo serves
    let health = find("http /api/health");
    assert_eq!((health.status, health.callers.len()), ("no-callers", 0));
    let reports = find("http /api/reports");
    assert_eq!((reports.status, reports.handlers.len()), ("no-handler", 0));
    assert_eq!(reports.callers[0].path, "web/src/api.ts#fetchReport");
    // gaps sort first: they are what needs attention
    assert_ne!(eps[0].status, "ok");
}
