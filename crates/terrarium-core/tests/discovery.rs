// The discovery orchestration, driven by stand-in agents so nothing is spent.
use serde_json::{Value, json};
use std::sync::Mutex;
use terrarium_core::atlas;
use terrarium_core::discovery::{self, AgentCall, AgentEvent, FlowRequest, Options, Progress};
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
        "journey" => {
            assert!(call.prompt.contains("Participants"), "narrators get the participant list");
            let note = call.prompt.split("steering this atlas says: ").nth(1).map(|s| s.lines().next().unwrap_or("").to_string());
            if call.target.as_deref() == Some("j:nightly-cleanup") {
                assert!(call.prompt.contains("no trace for this flow"), "{}", call.prompt);
                return Ok((json!({ "name": "Clean up old jobs", "summary": "The worker sweeps the job table.", "messages": [
                    { "from": "Jobs worker", "to": "PostgreSQL", "kind": "store", "label": "deletes old jobs", "caption": "The worker deletes jobs older than a week." }
                ] }), 0.12));
            }
            assert!(call.prompt.contains("The engine's draft of the messages"), "{}", call.prompt);
            Ok((json!({ "name": if note.is_some() { "Open the page, with a note" } else { "Open the page and scan a repository" }, "summary": "The page loads users, then asks the desktop shell to scan a path.", "messages": [
                { "from": "Developer", "to": "Web front end", "kind": "call", "label": "opens the page", "caption": "The developer opens the page." },
                { "from": "Web front end / Helpers", "to": "Users API / Entry point", "kind": "flow", "label": "GET /api/users", "caption": "The page asks the API for the user list over HTTP." },
                { "from": "Users API / Entry point", "to": "PostgreSQL", "kind": "store", "label": "reads users", "caption": "The API reads the users." },
                { "from": "Users API / Entry point", "to": "Web front end / Helpers", "kind": "return", "label": "user list", "caption": "The users come back as JSON." },
                { "from": "Jobs worker", "to": "Desktop shell", "kind": "call", "label": "pings", "caption": "The worker pings the desktop shell, says the narrator." },
                { "from": "Nowhere", "to": "Web front end", "kind": "call", "label": "", "caption": "does not exist" }
            ] }), 0.15))
        }
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

    // journeys: the narrator's messages, resolved onto the atlas and checked
    assert_eq!(a.journeys.len(), 2);
    let j = &a.journeys[0];
    assert_eq!(j.name, "Open the page and scan a repository");
    assert_eq!(j.source, "claude");
    assert!(j.steps.iter().any(|s| s.caption == "The page asks the API for the user list over HTTP."), "{:#?}", j.steps);
    let ms = &j.messages;
    assert_eq!((ms[0].from.as_str(), ms[0].to.as_str(), ms[0].source.as_str()), ("p:developer", "c:polyglot-web", "survey"));
    let http = ms.iter().find(|m| m.label == "GET /api/users").unwrap();
    assert_eq!((http.from.as_str(), http.to.as_str(), http.kind.as_str(), http.source.as_str(), http.by.as_str()), ("c:polyglot-web/entry-point", "c:polyglot-api/entry-point", "flow", "code", "claude"), "the narrator said Helpers, but api.ts is in the entry point: the code decides");
    assert_eq!(http.from_path, "web/src/api.ts#fetchUsers", "a message that matches the engine's draft keeps its evidence");
    let ret = ms.iter().find(|m| m.kind == "return").unwrap();
    assert_eq!(ret.source, "code");
    let store = ms.iter().find(|m| m.kind == "store" && m.to == pg.id).unwrap();
    assert!(store.from.starts_with("c:polyglot-api/"), "the narrator said the entry point; the code says which component touches the database: {}", store.from);
    assert_eq!(store.source, "code", "the engine's database evidence merged into the survey's PostgreSQL");
    let worker_claim = ms.iter().find(|m| m.from == "c:worker" && m.to == "c:polyglot-native").unwrap();
    assert_eq!(worker_claim.source, "claimed", "nothing joins the worker to the desktop shell");
    assert!(ms.iter().all(|m| m.from != "Nowhere" && m.caption != "does not exist"), "unknown participants are dropped");
    // the second journey had no narrator answer beyond the stand-in's default: the engine's messages stand
    assert_eq!(a.journeys[1].source, "claude");
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
fn a_person_steers_which_flows_are_narrated_and_keeps_their_own() {
    let g = fixture();
    let mut kept = atlas::engine_atlas(&g).journeys[1].clone();
    kept.id = "j:mine".into();
    kept.name = "My own journey".into();
    kept.source = "user".into();
    let flows = vec![
        FlowRequest { entry: "web/src/app.ts#main".into(), name: "Open the page".into(), note: "watch the users list".into() },
        FlowRequest { entry: String::new(), name: "Nightly cleanup".into(), note: "there is no trace; read the worker".into() },
    ];
    let prompts = Mutex::new(Vec::<String>::new());
    let runner = |call: &AgentCall, sink: &(dyn Fn(AgentEvent) + Sync)| {
        prompts.lock().unwrap().push(format!("{}:{}", call.role, call.prompt));
        agents(call, sink)
    };
    let (a, run) = discovery::discover(&g, &Options { parallel: 2, max_journeys: 1, flows, keep: vec![kept], ..Options::default() }, &runner, &|_| {}).unwrap();
    // the survey was told, and the narrators got the notes
    let prompts = prompts.lock().unwrap();
    assert!(prompts.iter().any(|p| p.starts_with("survey:") && p.contains("chosen the journeys to narrate") && p.contains("Nightly cleanup")));
    assert!(prompts.iter().any(|p| p.starts_with("journey:") && p.contains("steering this atlas says: watch the users list")));
    assert_eq!(run.agents.iter().filter(|r| r.role == "journey").count(), 2, "the person's flows, not max_journeys, decide");
    let ids: Vec<&str> = a.journeys.iter().map(|j| j.id.as_str()).collect();
    assert_eq!(ids, ["j:web-src-app-ts-main", "j:nightly-cleanup", "j:mine"], "{ids:?}");
    assert_eq!(a.journeys[0].name, "Open the page, with a note");
    assert_eq!(a.journeys[0].note, "watch the users list");
    let cleanup = &a.journeys[1];
    assert_eq!(cleanup.entry, "");
    assert_eq!(cleanup.messages.len(), 1);
    assert_eq!((cleanup.messages[0].from.as_str(), cleanup.messages[0].source.as_str()), ("c:worker", "code"), "no trace, but the code shows the worker on the database");
    let mine = &a.journeys[2];
    assert_eq!((mine.source.as_str(), mine.name.as_str()), ("user", "My own journey"));
    assert!(!mine.messages.is_empty(), "the kept journey's messages were re-resolved onto the new components: {:#?}", mine.messages);
    let ids: Vec<String> = a.containers.iter().flat_map(|c| std::iter::once(c.id.clone()).chain(c.components.iter().map(|k| k.id.clone()))).chain(a.people.iter().map(|p| p.id.clone())).chain(a.externals.iter().map(|x| x.id.clone())).collect();
    assert!(mine.messages.iter().all(|m| ids.contains(&m.from) && ids.contains(&m.to)), "{:#?}", mine.messages);
}

#[test]
fn one_journey_can_be_narrated_again_with_a_note() {
    let g = fixture();
    let (a, _) = discovery::discover(&g, &Options { parallel: 2, max_journeys: 1, ..Options::default() }, &agents, &|_| {}).unwrap();
    let seen = Mutex::new(Vec::<String>::new());
    let progress = |p: Progress| seen.lock().unwrap().push(serde_json::to_value(&p).unwrap()["event"].as_str().unwrap().to_string());
    let (b, run) = discovery::narrate_one(&g, &a, "j:web-src-app-ts-main", "say more about the return", &Options::default(), &agents, &progress).unwrap();
    assert_eq!(run.agents.len(), 1);
    assert_eq!(run.agents[0].role, "journey");
    assert_eq!(b.journeys.len(), a.journeys.len(), "nothing else in the atlas moved");
    let j = b.journeys.iter().find(|j| j.id == "j:web-src-app-ts-main").unwrap();
    assert_eq!(j.name, "Open the page, with a note");
    assert_eq!(j.note, "say more about the return");
    assert!(j.messages.iter().any(|m| m.kind == "return" && m.source == "code"));
    assert_eq!(b.system.name, a.system.name);
    let seen = seen.lock().unwrap();
    assert!(seen.contains(&"journey_done".to_string()) && seen.contains(&"agent_done".to_string()), "{seen:?}");
    // a journey nobody has heard of is followed from the code alone
    let (c, _) = discovery::narrate_one(&g, &a, "Nightly cleanup", "", &Options::default(), &agents, &|_| {}).unwrap();
    assert!(c.journeys.iter().any(|j| j.id == "j:nightly-cleanup" && j.entry.is_empty()));
    let err = discovery::narrate_one(&g, &a, "j:web-src-app-ts-main", "", &Options::default(), &|_: &AgentCall, _: &(dyn Fn(AgentEvent) + Sync)| -> anyhow::Result<(Value, f64)> { anyhow::bail!("cannot start claude") }, &|_| {}).unwrap_err();
    assert!(err.to_string().contains("cannot start claude"));
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
