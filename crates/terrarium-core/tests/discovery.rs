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

/// Stand-in agents: a surveyor that names things, a scout that proposes flows
/// (traced, untraced, and near copies), container agents that group files (one is
/// careless), a journey narrator, an editor, and one that fails.
fn agents(call: &AgentCall, sink: &(dyn Fn(AgentEvent) + Sync)) -> anyhow::Result<(Value, f64)> {
    match call.role.as_str() {
        "scout" => {
            assert!(call.tools);
            assert!(call.prompt.starts_with("You are scouting"), "{}", call.prompt);
            assert!(call.prompt.contains("- web/src/app.ts#main (4 boundaries"), "the scout sees the traces: {}", call.prompt);
            assert!(call.prompt.contains("http /api/health (no-callers)"), "and the endpoints: {}", call.prompt);
            assert!(call.prompt.contains("Participants on the diagrams"));
            sink(AgentEvent::Reading { path: "api/main.py".into() });
            Ok((json!({ "flows": [
                { "name": "Open the page and scan a repository", "why": "It crosses every container once.", "start": "web/src/app.ts#main", "containers": ["polyglot-web", "polyglot-api"], "trace": "web/src/app.ts#main" },
                // a near copy: the same trace, reached through its file
                { "name": "Load the page", "why": "The page loads.", "start": "web/src/app.ts", "containers": ["polyglot-web"], "trace": "" },
                // traced, but it starts in the web app again: it waits behind the others
                { "name": "Look up one user", "why": "One user's page.", "start": "web/src/api.ts#fetchUser", "containers": ["polyglot-web"], "trace": "web/src/api.ts#fetchUser" },
                // a route nothing in the repository calls: no trace, the narrator starts at its handler
                { "name": "Check the API is up", "why": "Operators poll it.", "start": "GET /api/health", "containers": ["polyglot-api"], "trace": "" },
                // no start at all: the narrator finds it in the code
                { "name": "Nightly cleanup", "why": "Old jobs pile up without it.", "start": "", "containers": ["worker"], "trace": "" },
                { "name": "Sync the jobs", "why": "The desktop shell keeps its jobs current.", "start": "native/src/lib.rs#run", "containers": ["polyglot-native"], "trace": "native/src/lib.rs#run" },
                { "name": "", "why": "nameless", "start": "", "containers": [], "trace": "" }
            ] }), 0.05))
        }
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
            if call.target.as_deref() == Some("j:api-main-py-health") {
                assert!(call.prompt.contains("It starts at `api/main.py#health`"), "{}", call.prompt);
                assert!(call.prompt.contains("Why it matters: Operators poll it."), "{}", call.prompt);
                return Ok((json!({ "name": "Check the API is up", "summary": "An operator asks the API whether it is up.", "messages": [
                    { "from": "Developer", "to": "Users API", "kind": "call", "label": "GET /api/health", "caption": "The operator asks." },
                    { "from": "Users API", "to": "Developer", "kind": "return", "label": "ok", "caption": "The API says it is up." }
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

    // one survey, one scout (nobody chose the flows), four containers (one failed), two journeys, one editor
    assert_eq!(run.agents.len(), 9, "{:#?}", run.agents.iter().map(|r| format!("{} {:?} {}", r.role, r.target, r.ok)).collect::<Vec<_>>());
    let worker = run.agents.iter().find(|r| r.target.as_deref() == Some("c:worker")).unwrap();
    assert!(!worker.ok && worker.error.as_deref().unwrap().contains("600s"));
    // the scout's first two picks: the traced page load, and the health check it found with no trace
    assert!((run.cost_usd - (0.30 + 0.05 + 3.0 * 0.25 + 0.15 + 0.12 + 0.10)).abs() < 1e-9, "{}", run.cost_usd);
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
    // the narrator said "Web front end"; the draft, drawn over the checked components, knows which one the page starts in
    assert_eq!((ms[0].from.as_str(), ms[0].to.as_str(), ms[0].source.as_str()), ("p:developer", "c:polyglot-web/helpers", "survey"));
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
    assert_eq!(j.why, "It crosses every container once.", "the journey keeps why the scout chose it");
    // the second journey is one the scanner cannot trace: the scout found it, the narrator followed it
    assert_eq!((a.journeys[1].id.as_str(), a.journeys[1].entry.as_str(), a.journeys[1].source.as_str()), ("j:api-main-py-health", "api/main.py#health", "claude"));
    // the guide resolves names to ids and drops what does not exist
    let starts: Vec<&str> = a.guide.start_here.iter().map(|p| p.element.as_str()).collect();
    assert_eq!(starts, ["c:polyglot-web", j.id.as_str()]);
    assert_eq!(a.guide.callouts[0].element.as_deref(), Some("c:polyglot-web"));

    // progress: stages in order, partial results along the way
    let events: Vec<String> = seen.lock().unwrap().iter().map(|e| e["event"].as_str().unwrap().to_string()).collect();
    assert_eq!(events[0], "started");
    let stages: Vec<&String> = events.iter().filter(|e| *e == "stage").collect();
    assert_eq!(stages.len(), 6, "survey, field, scout, journeys, editor, verify");
    assert_eq!(events.iter().filter(|e| *e == "proposed").count(), 1);
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
        FlowRequest { entry: "web/src/app.ts#main".into(), name: "Open the page".into(), note: "watch the users list".into(), ..FlowRequest::default() },
        FlowRequest { entry: String::new(), name: "Nightly cleanup".into(), note: "there is no trace; read the worker".into(), ..FlowRequest::default() },
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
    assert!(run.agents.iter().all(|r| r.role != "scout"), "the person chose the flows: no scout");
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
fn the_scout_proposes_key_flows_and_the_narrators_follow_them() {
    let g = fixture();
    let prompts = Mutex::new(Vec::<String>::new());
    let runner = |call: &AgentCall, sink: &(dyn Fn(AgentEvent) + Sync)| {
        prompts.lock().unwrap().push(format!("{}:{}", call.role, call.prompt));
        agents(call, sink)
    };
    let seen = Mutex::new(Vec::<Value>::new());
    let progress = |p: Progress| seen.lock().unwrap().push(serde_json::to_value(&p).unwrap());
    let (a, run) = discovery::discover(&g, &Options { parallel: 3, max_journeys: 4, ..Options::default() }, &runner, &progress).unwrap();
    // near copies dropped, the web app's second flow left out for flows that start elsewhere, the scout's order kept
    let ids: Vec<&str> = a.journeys.iter().map(|j| j.id.as_str()).collect();
    assert_eq!(ids, ["j:web-src-app-ts-main", "j:api-main-py-health", "j:nightly-cleanup", "j:native-src-lib-rs-run"], "{ids:?}");
    assert_eq!(run.agents.iter().filter(|r| r.role == "journey").count(), 4);
    let cleanup = a.journeys.iter().find(|j| j.id == "j:nightly-cleanup").unwrap();
    assert_eq!((cleanup.entry.as_str(), cleanup.why.as_str()), ("", "Old jobs pile up without it."), "no start: narrated from the code, and the why stays");
    let sync = a.journeys.iter().find(|j| j.id == "j:native-src-lib-rs-run").unwrap();
    assert_eq!(sync.why, "The desktop shell keeps its jobs current.");
    let prompts = prompts.lock().unwrap();
    assert!(prompts.iter().any(|p| p.starts_with("journey:") && p.contains("Why it matters: It crosses every container once.")), "narrators read why");
    // the proposals, as the field notes see them
    let seen = seen.lock().unwrap();
    let started = seen.iter().find(|e| e["event"] == "started").unwrap();
    assert_eq!((started["scout"].as_bool(), started["journeys"].as_u64()), (Some(true), Some(4)));
    let proposed = seen.iter().find(|e| e["event"] == "proposed").unwrap();
    let flows = proposed["flows"].as_array().unwrap();
    assert_eq!(flows.len(), 4);
    assert_eq!((flows[0]["matched"].as_str(), flows[0]["hops"].as_u64(), flows[0]["containers"][0].as_str()), (Some("trace"), Some(4), Some("c:polyglot-web")));
    assert_eq!((flows[1]["matched"].as_str(), flows[1]["entry"].as_str(), flows[1]["containers"][0].as_str()), (Some("endpoint"), Some("api/main.py#health"), Some("c:polyglot-api")));
    assert_eq!((flows[2]["matched"].as_str(), flows[2]["entry"].as_str(), flows[2]["containers"][0].as_str()), (Some("none"), Some(""), Some("c:worker")));
    let stages: Vec<&str> = seen.iter().filter(|e| e["event"] == "stage").map(|e| e["stage"].as_str().unwrap()).collect();
    assert_eq!(stages, ["survey", "field", "scout", "journeys", "editor", "verify"], "the scout and the narrators run after the base C4 pass is checked");
    assert!(seen.iter().any(|e| e["event"] == "agent_activity" && e["role"] == "scout" && e["path"] == "api/main.py"));
}

#[test]
fn when_the_scout_fails_the_survey_picks_stand() {
    let g = fixture();
    for broken in [json!(null), json!({ "flows": "not a list" })] {
        let runner = |call: &AgentCall, sink: &(dyn Fn(AgentEvent) + Sync)| -> anyhow::Result<(Value, f64)> {
            if call.role == "scout" {
                if broken.is_null() {
                    anyhow::bail!("the scout could not start");
                }
                return Ok((broken.clone(), 0.01));
            }
            agents(call, sink)
        };
        let (a, run) = discovery::discover(&g, &Options { parallel: 2, max_journeys: 2, ..Options::default() }, &runner, &|_| {}).unwrap();
        let scout = run.agents.iter().find(|r| r.role == "scout").unwrap();
        assert!(!scout.ok, "{:?}", scout);
        // the survey's pick first, then the traces the engine ranks best
        let ids: Vec<&str> = a.journeys.iter().map(|j| j.id.as_str()).collect();
        assert_eq!(ids, ["j:web-src-app-ts-main", "j:native-src-lib-rs-run"], "{ids:?}");
        assert!(a.journeys.iter().all(|j| j.why.is_empty()));
    }
}

#[test]
fn the_scout_runs_alone_so_a_person_steers_from_its_proposals() {
    let g = fixture();
    let base = atlas::engine_atlas(&g);
    let seen = Mutex::new(Vec::<String>::new());
    let progress = |p: Progress| seen.lock().unwrap().push(serde_json::to_value(&p).unwrap()["event"].as_str().unwrap().to_string());
    let (proposals, run) = discovery::propose_flows(&g, &base, &Options { max_journeys: 3, ..Options::default() }, &agents, &progress).unwrap();
    assert_eq!(run.agents.len(), 1);
    assert_eq!(run.agents[0].role, "scout");
    assert!((run.cost_usd - 0.05).abs() < 1e-9);
    let names: Vec<&str> = proposals.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Open the page and scan a repository", "Check the API is up", "Nightly cleanup"]);
    assert_eq!(*seen.lock().unwrap(), ["stage", "agent_started", "agent_activity", "agent_done", "proposed"]);
    // a proposal is a flow the person can hand straight to discovery
    let flows: Vec<FlowRequest> = proposals.iter().map(|p| p.flow()).collect();
    assert_eq!((flows[0].entry.as_str(), flows[0].why.as_str()), ("web/src/app.ts#main", "It crosses every container once."));
    let (a, run) = discovery::discover(&g, &Options { parallel: 2, flows, ..Options::default() }, &agents, &|_| {}).unwrap();
    assert!(run.agents.iter().all(|r| r.role != "scout"));
    let ids: Vec<&str> = a.journeys.iter().map(|j| j.id.as_str()).collect();
    assert_eq!(ids, ["j:web-src-app-ts-main", "j:api-main-py-health", "j:nightly-cleanup"], "the same journeys, whether the scout ran inside discovery or before it");
    assert_eq!(a.journeys[1].why, "Operators poll it.");
    // a scout that cannot run is an error the person sees
    let err = discovery::propose_flows(&g, &base, &Options::default(), &|_: &AgentCall, _: &(dyn Fn(AgentEvent) + Sync)| -> anyhow::Result<(Value, f64)> { anyhow::bail!("cannot start claude") }, &|_| {}).unwrap_err();
    assert!(err.to_string().contains("cannot start claude"), "{err}");
}

#[test]
fn a_start_is_matched_to_the_code_however_it_is_written() {
    let g = fixture();
    let traces = terrarium_core::query::traces(&g);
    let at = |s: &str| {
        let (entry, t, how) = discovery::resolve_start(&g, &traces, s, "");
        (entry, t.map(|t| t.entry_path), how)
    };
    assert_eq!(at("web/src/app.ts#main"), ("web/src/app.ts#main".into(), Some("web/src/app.ts#main".into()), "trace"));
    assert_eq!(at("web/src/app.ts"), ("web/src/app.ts#main".into(), Some("web/src/app.ts#main".into()), "file"));
    assert_eq!(at("GET /api/users/{id}").2, "none", "routes are matched as the scanner spells them");
    assert_eq!(at("GET /api/users/*"), ("web/src/api.ts#fetchUser".into(), Some("web/src/api.ts#fetchUser".into()), "endpoint"), "a route is followed from its caller");
    assert_eq!(at("scan_repo").2, "endpoint", "a bare word is an IPC command");
    assert_eq!(at("http /api/health"), ("api/main.py#health".into(), None, "endpoint"));
    assert_eq!(at("worker/store/store.go"), ("worker/store/store.go".into(), None, "file"));
    assert_eq!(at("api/main.py#health"), ("api/main.py#health".into(), None, "symbol"));
    assert_eq!(at("Nightly cleanup"), (String::new(), None, "none"));
    // the scout's trace wins over its start
    let (entry, _, how) = discovery::resolve_start(&g, &traces, "somewhere else", "native/src/lib.rs#run");
    assert_eq!((entry.as_str(), how), ("native/src/lib.rs#run", "trace"));
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
