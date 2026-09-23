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
    assert_eq!(j.source, "engine");
    assert!(j.steps.iter().any(|s| s.from == "c:polyglot-web/api" && s.to == "c:polyglot-api/core" && s.label == "http /api/users"), "{:#?}", j.steps);
    assert!(j.steps.iter().any(|s| s.to.starts_with("x:database")), "sinks point at the external: {:#?}", j.steps);
    assert!(a.guide.start_here.iter().any(|p| p.element == "c:polyglot-web"));
    assert!(a.guide.callouts.iter().any(|c| c.title.contains("/api/reports")), "{:#?}", a.guide.callouts);
    // the sequence: a person starts it, every arrow joins two atlas elements, and each is sourced
    let m0 = &j.messages[0];
    assert_eq!((m0.from.as_str(), m0.to.as_str(), m0.source.as_str(), m0.kind.as_str()), ("p:user", "c:polyglot-web/app", "survey", "call"));
    let http = j.messages.iter().find(|m| m.label == "http /api/users").unwrap();
    assert_eq!((http.from.as_str(), http.to.as_str(), http.kind.as_str(), http.source.as_str(), http.by.as_str()), ("c:polyglot-web/api", "c:polyglot-api/core", "flow", "code", "engine"));
    assert_eq!(http.from_path, "web/src/api.ts#fetchUsers");
    assert!(j.messages.iter().any(|m| m.kind == "store" && m.to.starts_with("x:database")), "{:#?}", j.messages);
    assert!(j.messages.iter().all(|m| m.from != m.to));
    assert_eq!(j.steps.len(), j.messages.iter().filter(|m| !m.from.starts_with("p:")).count(), "the map's steps are the messages minus the person's");
}

#[test]
fn the_engine_ranks_traces_for_where_they_start_and_what_they_add() {
    let g = fixture();
    let find = |p: &str| g.find_by_path(p).unwrap().id;
    // starts a person would recognise
    assert_eq!(atlas::entry_kind(&g, find("web/src/app.ts#main")), Some("main"));
    assert_eq!(atlas::entry_kind(&g, find("api/main.py#get_users")), Some("route"));
    assert_eq!(atlas::entry_kind(&g, find("native/src/commands.rs#scan_repo")), Some("command"));
    assert_eq!(atlas::entry_kind(&g, find("web/src/api.ts#fetchUser")), None);
    let traces = terrarium_core::query::traces(&g);
    let entries = |ts: &[terrarium_core::query::Trace]| ts.iter().map(|t| t.entry_path.clone()).collect::<Vec<_>>();
    // the fixture's three traces keep the order length gave them: each starts somewhere new or crosses something new
    assert_eq!(entries(&atlas::rank_traces(&g, &traces, 5)), ["web/src/app.ts#main", "native/src/lib.rs#run", "web/src/api.ts#fetchUser"]);
    // a near copy of the longest trace (same packages, same boundaries, from the same web app) is longer
    // than the other two but shows nothing new, so it falls behind them
    let mut copy = traces[0].clone();
    copy.entry = find("web/src/api.ts#fetchUser");
    copy.entry_path = "web/src/app.ts#copy".into();
    copy.hops += 1;
    let mut with_copy = vec![traces[0].clone(), copy];
    with_copy.extend(traces[1..].iter().cloned());
    assert_eq!(entries(&atlas::rank_traces(&g, &with_copy, 3)), ["web/src/app.ts#main", "native/src/lib.rs#run", "web/src/api.ts#fetchUser"]);
    assert_eq!(entries(&atlas::rank_traces(&g, &with_copy, 4))[3], "web/src/app.ts#copy");
    // a recognisable start beats a longer trace that starts nowhere in particular
    let mut plain = traces[0].clone();
    plain.entry = find("web/src/api.ts#fetchUser");
    plain.entry_path = "web/src/app.ts#plain".into();
    assert_eq!(entries(&atlas::rank_traces(&g, &[plain, traces[0].clone()], 1)), ["web/src/app.ts#main"]);
}

#[test]
fn a_journey_projects_onto_every_level_and_tallies_with_the_boxes() {
    let g = fixture();
    let a = atlas::engine_atlas(&g);
    let j = &a.journeys[0];
    let ids: std::collections::HashSet<String> = a.containers.iter().flat_map(|c| std::iter::once(c.id.clone()).chain(c.components.iter().map(|k| k.id.clone()))).chain(a.people.iter().map(|p| p.id.clone())).chain(a.externals.iter().map(|x| x.id.clone())).collect();
    // context: the person, the system, the outside systems
    let ctx = atlas::project(&a, j, "context", None);
    assert!(ctx.participants.contains(&"p:user".to_string()) && ctx.participants.contains(&"s".to_string()), "{:?}", ctx.participants);
    assert!(ctx.participants.iter().all(|p| p == "s" || ids.contains(p)));
    assert!(ctx.messages.iter().all(|m| m.from != m.to));
    // containers: every participant is a container, a person or an outside system
    let cont = atlas::project(&a, j, "containers", None);
    assert!(cont.participants.iter().all(|p| ids.contains(p) && !p.contains('/')), "{:?}", cont.participants);
    assert!(cont.messages.iter().any(|m| m.from == "c:polyglot-web" && m.to == "c:polyglot-api"), "{:#?}", cont.messages);
    assert!(cont.messages.len() < j.messages.len(), "inside-a-container messages fold away");
    // components of the web app: its components open up, other containers stay whole
    let comp = atlas::project(&a, j, "components", Some("c:polyglot-web"));
    assert!(comp.participants.iter().any(|p| p.starts_with("c:polyglot-web/")), "{:?}", comp.participants);
    assert!(comp.participants.iter().filter(|p| p.starts_with("c:polyglot-api")).all(|p| !p.contains('/')), "{:?}", comp.participants);
    assert!(comp.messages.iter().all(|m| m.n >= 1 && m.n <= j.messages.len()));
    // mermaid
    let mm = atlas::to_mermaid(&a, j, "containers", None);
    assert!(mm.starts_with("sequenceDiagram\n"));
    assert!(mm.contains("actor p_user as User"));
    assert!(mm.contains("c_polyglot_web ->> c_polyglot_api: http /api/users"), "{mm}");
}

#[test]
fn edited_journeys_are_checked_and_follow_their_files_when_components_move() {
    let g = fixture();
    let a = atlas::engine_atlas(&g);
    let mut j = a.journeys[0].clone();
    // a person adds an arrow the code backs, one it does not, and a return
    j.source = "user".into();
    j.note = "show the worker too".into();
    j.messages.push(atlas::Message { from: "c:polyglot-api/core".into(), to: "c:worker".into(), label: "asks for jobs".into(), caption: "The API asks the worker.".into(), kind: "call".into(), depth: 1, source: String::new(), by: "user".into(), from_path: String::new(), to_path: String::new() });
    j.messages.push(atlas::Message { from: "c:polyglot-api/core".into(), to: "c:polyglot-web/api".into(), label: "user list".into(), caption: "The users come back.".into(), kind: "return".into(), depth: 1, source: String::new(), by: "user".into(), from_path: String::new(), to_path: String::new() });
    j.messages.push(atlas::Message { from: "c:polyglot-web/api".into(), to: "x:nowhere".into(), label: "".into(), caption: "".into(), kind: "call".into(), depth: 0, source: String::new(), by: "user".into(), from_path: String::new(), to_path: String::new() });
    let v = atlas::upsert_journey(&g, &a, j);
    let j = v.journeys.iter().find(|x| x.id == "j:web-src-app-ts-main").unwrap();
    assert_eq!((j.source.as_str(), j.note.as_str()), ("user", "show the worker too"));
    let claim = j.messages.iter().find(|m| m.to == "c:worker").unwrap();
    assert_eq!((claim.source.as_str(), claim.by.as_str()), ("claimed", "user"));
    let ret = j.messages.iter().find(|m| m.kind == "return").unwrap();
    assert_eq!(ret.source, "code", "a return of a backed call is backed");
    assert!(j.messages.iter().all(|m| m.to != "x:nowhere"));
    assert!(v.report.notes.iter().any(|n| n.contains("dropped 1 message")), "{:?}", v.report.notes);
    // the web app is regrouped: the message that carried a path follows its file into the new component
    let mut b = v.clone();
    let web = b.containers.iter_mut().find(|c| c.package == "polyglot-web").unwrap();
    web.components = vec![Component { id: String::new(), name: "Everything".into(), description: "All of it.".into(), technology: String::new(), responsibilities: vec![], files: vec!["web/src/app.ts".into(), "web/src/api.ts".into(), "web/src/view.ts".into()] }];
    let w = atlas::verify(&g, &b, None);
    let j = w.journeys.iter().find(|x| x.id == "j:web-src-app-ts-main").unwrap();
    let http = j.messages.iter().find(|m| m.label == "http /api/users").unwrap();
    assert_eq!(http.from, "c:polyglot-web/everything");
    assert_eq!(http.source, "code");
    assert!(j.messages.iter().all(|m| m.from != m.to));
    // removing it leaves the other journeys
    let r = atlas::remove_journey(&g, &w, "j:web-src-app-ts-main");
    assert_eq!(r.journeys.len(), 2);
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
