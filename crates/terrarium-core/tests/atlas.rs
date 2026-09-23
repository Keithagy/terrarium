// The engine's atlas of the fixture, the check against the graph, and the export.
use std::path::PathBuf;
use terrarium_core::atlas::{self, Atlas, Component, Relationship};
use terrarium_core::{ScanOptions, scan};

fn fixture() -> terrarium_core::Graph {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/polyglot");
    scan(&root, &ScanOptions::default()).unwrap()
}

fn container<'a>(a: &'a Atlas, pkg: &str) -> &'a atlas::Container {
    a.containers.iter().find(|c| c.package == pkg).unwrap_or_else(|| panic!("no container for {pkg}"))
}

fn rel<'a>(a: &'a Atlas, from: &str, to: &str) -> &'a Relationship {
    a.relationships.iter().find(|r| r.from == from && r.to == to).unwrap_or_else(|| panic!("no relationship {from} -> {to} in {:#?}", a.relationships.iter().map(|r| format!("{} -> {}", r.from, r.to)).collect::<Vec<_>>()))
}

#[test]
fn the_engine_reads_containers_off_the_packages() {
    let g = fixture();
    let a = atlas::engine_atlas(&g);
    assert_eq!(a.source, "engine");
    assert_eq!(a.containers.len(), 5);
    assert_eq!(container(&a, "polyglot-web").kind, "web");
    assert_eq!(container(&a, "polyglot-api").kind, "service");
    assert_eq!(container(&a, "polyglot-native").kind, "desktop");
    assert_eq!(container(&a, "worker").kind, "service");
    assert!(container(&a, "polyglot-root").hidden, "an empty workspace root is tooling");
    assert_eq!(container(&a, "polyglot-api").technology, "Python, Celery, FastAPI");
    // components: a flat package gets one per file, a nested one gets one per directory
    let web: Vec<&str> = container(&a, "polyglot-web").components.iter().map(|k| k.name.as_str()).collect();
    assert_eq!(web, ["Api", "App", "View"]);
    let api: Vec<&str> = container(&a, "polyglot-api").components.iter().map(|k| k.name.as_str()).collect();
    assert_eq!(api, ["Core", "Services"]);
    // every file is in exactly one component
    let mut all: Vec<&str> = a.containers.iter().flat_map(|c| c.components.iter().flat_map(|k| k.files.iter().map(String::as_str))).collect();
    all.sort();
    assert_eq!(all.len(), 12);
    all.dedup();
    assert_eq!(all.len(), 12);
}

#[test]
fn relationships_come_from_the_code_with_evidence() {
    let g = fixture();
    let a = atlas::engine_atlas(&g);
    let r = rel(&a, "c:polyglot-web", "c:polyglot-api");
    assert_eq!(r.source, "code");
    assert_eq!(r.level, "container");
    assert!(r.label.contains("http /api/users"), "{}", r.label);
    assert_eq!(r.technology, "HTTP");
    assert!(r.evidence.iter().any(|e| e.from == "web/src/api.ts#fetchUsers" && e.via == "flow"));
    assert_eq!(rel(&a, "c:polyglot-web", "c:polyglot-native").technology, "IPC");
    // component level, inside a container
    let r = rel(&a, "c:polyglot-web/app", "c:polyglot-web/api");
    assert_eq!(r.level, "component");
    assert!(r.evidence.iter().any(|e| e.via == "imports" || e.via == "calls"));
    // the database is shared: both packages read DATABASE_URL
    let db = a.externals.iter().find(|x| x.kind == "database").unwrap();
    assert_eq!(db.name, "Database (DATABASE_URL)");
    assert_eq!(rel(&a, "c:polyglot-api", &db.id).source, "code");
    assert_eq!(rel(&a, "c:worker", &db.id).label, "reads and writes");
    assert!(a.externals.iter().any(|x| x.kind == "queue" && x.name.contains("REDIS_URL")));
    assert!(a.externals.iter().any(|x| x.kind == "filesystem"));
    // an HTTP call nothing serves points outside
    let out = a.externals.iter().find(|x| x.kind == "service").unwrap();
    assert!(rel(&a, "c:polyglot-web", &out.id).label.contains("/api/reports"));
    // a person uses what a person can run
    assert_eq!(a.people.len(), 1);
    assert_eq!(rel(&a, "p:user", "c:polyglot-web").source, "survey");
    assert_eq!(a.report.claimed, 0);
    assert!(a.report.backed >= 8, "{:?}", a.report);
}

#[test]
fn journeys_follow_the_traces_across_components() {
    let g = fixture();
    let a = atlas::engine_atlas(&g);
    assert_eq!(a.journeys.len(), 3);
    let j = &a.journeys[0];
    assert_eq!(j.entry, "web/src/app.ts#main");
    assert!(j.steps.iter().any(|s| s.from == "c:polyglot-web/api" && s.to == "c:polyglot-api/core" && s.label == "http /api/users"), "{:#?}", j.steps);
    assert!(j.steps.iter().any(|s| s.to.starts_with("x:database")), "sinks point at the external: {:#?}", j.steps);
    assert!(a.guide.start_here.iter().any(|p| p.element == "c:polyglot-web"));
    assert!(a.guide.callouts.iter().any(|c| c.title.contains("/api/reports")), "{:#?}", a.guide.callouts);
}

#[test]
fn verify_keeps_only_what_the_code_shows() {
    let g = fixture();
    let mut a = atlas::engine_atlas(&g);
    // an agent regroups the api: one component with a foreign file and a made-up one, and forgets billing.py
    let api = a.containers.iter_mut().find(|c| c.package == "polyglot-api").unwrap();
    api.components = vec![Component {
        id: String::new(),
        name: "HTTP routes".into(),
        description: "Answers the web app.".into(),
        technology: String::new(),
        responsibilities: vec![],
        files: vec!["api/main.py".into(), "api/services/users.py".into(), "web/src/api.ts".into(), "api/made_up.py".into(), "api/services/__init__.py".into()],
    }];
    let mut claims = atlas::Words::new();
    claims.insert(("c:polyglot-web/app".into(), "c:polyglot-web/api".into()), ("loads users through".into(), "function call".into()));
    claims.insert(("c:polyglot-api/http-routes".into(), "c:worker".into()), ("asks for jobs from".into(), "HTTP".into()));
    claims.insert(("c:polyglot-api".into(), "x:nowhere".into()), ("sends".into(), String::new()));
    let v = atlas::verify(&g, &a, Some(&claims));
    let api = container(&v, "polyglot-api");
    let names: Vec<&str> = api.components.iter().map(|k| k.name.as_str()).collect();
    assert_eq!(names, ["HTTP routes", "Other files"]);
    assert_eq!(api.components[0].id, "c:polyglot-api/http-routes");
    assert_eq!(api.components[0].files, ["api/main.py", "api/services/users.py", "api/services/__init__.py"]);
    assert_eq!(api.components[1].files, ["api/services/billing.py"]);
    assert_eq!(v.report.unplaced, ["api/services/billing.py"]);
    // words landed on the backed relationship
    let r = rel(&v, "c:polyglot-web/app", "c:polyglot-web/api");
    assert_eq!(r.label, "loads users through");
    assert_eq!(r.source, "code");
    // a relationship the code does not show is kept as a claim
    let r = rel(&v, "c:polyglot-api/http-routes", "c:worker");
    assert_eq!(r.source, "claimed");
    assert!(r.evidence.is_empty());
    assert_eq!(v.report.claimed, 1);
    assert!(v.relationships.iter().all(|r| r.to != "x:nowhere"));
    assert!(v.report.notes.iter().any(|n| n.contains("x:nowhere")), "{:?}", v.report.notes);
    assert!(v.report.notes.iter().any(|n| n.contains("dropped 2 file(s)")), "{:?}", v.report.notes);
}

#[test]
fn exports_structurizr_dsl() {
    let g = fixture();
    let a = atlas::engine_atlas(&g);
    let dsl = atlas::to_dsl(&a);
    assert!(dsl.starts_with("workspace \"Polyglot\""));
    assert!(dsl.contains("c_polyglot_api = container \"Polyglot Api\""));
    assert!(dsl.contains("c_polyglot_web -> c_polyglot_api \"http /api/users"));
    assert!(dsl.contains("systemContext s {"));
    assert!(dsl.contains("dynamic s j_web_src_app_ts_main"));
    assert!(!dsl.contains("c_polyglot_root = container"), "hidden containers stay off the diagrams");
}

#[test]
fn a_saved_atlas_is_checked_against_a_newer_scan() {
    let g = fixture();
    let mut a = atlas::engine_atlas(&g);
    a.scanned_at = "2020-01-01T00:00:00Z".into();
    a.source = "claude".into();
    let (v, stale) = atlas::for_graph(&g, Some(a));
    assert!(stale.unwrap().contains("earlier scan"));
    assert_eq!(v.source, "claude");
    assert_eq!(v.containers.len(), 5);
}
