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
