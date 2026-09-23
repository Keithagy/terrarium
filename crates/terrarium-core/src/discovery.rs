//! Discovery: agents read the codebase and write the atlas, and the engine
//! checks every word against the graph.
//!
//! Four kinds of agent run in three stages:
//!
//! 1. The surveyor (one agent) reads the manifests, README and entry points and
//!    decides what the system is, who uses it, what it talks to, and what each
//!    package runs as.
//! 2. The field: one agent per container reads that container's code and
//!    groups it into components, and one agent per journey follows a flow and
//!    writes its messages, over the atlas's own elements, as a sequence diagram.
//!    They run in parallel. The person steering the atlas may choose the flows
//!    and leave a note for each ([`Options::flows`]); one flow can be narrated
//!    again on its own ([`narrate_one`]).
//! 3. The editor (one agent, no tools) writes the summary, the reading guide and
//!    the callouts from what the others found.
//!
//! Then the engine verifies: components only hold files that exist, every
//! relationship is backed by code, declared by the survey, or marked a claim, and
//! every message of every journey joins two elements and is sourced the same way.
//!
//! Agents run through the Claude Code CLI (`claude -p`) with read-only tools, a
//! JSON schema for their answer and streamed output, so every file an agent
//! opens is reported as it happens.

use crate::atlas::{self, Atlas, Component, Container, External, Journey, Person, Words};
use crate::model::*;
use crate::query;
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";

#[derive(Debug, Clone)]
pub struct Options {
    pub model: String,
    /// `low` … `max`.
    pub effort: String,
    /// Agents running at once in the field stage.
    pub parallel: usize,
    /// Spending cap per agent, in US dollars.
    pub budget_usd: f64,
    pub timeout: Duration,
    pub claude: PathBuf,
    /// Journeys to narrate (the survey picks which, unless `flows` says).
    pub max_journeys: usize,
    /// Flows the person steering the atlas asked for. When any are given they are
    /// the journeys; the survey's picks are not used.
    pub flows: Vec<FlowRequest>,
    /// Journeys to carry into the new atlas untouched (a person's edits).
    pub keep: Vec<Journey>,
}

/// A flow a person asked the narrators to follow.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FlowRequest {
    /// Entry symbol path of a trace the scanner found (`web/src/app.ts#main`); empty
    /// when the person only named the flow and the narrator must find it in the code.
    #[serde(default)]
    pub entry: String,
    /// What the user is doing, as a verb phrase. Empty means the narrator names it.
    #[serde(default)]
    pub name: String,
    /// What to pay attention to, in the person's words.
    #[serde(default)]
    pub note: String,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            model: std::env::var("TERRARIUM_DISCOVERY_MODEL").or_else(|_| std::env::var("TERRARIUM_DESIGN_MODEL")).unwrap_or_else(|_| DEFAULT_MODEL.to_string()),
            effort: "high".into(),
            parallel: 4,
            budget_usd: 2.0,
            timeout: Duration::from_secs(600),
            claude: find_claude(),
            max_journeys: 4,
            flows: vec![],
            keep: vec![],
        }
    }
}

/// `claude` as a GUI app sees it: its PATH rarely includes the install directory.
pub fn find_claude() -> PathBuf {
    if let Ok(p) = std::env::var("TERRARIUM_CLAUDE") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_default();
    for p in [format!("{home}/.local/bin/claude"), format!("{home}/.claude/local/claude"), "/opt/homebrew/bin/claude".into(), "/usr/local/bin/claude".into()] {
        if Path::new(&p).exists() {
            return PathBuf::from(p);
        }
    }
    PathBuf::from("claude")
}

/// One agent's job, as handed to a [`Runner`].
#[derive(Debug, Clone)]
pub struct AgentCall {
    /// `survey`, `container`, `journey` or `editor`.
    pub role: String,
    /// The container or journey id this agent works on.
    pub target: Option<String>,
    pub prompt: String,
    pub schema: Value,
    /// Whether the agent may read the repository.
    pub tools: bool,
}

/// Something an agent did while working, reported as it happens.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    Reading { path: String },
    Searching { query: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentRun {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub cost_usd: f64,
    pub secs: f64,
    /// Files the agent opened.
    pub reads: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Run {
    pub model: String,
    pub agents: Vec<AgentRun>,
    pub cost_usd: f64,
    pub secs: f64,
}

/// Progress, in the order it happens. Partial results ride along so a UI can
/// draw the atlas as it is discovered.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Progress {
    Started { model: String, containers: usize, journeys: usize },
    /// `survey`, `field`, `editor`, `verify`.
    Stage { stage: String },
    AgentStarted { role: String, target: Option<String>, name: String },
    AgentActivity { role: String, target: Option<String>, #[serde(flatten)] activity: AgentEvent },
    AgentDone { role: String, target: Option<String>, ok: bool, cost_usd: f64, secs: f64, error: Option<String> },
    SurveyDone { system: atlas::System, people: Vec<Person>, externals: Vec<External>, containers: Vec<Container> },
    ContainerDone { container: Container },
    JourneyDone { journey: Journey },
    Verified { atlas: Atlas },
}

/// What runs an agent: the real CLI, or a stand-in in tests. Reports what the
/// agent does through the sink and returns its structured answer and cost.
pub type Runner<'a> = dyn Fn(&AgentCall, &(dyn Fn(AgentEvent) + Sync)) -> Result<(Value, f64)> + Sync + 'a;

// ---- answers -------------------------------------------------------------------------

#[derive(Deserialize)]
struct SurveyAnswer {
    system: SystemAnswer,
    #[serde(default)]
    people: Vec<PersonAnswer>,
    #[serde(default)]
    externals: Vec<ExternalAnswer>,
    containers: Vec<ContainerSurvey>,
    #[serde(default)]
    journeys: Vec<JourneyPick>,
}

#[derive(Deserialize)]
struct SystemAnswer {
    name: String,
    purpose: String,
    summary: String,
}

#[derive(Deserialize)]
struct PersonAnswer {
    name: String,
    description: String,
    #[serde(default)]
    uses: Vec<UseAnswer>,
}

#[derive(Deserialize)]
struct UseAnswer {
    container: String,
    how: String,
}

#[derive(Deserialize)]
struct ExternalAnswer {
    name: String,
    kind: String,
    description: String,
    #[serde(default)]
    used_by: Vec<UsedByAnswer>,
}

#[derive(Deserialize)]
struct UsedByAnswer {
    container: String,
    how: String,
    #[serde(default)]
    technology: String,
}

#[derive(Deserialize)]
struct ContainerSurvey {
    package: String,
    name: String,
    kind: String,
    technology: String,
    description: String,
    #[serde(default)]
    hidden: bool,
}

#[derive(Deserialize)]
struct JourneyPick {
    entry: String,
    name: String,
}

#[derive(Deserialize)]
struct ContainerAnswer {
    description: String,
    technology: String,
    #[serde(default)]
    responsibilities: Vec<String>,
    components: Vec<ComponentAnswer>,
    #[serde(default)]
    relationships: Vec<RelAnswer>,
}

#[derive(Deserialize)]
struct ComponentAnswer {
    name: String,
    description: String,
    #[serde(default)]
    technology: String,
    #[serde(default)]
    responsibilities: Vec<String>,
    files: Vec<String>,
}

#[derive(Deserialize)]
struct RelAnswer {
    from: String,
    to: String,
    label: String,
    #[serde(default)]
    technology: String,
}

#[derive(Deserialize)]
struct JourneyAnswer {
    name: String,
    summary: String,
    #[serde(default)]
    messages: Vec<MessageAnswer>,
}

#[derive(Deserialize)]
struct MessageAnswer {
    from: String,
    to: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    caption: String,
}

#[derive(Deserialize)]
struct EditorAnswer {
    summary: String,
    #[serde(default)]
    start_here: Vec<PointerAnswer>,
    #[serde(default)]
    callouts: Vec<CalloutAnswer>,
}

#[derive(Deserialize)]
struct PointerAnswer {
    element: String,
    why: String,
}

#[derive(Deserialize)]
struct CalloutAnswer {
    title: String,
    detail: String,
    #[serde(default)]
    element: String,
}

// ---- schemas -------------------------------------------------------------------------

fn s(desc: &str) -> Value {
    json!({ "type": "string", "description": desc })
}

fn arr(items: Value) -> Value {
    json!({ "type": "array", "items": items })
}

fn obj(required: &[&str], props: Value) -> Value {
    json!({ "type": "object", "additionalProperties": false, "required": required, "properties": props })
}

fn survey_schema() -> Value {
    obj(
        &["system", "people", "externals", "containers", "journeys"],
        json!({
            "system": obj(&["name", "purpose", "summary"], json!({
                "name": s("2 to 5 words: what the software is, as a product name a newcomer would recognise"),
                "purpose": s("one sentence: what it is for and for whom"),
                "summary": s("2 or 3 sentences: what it does and how the containers fit together")
            })),
            "people": arr(obj(&["name", "description", "uses"], json!({
                "name": s("a role, e.g. Developer, Operator, Customer"),
                "description": s("one sentence: who they are and what they want"),
                "uses": arr(obj(&["container", "how"], json!({
                    "container": s("package name of a container they use directly"),
                    "how": s("2 to 6 words, e.g. 'opens repositories in', 'runs from a shell'")
                })))
            }))),
            "externals": arr(obj(&["name", "kind", "description", "used_by"], json!({
                "name": s("e.g. PostgreSQL, Redis, Stripe API, File system"),
                "kind": { "type": "string", "enum": ["database", "queue", "filesystem", "service", "system"] },
                "description": s("one sentence"),
                "used_by": arr(obj(&["container", "how", "technology"], json!({
                    "container": s("package name"),
                    "how": s("2 to 6 words, e.g. 'stores users in', 'enqueues emails on'"),
                    "technology": s("e.g. SQL, HTTP/JSON, AMQP; empty if unknown")
                })))
            }))),
            "containers": arr(obj(&["package", "name", "kind", "technology", "description", "hidden"], json!({
                "package": s("the package name exactly as listed"),
                "name": s("2 to 4 words: what it is to a newcomer, e.g. 'Web front end', 'Jobs API', 'Report worker'"),
                "kind": { "type": "string", "enum": ["web", "desktop", "service", "worker", "cli", "library", "tooling"] },
                "technology": s("language and main frameworks, e.g. 'Python, FastAPI, Celery'"),
                "description": s("one or two sentences: what it does for the system"),
                "hidden": { "type": "boolean", "description": "true for tooling, fixtures, examples: things that are not part of the running system" }
            }))),
            "journeys": arr(obj(&["entry", "name"], json!({
                "entry": s("the entry exactly as listed under journeys"),
                "name": s("what the user is doing, as a verb phrase: 'Open a repository', 'Sign up and get a welcome email'")
            })))
        }),
    )
}

fn container_schema() -> Value {
    obj(
        &["description", "technology", "responsibilities", "components", "relationships"],
        json!({
            "description": s("2 or 3 sentences: what this container does, for whom, and how it is put together"),
            "technology": s("language and main frameworks"),
            "responsibilities": arr(s("one responsibility each, 3 to 6 words, starting with a verb")),
            "components": arr(obj(&["name", "description", "technology", "responsibilities", "files"], json!({
                "name": s("2 to 4 words naming what it does, e.g. 'HTTP routes', 'User store', 'Scene renderer'"),
                "description": s("1 or 2 sentences: what it does and what depends on it"),
                "technology": s("only when different from the container's; else empty"),
                "responsibilities": arr(s("2 to 4 short responsibilities")),
                "files": arr(s("file paths exactly as listed"))
            }))),
            "relationships": arr(obj(&["from", "to", "label", "technology"], json!({
                "from": s("a component name from above"),
                "to": s("a component name, another container's name, or an external system's name"),
                "label": s("2 to 6 words saying what passes or why, e.g. 'fetches users from', 'writes jobs to'"),
                "technology": s("e.g. 'JSON over HTTP', 'Tauri IPC', 'function call'; empty if plain code")
            })))
        }),
    )
}

fn journey_schema() -> Value {
    obj(
        &["name", "summary", "messages"],
        json!({
            "name": s("what the user or system is doing, as a verb phrase of 3 to 8 words"),
            "summary": s("2 sentences from the user's point of view: what starts it and what it ends with"),
            "messages": arr(obj(&["from", "to", "kind", "label", "caption"], json!({
                "from": s("a participant exactly as listed (its id or its name)"),
                "to": s("a participant exactly as listed (its id or its name)"),
                "kind": { "type": "string", "enum": ["call", "flow", "store", "return"], "description": "call inside the system; flow when it crosses a process or language boundary (HTTP, IPC, a queue); store when data comes to rest in an outside system; return for an answer going back" },
                "label": s("2 to 6 words on the arrow: what is asked or carried, e.g. 'GET /api/users', 'user list', 'saves the job'"),
                "caption": s("one plain sentence: what happens here and what is carried across")
            })))
        }),
    )
}

fn editor_schema() -> Value {
    obj(
        &["summary", "start_here", "callouts"],
        json!({
            "summary": s("3 to 5 sentences a newcomer reads first: what the system does, how the containers work together, where the data lives"),
            "start_here": arr(obj(&["element", "why"], json!({
                "element": s("a container, component or journey name exactly as listed"),
                "why": s("one sentence on why to look here first")
            }))),
            "callouts": arr(obj(&["title", "detail", "element"], json!({
                "title": s("up to 10 words"),
                "detail": s("1 or 2 sentences"),
                "element": s("a container, component or journey name it is about; empty if none")
            })))
        }),
    )
}

// ---- prompts -------------------------------------------------------------------------

const VOICE: &str = "Write for someone who will never read the code: plain verbs, concrete nouns, no filler (robust, seamless, powerful, leverage). Name things by what they do, not by their file names. Do not change anything in the repository.";

fn survey_prompt(g: &Graph, f: &atlas::Facts, base: &Atlas, traces: &[query::Trace], asked: &[FlowRequest]) -> String {
    let mut pk = String::new();
    for c in &base.containers {
        let (pid, files) = &f.packages[&c.package];
        let dir = &g.node(*pid).path;
        let tags = f.tags.get(&c.package).cloned().unwrap_or_default();
        pk.push_str(&format!(
            "- {} (dir `{}`): {} files, guessed {} in {}{}{}\n",
            c.package,
            if dir.is_empty() { "." } else { dir },
            files.len(),
            atlas::kind_label(&c.kind).to_lowercase(),
            if c.technology.is_empty() { "unknown technology".to_string() } else { c.technology.clone() },
            if tags.is_empty() { String::new() } else { format!("; touches: {}", tags.join(" ")) },
            match f.deps.get(&c.package) {
                Some(d) if !d.is_empty() => format!("; depends on {}", d.iter().take(8).cloned().collect::<Vec<_>>().join(", ")),
                _ => String::new(),
            }
        ));
    }
    let eps: String = query::endpoints(g).iter().take(40).map(|e| format!("- {} ({})\n", e.key, e.status)).collect();
    let flows: String = query::flows(g).iter().take(40).map(|fl| format!("- {} -> {} via {}\n", fl.from_path, fl.to_path, fl.label)).collect();
    let tr: String = traces.iter().map(|t| format!("- {} ({} boundaries: {})\n", t.entry_path, t.hops, t.via.join(", "))).collect();
    format!(
        "You are surveying a codebase to draw its C4 diagrams: the system in context (people and outside systems), its containers (the things that run: web apps, services, workers, command lines, libraries) and later their components. Read the README, the manifests and the entry points; skim more if the purpose is unclear. {VOICE}\n\
\n\
Repository: `{root}`, {files} files, {loc} lines.\n\
\n\
Packages the scanner found (each becomes a container unless it is tooling):\n{pk}\n\
Endpoints (routes, IPC commands, queue topics) and whether both sides exist in the repository:\n{eps}\n\
Places where data crosses between languages:\n{flows}\n\
Journeys the scanner can follow end to end ({pick}):\n{tr}{chosen}\n\
Answer with: the system's name, purpose and summary; the people (roles) who use it and which containers they use; the outside systems it depends on (databases, queues, file systems, third-party APIs) and which containers use them; for every package, what it runs as and a name for it; and the journeys to narrate.",
        root = g.root.rsplit('/').next().unwrap_or(&g.root),
        files = g.stats.files,
        loc = g.stats.loc,
        pick = if asked.is_empty() { format!("pick up to {} that best show what the system does, and name each by what the user is doing", traces.len().min(8)) } else { "for reference; the journeys are already chosen below, so answer with an empty list".to_string() },
        chosen = if asked.is_empty() { String::new() } else { format!("\nThe person steering this atlas has chosen the journeys to narrate:\n{}", asked.iter().map(|f| format!("- {}{}{}\n", if f.name.is_empty() { f.entry.clone() } else { f.name.clone() }, if f.entry.is_empty() || f.name.is_empty() { String::new() } else { format!(" (from {})", f.entry) }, if f.note.is_empty() { String::new() } else { format!(": {}", f.note) })).collect::<String>()) },
    )
}

fn container_prompt(g: &Graph, f: &atlas::Facts, a: &Atlas, c: &Container) -> String {
    let others: Vec<String> = a.containers.iter().filter(|o| o.id != c.id && !o.hidden).map(|o| format!("{} (package {})", o.name, o.package)).collect();
    let externals: Vec<String> = a.externals.iter().map(|x| x.name.clone()).collect();
    format!(
        "You are describing one container of a system for its C4 component diagram. The system is {sys}: {purpose}\n\
\n\
This container is {name} (package `{pkg}`, {kind}): {desc}\n\
\n\
Read its files (the repository is the current directory) and group every file into components: the parts a newcomer should know about, named by what they do. Most containers have 2 to 6 components. {VOICE}\n\
\n\
Its files:\n{brief}\n\
Other containers in the system: {others}.\nOutside systems: {externals}.\n\
\n\
Answer with: a description; its technology; 3 to 6 responsibilities; the components (every file above in exactly one component, paths spelled exactly as above); and the relationships between components, and from components to other containers or outside systems, with a label saying what passes. The engine will check every relationship against the imports and calls it found; a relationship it cannot see is kept but marked as a claim, so only write what the code shows.",
        sys = a.system.name,
        purpose = a.system.purpose,
        name = c.name,
        pkg = c.package,
        kind = atlas::kind_label(&c.kind).to_lowercase(),
        desc = c.description,
        brief = atlas::container_brief(g, f, c),
        others = if others.is_empty() { "none".into() } else { others.join("; ") },
        externals = if externals.is_empty() { "none known".into() } else { externals.join("; ") },
    )
}

/// The participants a narrator may draw arrows between: every element of the
/// atlas by id and name, so the answer tallies with the boxes on the diagrams.
fn participants_block(a: &Atlas) -> String {
    let mut s = String::new();
    for p in &a.people {
        s.push_str(&format!("- {} = {} (person)\n", p.id, p.name));
    }
    for c in a.containers.iter().filter(|c| !c.hidden) {
        s.push_str(&format!("- {} = {} ({})\n", c.id, c.name, atlas::kind_label(&c.kind).to_lowercase()));
        for k in &c.components {
            s.push_str(&format!("  - {} = {} / {} (component; files: {})\n", k.id, c.name, k.name, k.files.iter().take(6).cloned().collect::<Vec<_>>().join(", ")));
        }
    }
    for x in &a.externals {
        s.push_str(&format!("- {} = {} ({})\n", x.id, x.name, x.kind));
    }
    s
}

/// One journey: the participants, the engine's draft messages (from the trace, if
/// there is one), the person's note, and the ask.
fn journey_prompt(a: &Atlas, name_hint: &str, entry: &str, trace: Option<&query::Trace>, draft: &[atlas::Message], note: &str) -> String {
    let mut lines = String::new();
    if let Some(t) = trace {
        for (i, st) in t.steps.iter().enumerate() {
            let indent = "  ".repeat(st.depth as usize);
            let via = match (&st.via, &st.label) {
                (Some(EdgeKind::Flow), Some(l)) => format!(" <- crosses via {l}"),
                (Some(EdgeKind::Calls), _) => " <- called".into(),
                _ => String::new(),
            };
            let sinks = if st.sinks.is_empty() { String::new() } else { format!(" [{}]", st.sinks.join(", ")) };
            lines.push_str(&format!("{}. {indent}{}{}{}{}\n", i, st.path, st.line.map(|l| format!(":{l}")).unwrap_or_default(), via, sinks));
        }
    }
    let mut msgs = String::new();
    for (i, m) in draft.iter().enumerate() {
        msgs.push_str(&format!("{}. {} -> {} [{}] {}{}\n", i + 1, m.from, m.to, m.kind, m.label, if m.from_path.is_empty() { String::new() } else { format!(" ({} -> {})", m.from_path, if m.to_path.is_empty() { "…" } else { &m.to_path }) }));
    }
    let hint = if name_hint.is_empty() { trace.map(|t| format!("{} runs", t.name)).unwrap_or_else(|| "this flow runs".into()) } else { name_hint.to_lowercase() };
    let trace_block = if lines.is_empty() {
        if entry.is_empty() {
            "The scanner has no trace for this flow: find where it starts (a route, a command, a handler, an entry point) and follow the calls yourself.\n".to_string()
        } else {
            format!("It starts at `{entry}`.\n")
        }
    } else {
        format!("The scanner followed the calls from the entry point; indentation is call depth, `crosses via` marks data leaving one language for another, and square brackets mark where data comes to rest (a database, a queue, the file system):\n{lines}")
    };
    let draft_block = if msgs.is_empty() { String::new() } else { format!("\nThe engine's draft of the messages, over the participants below (keep what is right, reword it, drop what does not belong, add what the code shows):\n{msgs}") };
    let note_block = if note.trim().is_empty() { String::new() } else { format!("\nThe person steering this atlas says: {}\n", note.trim()) };
    format!(
        "You are narrating one journey through {sys} for its sequence diagram: what happens, in order, when {hint}. Read the code at each step (the repository is the current directory). {VOICE}\n\
\n\
{trace_block}{draft_block}\n\
Participants (every arrow joins two of these; use the ids or the names exactly as written; a component is finer than its container, so prefer it when you know which one):\n{participants}{note_block}\n\
Answer with a name (what the user is doing), a 2-sentence summary, and the messages in order: from, to, kind (call, flow, store or return), a short label for the arrow, and a one-sentence caption. The engine will check every message against the calls, imports and flows it found; a message it cannot see is kept but marked as a claim, so only write what the code shows.",
        sys = a.system.name,
        participants = participants_block(a),
    )
}

fn editor_prompt(g: &Graph, a: &Atlas) -> String {
    let containers: String = a
        .containers
        .iter()
        .filter(|c| !c.hidden)
        .map(|c| format!("- {} ({}; {}): {}\n  components: {}\n", c.name, atlas::kind_label(&c.kind).to_lowercase(), c.technology, c.description, c.components.iter().map(|k| k.name.clone()).collect::<Vec<_>>().join(", ")))
        .collect();
    let people: String = a.people.iter().map(|p| format!("- {}: {}\n", p.name, p.description)).collect();
    let externals: String = a.externals.iter().map(|x| format!("- {} ({}): {}\n", x.name, x.kind, x.description)).collect();
    let journeys: String = a.journeys.iter().map(|j| format!("- {}: {}\n", j.name, j.summary)).collect();
    let facts: String = atlas::engine_atlas(g).guide.callouts.iter().map(|c| format!("- {}: {}\n", c.title, c.detail)).collect();
    let claims: String = a.relationships.iter().filter(|r| r.source == "claimed").map(|r| format!("- {} -> {} ({})\n", r.from, r.to, r.label)).collect();
    format!(
        "You are editing the atlas of {sys} ({purpose}) after the field agents came back. {VOICE}\n\
\n\
Containers:\n{containers}\nPeople:\n{people}\nOutside systems:\n{externals}\nJourneys:\n{journeys}\n\
Facts the engine found in the code:\n{facts}{claims_block}\n\
Write: a summary (3 to 5 sentences a newcomer reads first: what the system does, how the containers work together, where the data lives); where to start (3 to 5 pointers to a container, component or journey, by name exactly as above, each with why); and callouts (3 to 6 things worth knowing, drawn from the facts above and from what stands out: gaps, cycles, the busiest part, surprising dependencies).",
        sys = a.system.name,
        purpose = a.system.purpose,
        claims_block = if claims.is_empty() { String::new() } else { format!("\nRelationships agents claimed that the code does not show (worth a callout if they matter):\n{claims}") },
    )
}

// ---- the run ------------------------------------------------------------------------

struct Resolver {
    by_name: HashMap<String, String>,
}

impl Resolver {
    fn new(a: &Atlas) -> Resolver {
        let mut by_name = HashMap::new();
        let mut put = |k: &str, v: &str| {
            by_name.entry(k.trim().to_lowercase()).or_insert_with(|| v.to_string());
        };
        for c in &a.containers {
            put(&c.id, &c.id);
            put(&c.package, &c.id);
            put(&c.name, &c.id);
            for k in &c.components {
                put(&k.id, &k.id);
                put(&format!("{}/{}", c.name, k.name), &k.id);
                put(&format!("{} / {}", c.name, k.name), &k.id);
            }
        }
        for c in &a.containers {
            for k in &c.components {
                put(&k.name, &k.id);
            }
        }
        for p in &a.people {
            put(&p.id, &p.id);
            put(&p.name, &p.id);
        }
        for x in &a.externals {
            put(&x.id, &x.id);
            put(&x.name, &x.id);
        }
        for j in &a.journeys {
            put(&j.id, &j.id);
            put(&j.name, &j.id);
        }
        Resolver { by_name }
    }

    fn get(&self, name: &str) -> Option<String> {
        self.by_name.get(&name.trim().to_lowercase()).cloned()
    }

    /// Prefer a component of `container` when the name is ambiguous.
    fn get_in(&self, container: &str, name: &str) -> Option<String> {
        self.by_name.get(&format!("{container}/{name}").to_lowercase()).cloned().or_else(|| self.get(name))
    }
}

fn kind_or(kind: &str, fallback: &str) -> String {
    if matches!(kind, "web" | "desktop" | "service" | "worker" | "cli" | "library" | "tooling") { kind.into() } else { fallback.into() }
}

fn ext_kind(kind: &str) -> String {
    if matches!(kind, "database" | "queue" | "filesystem" | "service" | "system") { kind.into() } else { "system".into() }
}

/// Run the discovery agents and verify what they wrote into an atlas.
pub fn discover(g: &Graph, opts: &Options, runner: &Runner, progress: &(dyn Fn(Progress) + Sync)) -> Result<(Atlas, Run)> {
    let started = Instant::now();
    let f = atlas::facts(g);
    let mut a = atlas::engine_atlas(g);
    a.source = "claude".into();
    a.model = Some(opts.model.clone());
    // The survey names people and outside systems; the engine only fills in what it misses.
    a.people.clear();
    a.externals.clear();
    a.relationships.retain(|r| !r.to.starts_with("x:") && !r.from.starts_with("p:"));
    a.journeys.clear();
    let traces = query::traces(g);
    let candidates: Vec<&query::Trace> = traces.iter().take(8).collect();
    let mut runs: Vec<AgentRun> = Vec::new();
    let mut claims: Words = HashMap::new();
    let field_containers = a.containers.iter().filter(|c| !c.hidden && !f.packages[&c.package].1.is_empty()).count();
    let planned = if opts.flows.is_empty() { candidates.len().min(opts.max_journeys) } else { opts.flows.len() };
    progress(Progress::Started { model: opts.model.clone(), containers: field_containers, journeys: planned });

    // ---- 1. survey
    progress(Progress::Stage { stage: "survey".into() });
    let call = AgentCall { role: "survey".into(), target: None, prompt: survey_prompt(g, &f, &a, &candidates.iter().map(|t| (*t).clone()).collect::<Vec<_>>(), &opts.flows), schema: survey_schema(), tools: true };
    let (run, value) = run_one(&call, "Surveying the repository", runner, progress);
    runs.push(run);
    let mut picks: Vec<(String, String)> = Vec::new();
    match value.and_then(|v| serde_json::from_value::<SurveyAnswer>(v).map_err(|e| tracing::warn!(error = %e, "survey answer did not match the schema")).ok()) {
        Some(sv) => {
            a.system = atlas::System { name: sv.system.name.trim().into(), purpose: sv.system.purpose.trim().into(), summary: sv.system.summary.trim().into() };
            for cs in sv.containers {
                if let Some(c) = a.containers.iter_mut().find(|c| c.package == cs.package.trim() || c.name.eq_ignore_ascii_case(cs.package.trim())) {
                    c.name = cs.name.trim().to_string();
                    c.kind = kind_or(cs.kind.trim(), &c.kind);
                    if !cs.technology.trim().is_empty() {
                        c.technology = cs.technology.trim().to_string();
                    }
                    c.description = cs.description.trim().to_string();
                    c.hidden = cs.hidden || c.kind == "tooling" || f.packages[&c.package].1.is_empty();
                }
            }
            for p in sv.people {
                let id = format!("p:{}", atlas::slug(&p.name));
                if a.people.iter().any(|x| x.id == id) {
                    continue;
                }
                a.people.push(Person { id: id.clone(), name: p.name.trim().into(), description: p.description.trim().into() });
                for u in p.uses {
                    if let Some(c) = a.containers.iter().find(|c| c.package.eq_ignore_ascii_case(u.container.trim()) || c.name.eq_ignore_ascii_case(u.container.trim())) {
                        claims.insert((id.clone(), c.id.clone()), (u.how, String::new()));
                    }
                }
            }
            for x in sv.externals {
                let id = format!("x:{}", atlas::slug(&x.name));
                if a.externals.iter().any(|e| e.id == id) {
                    continue;
                }
                a.externals.push(External { id: id.clone(), name: x.name.trim().into(), kind: ext_kind(x.kind.trim()), description: x.description.trim().into() });
                for u in x.used_by {
                    if let Some(c) = a.containers.iter().find(|c| c.package.eq_ignore_ascii_case(u.container.trim()) || c.name.eq_ignore_ascii_case(u.container.trim())) {
                        claims.insert((c.id.clone(), id.clone()), (u.how, u.technology));
                    }
                }
            }
            picks = sv.journeys.into_iter().map(|j| (j.entry.trim().to_string(), j.name.trim().to_string())).collect();
        }
        None => tracing::warn!("survey failed; the engine's names stand"),
    }
    // Journeys to narrate: the person's flows, else the survey's picks, else the longest traces.
    let mut chosen: Vec<Chosen> = Vec::new();
    if opts.flows.is_empty() {
        for (entry, name) in &picks {
            if let Some(t) = candidates.iter().find(|t| &t.entry_path == entry)
                && !chosen.iter().any(|c| c.entry == t.entry_path)
            {
                chosen.push(Chosen { id: atlas::journey_id(&t.entry_path, ""), entry: t.entry_path.clone(), name: name.clone(), note: String::new(), trace: Some((*t).clone()) });
            }
        }
        for t in &candidates {
            if chosen.len() >= opts.max_journeys {
                break;
            }
            if !chosen.iter().any(|c| c.entry == t.entry_path) {
                chosen.push(Chosen { id: atlas::journey_id(&t.entry_path, ""), entry: t.entry_path.clone(), name: String::new(), note: String::new(), trace: Some((*t).clone()) });
            }
        }
        chosen.truncate(opts.max_journeys);
    } else {
        for fr in &opts.flows {
            let c = choose_flow(g, &traces, fr);
            if !chosen.iter().any(|x| x.id == c.id) {
                chosen.push(c);
            }
        }
    }
    progress(Progress::SurveyDone { system: a.system.clone(), people: a.people.clone(), externals: a.externals.clone(), containers: a.containers.clone() });

    // ---- 2. the field: containers and journeys, in parallel
    progress(Progress::Stage { stage: "field".into() });
    enum Job {
        Container(usize),
        Journey(usize),
    }
    let mut jobs: Vec<(Job, AgentCall, String)> = Vec::new();
    for (i, c) in a.containers.iter().enumerate() {
        if c.hidden || f.packages[&c.package].1.is_empty() {
            continue;
        }
        jobs.push((Job::Container(i), AgentCall { role: "container".into(), target: Some(c.id.clone()), prompt: container_prompt(g, &f, &a, c), schema: container_schema(), tools: true }, format!("Reading {}", c.name)));
    }
    // Narrators draw over the engine's components: the container agents have not
    // answered yet, so the draft is re-resolved through its file paths once they have.
    let mut drafts: Vec<Vec<atlas::Message>> = Vec::new();
    for (i, c) in chosen.iter().enumerate() {
        let draft = c.trace.as_ref().map(|t| atlas::messages_from_trace(&a, t)).unwrap_or_default();
        jobs.push((Job::Journey(i), AgentCall { role: "journey".into(), target: Some(c.id.clone()), prompt: journey_prompt(&a, &c.name, &c.entry, c.trace.as_ref(), &draft, &c.note), schema: journey_schema(), tools: true }, format!("Following {}", c.title())));
        drafts.push(draft);
    }
    let total = jobs.len();
    let queue = Mutex::new(jobs);
    let results: Mutex<Vec<(usize, AgentRun, Option<Value>)>> = Mutex::new(Vec::new());
    let jobs_kind: Mutex<Vec<Option<Job>>> = Mutex::new((0..total).map(|_| None).collect());
    std::thread::scope(|sc| {
        for _ in 0..opts.parallel.max(1).min(total.max(1)) {
            sc.spawn(|| {
                loop {
                    let next = {
                        let mut q = queue.lock().unwrap();
                        if q.is_empty() { None } else { Some((q.len() - 1, q.remove(0))) }
                    };
                    let Some((_, (job, call, name))) = next else { break };
                    let idx = {
                        let mut jk = jobs_kind.lock().unwrap();
                        let i = jk.iter().position(|j| j.is_none()).unwrap();
                        jk[i] = Some(job);
                        i
                    };
                    let (run, value) = run_one(&call, &name, runner, progress);
                    results.lock().unwrap().push((idx, run, value));
                }
            });
        }
    });
    let results = results.into_inner().unwrap();
    let jobs_kind = jobs_kind.into_inner().unwrap();
    let mut journey_answers: Vec<Option<JourneyAnswer>> = (0..chosen.len()).map(|_| None).collect();
    for (idx, run, value) in results {
        let ok_value = run.ok;
        runs.push(run);
        match (&jobs_kind[idx], value) {
            (Some(Job::Container(ci)), Some(v)) if ok_value => match serde_json::from_value::<ContainerAnswer>(v) {
                Ok(ans) => {
                    apply_container(&mut a, *ci, ans, &mut claims);
                    let c = a.containers[*ci].clone();
                    progress(Progress::ContainerDone { container: c });
                }
                Err(e) => {
                    let c = &a.containers[*ci];
                    tracing::warn!(container = %c.name, error = %e, "container answer did not match the schema");
                    if let Some(r) = runs.last_mut() {
                        r.ok = false;
                        r.error = Some(format!("answer did not match the schema: {e}"));
                    }
                }
            },
            (Some(Job::Journey(ji)), Some(v)) if ok_value => match serde_json::from_value::<JourneyAnswer>(v) {
                Ok(ans) => journey_answers[*ji] = Some(ans),
                Err(e) => {
                    if let Some(r) = runs.last_mut() {
                        r.ok = false;
                        r.error = Some(format!("answer did not match the schema: {e}"));
                    }
                }
            },
            _ => {}
        }
    }
    if !runs.iter().any(|r| r.ok) {
        let why = runs.iter().filter_map(|r| r.error.clone()).next().unwrap_or_else(|| "no agents ran".into());
        bail!("every discovery agent failed: {why}");
    }
    // Components are settled: check them so journeys and the editor see real ids.
    a = atlas::verify(g, &a, Some(&claims));
    for (i, c) in chosen.iter().enumerate() {
        let j = match journey_answers[i].take() {
            Some(ans) => Some(journey_from_answer(&a, c, &drafts[i], ans)),
            None => c.trace.as_ref().and_then(|t| atlas::engine_journey(&a, t)).map(|mut j| {
                j.name = c.title();
                j.note = c.note.clone();
                j
            }),
        };
        if let Some(j) = j {
            let checked = atlas::upsert_journey(g, &a, j);
            if let Some(j) = checked.journeys.iter().find(|x| x.id == c.id) {
                progress(Progress::JourneyDone { journey: j.clone() });
            }
            a = checked;
        }
    }
    // A person's journeys ride along untouched; the check re-resolves their messages.
    for k in &opts.keep {
        if !a.journeys.iter().any(|j| j.id == k.id) {
            a.journeys.push(k.clone());
        }
    }

    // ---- 3. the editor
    progress(Progress::Stage { stage: "editor".into() });
    let call = AgentCall { role: "editor".into(), target: None, prompt: editor_prompt(g, &a), schema: editor_schema(), tools: false };
    let (run, value) = run_one(&call, "Writing the guide", runner, progress);
    runs.push(run);
    if let Some(ed) = value.and_then(|v| serde_json::from_value::<EditorAnswer>(v).ok()) {
        let res = Resolver::new(&a);
        if !ed.summary.trim().is_empty() {
            a.system.summary = ed.summary.trim().to_string();
        }
        a.guide.start_here = ed.start_here.into_iter().filter_map(|p| res.get(&p.element).map(|e| atlas::Pointer { element: e, why: p.why.trim().to_string() })).collect();
        a.guide.callouts = ed.callouts.into_iter().map(|c| atlas::Callout { title: c.title.trim().to_string(), detail: c.detail.trim().to_string(), element: res.get(&c.element) }).collect();
    }
    if a.guide.start_here.is_empty() && a.guide.callouts.is_empty() {
        a.guide = atlas::engine_atlas(g).guide;
    }

    // ---- 4. verify
    progress(Progress::Stage { stage: "verify".into() });
    let a = atlas::verify(g, &a, Some(&claims));
    progress(Progress::Verified { atlas: a.clone() });
    let cost = runs.iter().map(|r| r.cost_usd).sum();
    Ok((a, Run { model: opts.model.clone(), agents: runs, cost_usd: cost, secs: started.elapsed().as_secs_f64() }))
}

/// A journey the narrators will follow.
#[derive(Debug, Clone)]
struct Chosen {
    id: String,
    entry: String,
    name: String,
    note: String,
    trace: Option<query::Trace>,
}

impl Chosen {
    fn title(&self) -> String {
        if !self.name.is_empty() {
            self.name.clone()
        } else if let Some(t) = &self.trace {
            format!("From {}", t.name)
        } else {
            self.entry.clone()
        }
    }
}

/// Match a person's flow request to a trace: by entry path, or by a name that
/// spells an entry path. A request with no trace is followed from the code alone.
fn choose_flow(g: &Graph, traces: &[query::Trace], fr: &FlowRequest) -> Chosen {
    let want = if fr.entry.is_empty() { fr.name.trim() } else { fr.entry.trim() };
    let trace = traces
        .iter()
        .find(|t| t.entry_path == want)
        .cloned()
        .or_else(|| if want.contains('#') { g.find_by_path(want).and_then(|n| query::trace_from(g, n.id)) } else { None });
    let entry = trace.as_ref().map(|t| t.entry_path.clone()).unwrap_or_else(|| fr.entry.trim().to_string());
    let name = if fr.name.trim() == entry { String::new() } else { fr.name.trim().to_string() };
    Chosen { id: atlas::journey_id(&entry, &name), entry, name, note: fr.note.trim().to_string(), trace }
}

/// The narrator's messages, resolved onto atlas ids. A message that matches one
/// of the engine's draft messages keeps the draft's finer ids and file paths; the
/// check that follows decides what backs each one.
fn journey_from_answer(a: &Atlas, c: &Chosen, draft: &[atlas::Message], ans: JourneyAnswer) -> Journey {
    let res = Resolver::new(a);
    // The draft was drawn over the engine's components; re-resolve it through its paths.
    let comp_of = atlas::component_index(a);
    let redraw = |m: &atlas::Message| -> atlas::Message {
        let mut m = m.clone();
        let re = |id: &str, path: &str| -> String { if path.is_empty() { id.to_string() } else { comp_of.get(path.split('#').next().unwrap_or(path)).cloned().unwrap_or_else(|| id.to_string()) } };
        m.from = re(&m.from, &m.from_path);
        m.to = re(&m.to, &m.to_path);
        m
    };
    let draft: Vec<atlas::Message> = draft.iter().map(redraw).collect();
    let mut used: Vec<bool> = vec![false; draft.len()];
    let container = |id: &str| id.rsplit_once('/').map(|(c, _)| c.to_string()).unwrap_or_else(|| id.to_string());
    let mut messages: Vec<atlas::Message> = Vec::new();
    for m in ans.messages {
        let (Some(from), Some(to)) = (res.get(&m.from), res.get(&m.to)) else { continue };
        if from == to {
            continue;
        }
        let kind = if matches!(m.kind.as_str(), "call" | "flow" | "store" | "return") { m.kind.clone() } else { "call".to_string() };
        // exact, then container-grain, match against the draft
        let hit = draft.iter().enumerate().position(|(i, d)| !used[i] && d.from == from && d.to == to && d.kind == kind).or_else(|| draft.iter().enumerate().position(|(i, d)| !used[i] && container(&d.from) == container(&from) && container(&d.to) == container(&to) && d.kind == kind));
        let mut out = match hit {
            Some(i) => {
                used[i] = true;
                let mut d = draft[i].clone();
                d.depth = draft[i].depth;
                d
            }
            None => atlas::Message { from, to, label: String::new(), caption: String::new(), kind: kind.clone(), depth: messages.last().map(|l: &atlas::Message| l.depth).unwrap_or(0), source: String::new(), by: "claude".into(), from_path: String::new(), to_path: String::new() },
        };
        out.by = "claude".into();
        if !m.label.trim().is_empty() {
            out.label = m.label.trim().to_string();
        }
        if !m.caption.trim().is_empty() {
            out.caption = m.caption.trim().to_string();
        }
        if out.label.is_empty() {
            out.label = kind.clone();
        }
        messages.push(out);
    }
    if messages.is_empty() {
        messages = draft;
    }
    let mut j = Journey {
        id: c.id.clone(),
        name: if ans.name.trim().is_empty() { c.title() } else { ans.name.trim().to_string() },
        summary: ans.summary.trim().to_string(),
        entry: c.entry.clone(),
        messages,
        steps: vec![],
        source: "claude".into(),
        note: c.note.clone(),
    };
    j.steps = atlas::steps_of(&j.messages);
    j
}

/// Narrate one journey again, with a note: one agent, nothing else in the atlas
/// touched. The journey keeps its id; the check re-sources every message.
pub fn narrate_one(g: &Graph, a: &Atlas, id: &str, note: &str, opts: &Options, runner: &Runner, progress: &(dyn Fn(Progress) + Sync)) -> Result<(Atlas, Run)> {
    let started = Instant::now();
    let existing = a.journeys.iter().find(|j| j.id == id || j.name.eq_ignore_ascii_case(id) || j.entry == id).cloned();
    let traces = query::traces(g);
    let chosen = match &existing {
        Some(j) => {
            let trace = if j.entry.is_empty() { None } else { traces.iter().find(|t| t.entry_path == j.entry).cloned().or_else(|| g.find_by_path(&j.entry).and_then(|n| query::trace_from(g, n.id))) };
            Chosen { id: j.id.clone(), entry: j.entry.clone(), name: j.name.clone(), note: if note.trim().is_empty() { j.note.clone() } else { note.trim().to_string() }, trace }
        }
        None => choose_flow(g, &traces, &FlowRequest { entry: if id.contains('#') { id.to_string() } else { String::new() }, name: if id.contains('#') { String::new() } else { id.to_string() }, note: note.to_string() }),
    };
    // The draft is what stands now: the journey's own messages, else the trace's.
    let draft: Vec<atlas::Message> = match &existing {
        Some(j) if !j.messages.is_empty() => j.messages.clone(),
        _ => chosen.trace.as_ref().map(|t| atlas::messages_from_trace(a, t)).unwrap_or_default(),
    };
    progress(Progress::Stage { stage: "field".into() });
    let call = AgentCall { role: "journey".into(), target: Some(chosen.id.clone()), prompt: journey_prompt(a, &chosen.name, &chosen.entry, chosen.trace.as_ref(), &draft, &chosen.note), schema: journey_schema(), tools: true };
    let (run, value) = run_one(&call, &format!("Following {}", chosen.title()), runner, progress);
    let ans = match value {
        Some(v) if run.ok => serde_json::from_value::<JourneyAnswer>(v).map_err(|e| anyhow!("the narrator's answer did not match the schema: {e}"))?,
        _ => bail!("{}", run.error.clone().unwrap_or_else(|| "the narrator did not answer".into())),
    };
    let j = journey_from_answer(a, &chosen, &draft, ans);
    progress(Progress::Stage { stage: "verify".into() });
    let out = atlas::upsert_journey(g, a, j);
    if let Some(j) = out.journeys.iter().find(|x| x.id == chosen.id) {
        progress(Progress::JourneyDone { journey: j.clone() });
    } else {
        bail!("the narrator's messages joined nothing the atlas knows");
    }
    let cost = run.cost_usd;
    Ok((out, Run { model: opts.model.clone(), agents: vec![run], cost_usd: cost, secs: started.elapsed().as_secs_f64() }))
}

fn apply_container(a: &mut Atlas, ci: usize, ans: ContainerAnswer, claims: &mut Words) {
    let cid = a.containers[ci].id.clone();
    {
        let c = &mut a.containers[ci];
        if !ans.description.trim().is_empty() {
            c.description = ans.description.trim().to_string();
        }
        if !ans.technology.trim().is_empty() {
            c.technology = ans.technology.trim().to_string();
        }
        c.responsibilities = ans.responsibilities.iter().map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).collect();
        c.components = ans
            .components
            .iter()
            .map(|k| Component {
                id: atlas::component_id(&cid, k.name.trim()),
                name: k.name.trim().to_string(),
                description: k.description.trim().to_string(),
                technology: k.technology.trim().to_string(),
                responsibilities: k.responsibilities.iter().map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).collect(),
                files: k.files.iter().map(|p| p.trim().to_string()).collect(),
            })
            .collect();
    }
    let res = Resolver::new(a);
    for r in ans.relationships {
        let (Some(from), Some(to)) = (res.get_in(&a.containers[ci].name, &r.from), res.get_in(&a.containers[ci].name, &r.to)) else { continue };
        if from == to {
            continue;
        }
        claims.insert((from, to), (r.label, r.technology));
    }
}

fn run_one(call: &AgentCall, name: &str, runner: &Runner, progress: &(dyn Fn(Progress) + Sync)) -> (AgentRun, Option<Value>) {
    progress(Progress::AgentStarted { role: call.role.clone(), target: call.target.clone(), name: name.into() });
    let reads = Mutex::new(0u32);
    let sink = |ev: AgentEvent| {
        if matches!(ev, AgentEvent::Reading { .. }) {
            *reads.lock().unwrap() += 1;
        }
        progress(Progress::AgentActivity { role: call.role.clone(), target: call.target.clone(), activity: ev });
    };
    let t0 = Instant::now();
    let out = runner(call, &sink);
    let secs = t0.elapsed().as_secs_f64();
    let reads = reads.into_inner().unwrap();
    let (run, value) = match out {
        Ok((v, cost)) => (AgentRun { role: call.role.clone(), target: call.target.clone(), ok: true, error: None, cost_usd: cost, secs, reads }, Some(v)),
        Err(e) => (AgentRun { role: call.role.clone(), target: call.target.clone(), ok: false, error: Some(e.to_string()), cost_usd: 0.0, secs, reads }, None),
    };
    progress(Progress::AgentDone { role: run.role.clone(), target: run.target.clone(), ok: run.ok, cost_usd: run.cost_usd, secs: run.secs, error: run.error.clone() });
    (run, value)
}

// ---- the real runner -------------------------------------------------------------------

/// Run one agent through the Claude Code CLI, in the repository, read-only, with
/// streamed output so every file it opens is reported as it happens.
pub fn claude_runner<'a>(root: &'a Path, opts: &'a Options) -> impl Fn(&AgentCall, &(dyn Fn(AgentEvent) + Sync)) -> Result<(Value, f64)> + Sync + 'a {
    move |call: &AgentCall, sink: &(dyn Fn(AgentEvent) + Sync)| {
        let mut cmd = Command::new(&opts.claude);
        cmd.current_dir(root)
            .arg("-p")
            .arg(&call.prompt)
            .args(["--output-format", "stream-json", "--verbose", "--no-session-persistence", "--strict-mcp-config"])
            .args(["--model", &opts.model, "--effort", &opts.effort])
            .args(["--max-budget-usd", &format!("{:.2}", opts.budget_usd)])
            .arg("--json-schema")
            .arg(call.schema.to_string());
        // Read-only by construction: the agent has no tool that writes or runs anything.
        if call.tools {
            cmd.args(["--tools", "Read,Grep,Glob", "--allowedTools", "Read,Grep,Glob"]);
        } else {
            cmd.args(["--tools", ""]);
        }
        let mut child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("cannot start {} (install Claude Code, or set TERRARIUM_CLAUDE)", opts.claude.display()))?;
        let stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let err_t = std::thread::spawn(move || {
            use std::io::Read;
            let mut b = Vec::new();
            let _ = stderr.read_to_end(&mut b);
            String::from_utf8_lossy(&b).to_string()
        });
        let (tx, rx) = mpsc::channel::<String>();
        let out_t = std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let deadline = Instant::now() + opts.timeout;
        let mut result: Option<Value> = None;
        let root_s = root.to_string_lossy().to_string();
        loop {
            let now = Instant::now();
            if now >= deadline {
                let _ = child.kill();
                bail!("agent took longer than {}s and was stopped", opts.timeout.as_secs());
            }
            match rx.recv_timeout(deadline - now) {
                Ok(line) => {
                    let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                    if let Some(ev) = stream_event(&v, &root_s) {
                        sink(ev);
                    }
                    if v["type"] == "result" || v.get("total_cost_usd").is_some() {
                        result = Some(v);
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let _ = child.kill();
                    bail!("agent took longer than {}s and was stopped", opts.timeout.as_secs());
                }
            }
        }
        let _ = out_t.join();
        let _ = child.wait();
        let stderr_s = err_t.join().unwrap_or_default();
        let v = result.ok_or_else(|| anyhow!("claude exited without an answer: {}", stderr_s.lines().last().unwrap_or("no output")))?;
        let cost = v["total_cost_usd"].as_f64().unwrap_or(0.0);
        if v["is_error"].as_bool() == Some(true) {
            bail!("claude reported an error: {}", v["result"].as_str().or(v["subtype"].as_str()).unwrap_or("unknown"));
        }
        let answer = v
            .get("structured_output")
            .cloned()
            .filter(|x| !x.is_null())
            .or_else(|| v["result"].as_str().and_then(|s| serde_json::from_str::<Value>(s).ok()).filter(|x| x.is_object()))
            .ok_or_else(|| anyhow!("claude answered without the structured output"))?;
        Ok((answer, cost))
    }
}

/// A tool use in the stream, as something a person can watch.
pub fn stream_event(v: &Value, root: &str) -> Option<AgentEvent> {
    if v["type"] != "assistant" {
        return None;
    }
    for block in v["message"]["content"].as_array()? {
        if block["type"] != "tool_use" {
            continue;
        }
        let input = &block["input"];
        match block["name"].as_str()? {
            "Read" => {
                let p = input["file_path"].as_str()?;
                let rel = p.strip_prefix(root).map(|s| s.trim_start_matches('/')).unwrap_or(p);
                return Some(AgentEvent::Reading { path: rel.to_string() });
            }
            "Grep" | "Glob" => {
                let q = input["pattern"].as_str().unwrap_or("").to_string();
                return Some(AgentEvent::Searching { query: q });
            }
            _ => {}
        }
    }
    None
}
