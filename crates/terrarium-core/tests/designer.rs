// The design orchestration, driven by stand-in agents so nothing is spent.
use serde_json::json;
use std::path::PathBuf;
use std::sync::Mutex;
use terrarium_core::designer::{self, AgentCall, Options};
use terrarium_core::{ScanOptions, build, scan};

fn fixture() -> terrarium_core::Graph {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/polyglot");
    scan(&root, &ScanOptions::default()).unwrap()
}

/// A stand-in agent: files listed in the prompt become steps, in reverse (which
/// breaks the order), plus one file from another package and one that is not real.
fn careless(call: &AgentCall) -> anyhow::Result<(serde_json::Value, f64)> {
    if call.role == "assembler" {
        return Ok((json!({ "title": "Polyglot Harbour", "summary": "Four languages, one town." }), 0.10));
    }
    if call.role == "worker" {
        anyhow::bail!("agent took longer than 600s and was stopped");
    }
    assert!(call.tools, "chapter agents may read the repository");
    let mut files: Vec<String> = call
        .prompt
        .lines()
        .filter_map(|l| l.trim().split_once(". ").and_then(|(n, rest)| n.parse::<u32>().ok().map(|_| rest)))
        .map(|rest| rest.split(" (").next().unwrap().to_string())
        .collect();
    files.reverse();
    let mut steps: Vec<_> = files.iter().map(|f| json!({ "title": format!("Raise {f}"), "caption": format!("{f} matters."), "files": [f] })).collect();
    steps.push(json!({ "title": "Borrow", "caption": "x", "files": ["worker/main.go", "made/up.rs"] }));
    Ok((json!({ "name": format!("The {} quarter", call.role), "blurb": "Does things.", "steps": steps }), 0.25))
}

#[test]
fn agents_write_the_words_and_the_engine_keeps_it_standing() {
    let g = fixture();
    let seen = Mutex::new(Vec::new());
    let progress = |p: designer::Progress| seen.lock().unwrap().push(serde_json::to_value(&p).unwrap()["event"].as_str().unwrap().to_string());
    let (design, run) = designer::design(&g, &Options { parallel: 3, ..Options::default() }, &careless, &progress).unwrap();

    // one agent per sub-build plus the assembler; the failed one is reported, not fatal
    assert_eq!(run.agents.len(), 5);
    let worker = run.agents.iter().find(|a| a.role == "worker").unwrap();
    assert!(!worker.ok && worker.error.as_deref().unwrap().contains("600s"));
    assert!((run.cost_usd - (3.0 * 0.25 + 0.10)).abs() < 1e-9);
    assert_eq!(run.model, "claude-opus-5-5");
    let events = seen.into_inner().unwrap();
    assert_eq!(events.first().map(String::as_str), Some("started"));
    assert_eq!(events.iter().filter(|e| *e == "agent_done").count(), 4);

    // the agents' words made it in
    assert_eq!(design.source, "claude");
    assert_eq!(design.title, "Polyglot Harbour");
    assert!(design.sub_builds.iter().any(|s| s.name == "The polyglot-api quarter"));
    assert!(design.steps.iter().any(|s| s.title == "Raise api/main.py"));
    // the sub-build whose agent failed keeps the engine's words
    let w = design.steps.iter().find(|s| s.files.contains(&"worker/store/store.go".to_string())).unwrap();
    assert!(w.title.starts_with("Add "), "{w:?}");

    // and whatever they wrote, the model holds together
    let c = build::check(&g, &design);
    assert!(c.ok, "{:#?}", c.weak);
    assert_eq!(c.files, 12);
}

#[test]
fn a_run_where_every_agent_fails_is_an_error() {
    let g = fixture();
    let fail = |_: &AgentCall| -> anyhow::Result<(serde_json::Value, f64)> { anyhow::bail!("cannot start claude") };
    let err = designer::design(&g, &Options::default(), &fail, &|_| {}).unwrap_err();
    assert!(err.to_string().contains("every design agent failed: cannot start claude"));
}
