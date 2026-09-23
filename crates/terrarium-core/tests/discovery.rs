// The discovery orchestration, driven by stand-in agents so nothing is spent.
use serde_json::{Value, json};
use std::sync::Mutex;
use terrarium_core::discovery::{self, AgentCall, AgentEvent, Options, Progress};
use terrarium_core::{ScanOptions, scan};

fn fixture() -> terrarium_core::Graph {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/polyglot");
    scan(&root, &ScanOptions::default()).unwrap()
}

/// Files listed in a container prompt, spelled as the engine spelled them.
fn files_in(prompt: &str) -> Vec<String> {
    prompt.lines().filter_map(|l| l.trim().split_once(". ").and_then(|(n, rest)| n.parse::<u32>().ok().map(|_| rest))).filter(|r| r.contains(" lines)")).map(|r| r.split(" (").next().unwrap().to_string()).collect()
}

/// Stand-in agents: a surveyor that names things, container agents that group
/// files (one is careless), a journey narrator, an editor, and one that fails.
fn agents(call: &AgentCall, sink: &(dyn Fn(AgentEvent) + Sync)) -> anyhow::Result<(Value, f64)> {
    match call.role.as_str() {
        "survey" => {
            sink(AgentEvent::Reading { path: "README.md".into() });
            Ok((json!({
                "system": { "name": "Polyglot Town", "purpose": "A demo of four languages sharing one job list.", "summary": "A web front end talks to a Python API and a Rust desktop shell, which asks a Go worker for jobs." },
                "people": [{ "name": "Developer", "description": "Opens repositories and watches jobs.", "uses": [{ "container": "polyglot-web", "how": "clicks around in" }] }],
                "externals": [{ "name": "PostgreSQL", "kind": "database", "description": "Users and jobs.", "used_by": [{ "container": "polyglot-api", "how": "stores users in", "technology": "SQL" }] }],
                "containers": [
                    { "package": "polyglot-web", "name": "Web front end", "kind": "web", "technology": "TypeScript, Vite", "description": "The page a developer sees.", "hidden": false },
                    { "package": "polyglot-api", "name": "Users API", "kind": "service", "technology": "Python, FastAPI", "description": "Serves users and sends welcomes.", "hidden": false },
                    { "package": "polyglot-native", "name": "Desktop shell", "kind": "desktop", "technology": "Rust, Tauri", "description": "Runs commands on the machine.", "hidden": false },
                    { "package": "worker", "name": "Jobs worker", "kind": "worker", "technology": "Go", "description": "Lists jobs from the database.", "hidden": false },
                    { "package": "polyglot-root", "name": "Workspace", "kind": "tooling", "technology": "", "description": "The npm workspace root.", "hidden": true }
                ],
                "journeys": [{ "entry": "web/src/app.ts#main", "name": "Open the page and scan a repository" }]
            }), 0.30))
        }
        "container" => {
            assert!(call.tools);
            let target = call.target.clone().unwrap();
            if target == "c:worker" {
                anyhow::bail!("agent took longer than 600s and was stopped");
            }
            let files = files_in(&call.prompt);
            for f in &files {
                sink(AgentEvent::Reading { path: f.clone() });
            }
            let (first, rest) = files.split_first().unwrap();
            let mut components = vec![json!({ "name": "Entry point", "description": "Where it starts.", "technology": "", "responsibilities": ["start things"], "files": [first] })];
            if !rest.is_empty() {
                components.push(json!({ "name": "Helpers", "description": "The rest.", "technology": "", "responsibilities": [], "files": rest }));
            }
            let mut relationships = vec![json!({ "from": "Helpers", "to": "Entry point", "label": "leans on", "technology": "function call" })];
            if target == "c:polyglot-api" {
                // careless: a foreign file, and a relationship the code does not show
                components[0]["files"].as_array_mut().unwrap().push(json!("worker/main.go"));
                relationships.push(json!({ "from": "Helpers", "to": "Jobs worker", "label": "asks for jobs from", "technology": "HTTP" }));
                relationships.push(json!({ "from": "Helpers", "to": "PostgreSQL", "label": "keeps users in", "technology": "SQL" }));
            }
            Ok((json!({ "description": format!("The {target} container."), "technology": "as surveyed", "responsibilities": ["do its job"], "components": components, "relationships": relationships }), 0.25))
        }
        "journey" => Ok((json!({ "name": "Open the page and scan a repository", "summary": "The page loads users, then asks the desktop shell to scan a path.", "captions": [{ "step": 2, "caption": "The page asks the API for the user list over HTTP." }] }), 0.15)),
        "editor" => {
            assert!(!call.tools);
            Ok((json!({ "summary": "Four containers, one job list.", "start_here": [{ "element": "Web front end", "why": "Everything starts with a click." }, { "element": "Open the page and scan a repository", "why": "It touches every container." }, { "element": "Nowhere", "why": "does not exist" }], "callouts": [{ "title": "Reports are never served", "detail": "The page calls /api/reports and nothing answers.", "element": "Web front end" }] }), 0.10))
        }
        other => anyhow::bail!("unexpected role {other}"),
    }
}

#[test]
fn agents_write_the_words_and_the_engine_checks_them() {
    let g = fixture();
    let seen = Mutex::new(Vec::<Value>::new());
    let progress = |p: Progress| seen.lock().unwrap().push(serde_json::to_value(&p).unwrap());
    let (a, run) = discovery::discover(&g, &Options { parallel: 3, max_journeys: 2, ..Options::default() }, &agents, &progress).unwrap();

    // one survey, four containers (one failed), two journeys, one editor
    assert_eq!(run.agents.len(), 8, "{:#?}", run.agents.iter().map(|r| format!("{} {:?} {}", r.role, r.target, r.ok)).collect::<Vec<_>>());
    let worker = run.agents.iter().find(|r| r.target.as_deref() == Some("c:worker")).unwrap();
    assert!(!worker.ok && worker.error.as_deref().unwrap().contains("600s"));
    assert!((run.cost_usd - (0.30 + 3.0 * 0.25 + 2.0 * 0.15 + 0.10)).abs() < 1e-9);
    assert_eq!(run.agents.iter().find(|r| r.role == "survey").unwrap().reads, 1);

    // the survey's words
    assert_eq!(a.source, "claude");
    assert_eq!(a.system.name, "Polyglot Town");
    assert_eq!(a.system.summary, "Four containers, one job list.");
    let web = a.containers.iter().find(|c| c.package == "polyglot-web").unwrap();
    assert_eq!(web.name, "Web front end");
    assert!(a.people.iter().any(|p| p.name == "Developer"));
    let dev_uses = a.relationships.iter().find(|r| r.from == "p:developer" && r.to == "c:polyglot-web").unwrap();
    assert_eq!((dev_uses.label.as_str(), dev_uses.source.as_str()), ("clicks around in", "survey"));
    assert!(a.containers.iter().find(|c| c.package == "polyglot-root").unwrap().hidden);

    // the container agents' components, checked
    let names: Vec<&str> = web.components.iter().map(|k| k.name.as_str()).collect();
    assert_eq!(names, ["Entry point", "Helpers"]);
    let api = a.containers.iter().find(|c| c.package == "polyglot-api").unwrap();
    assert!(api.components[0].files.iter().all(|f| f.starts_with("api/")), "foreign file dropped: {:?}", api.components[0].files);
    // the failed container keeps the engine's components
    let worker = a.containers.iter().find(|c| c.package == "worker").unwrap();
    assert_eq!(worker.name, "Jobs worker", "the survey's name stands");
    assert!(worker.components.iter().any(|k| k.name == "Core" || k.name == "Main"), "{:?}", worker.components.iter().map(|k| &k.name).collect::<Vec<_>>());
    // relationships: words on backed edges, claims marked, survey externals kept
    let r = a.relationships.iter().find(|r| r.from == "c:polyglot-web/helpers" && r.to == "c:polyglot-web/entry-point").unwrap();
    assert_eq!((r.label.as_str(), r.source.as_str()), ("leans on", "code"));
    let claim = a.relationships.iter().find(|r| r.from == "c:polyglot-api/helpers" && r.to == "c:worker").unwrap();
    assert_eq!(claim.source, "claimed");
    let pg = a.externals.iter().find(|x| x.name == "PostgreSQL").unwrap();
    assert_eq!(a.relationships.iter().find(|r| r.from == "c:polyglot-api" && r.to == pg.id).unwrap().label, "stores users in");
    assert_eq!(a.report.claimed, 2, "the api's helpers -> entry point is a claim too: main.py imports the services, not the reverse");

    // journeys: the narrator's words, the engine's steps
    assert_eq!(a.journeys.len(), 2);
    let j = &a.journeys[0];
    assert_eq!(j.name, "Open the page and scan a repository");
    assert!(j.steps.iter().any(|s| s.caption == "The page asks the API for the user list over HTTP."), "{:#?}", j.steps);
    // the guide resolves names to ids and drops what does not exist
    let starts: Vec<&str> = a.guide.start_here.iter().map(|p| p.element.as_str()).collect();
    assert_eq!(starts, ["c:polyglot-web", j.id.as_str()]);
    assert_eq!(a.guide.callouts[0].element.as_deref(), Some("c:polyglot-web"));

    // progress: stages in order, partial results along the way
    let events: Vec<String> = seen.lock().unwrap().iter().map(|e| e["event"].as_str().unwrap().to_string()).collect();
    assert_eq!(events[0], "started");
    let stages: Vec<&String> = events.iter().filter(|e| *e == "stage").collect();
    assert_eq!(stages.len(), 4);
    assert!(events.iter().any(|e| e == "agent_activity"));
    assert_eq!(events.iter().filter(|e| *e == "container_done").count(), 3);
    assert_eq!(events.iter().filter(|e| *e == "journey_done").count(), 2);
    assert_eq!(events.last().map(String::as_str), Some("verified"));
    let seen = seen.lock().unwrap();
    let activity = seen.iter().find(|e| e["event"] == "agent_activity").unwrap();
    assert_eq!(activity["kind"], "reading");
    assert_eq!(activity["path"], "README.md");
}

#[test]
fn a_run_where_every_agent_fails_is_an_error() {
    let g = fixture();
    let fail = |_: &AgentCall, _: &(dyn Fn(AgentEvent) + Sync)| -> anyhow::Result<(Value, f64)> { anyhow::bail!("cannot start claude") };
    let err = discovery::discover(&g, &Options::default(), &fail, &|_| {}).unwrap_err();
    assert!(err.to_string().contains("every discovery agent failed: cannot start claude"), "{err}");
}

#[test]
fn stream_events_name_the_files_agents_open() {
    let v = json!({ "type": "assistant", "message": { "content": [{ "type": "tool_use", "name": "Read", "input": { "file_path": "/repo/api/main.py" } }] } });
    match discovery::stream_event(&v, "/repo") {
        Some(AgentEvent::Reading { path }) => assert_eq!(path, "api/main.py"),
        other => panic!("{other:?}"),
    }
    let v = json!({ "type": "assistant", "message": { "content": [{ "type": "tool_use", "name": "Grep", "input": { "pattern": "fetch" } }] } });
    assert!(matches!(discovery::stream_event(&v, "/repo"), Some(AgentEvent::Searching { query }) if query == "fetch"));
    assert!(discovery::stream_event(&json!({ "type": "user" }), "/repo").is_none());
}
