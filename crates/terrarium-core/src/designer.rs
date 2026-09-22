//! Designs written by Claude: generative UI that has to hold together.
//!
//! The engine lays down an order that respects every dependency. Then one agent
//! per sub-build reads that package's files and writes the part of the manual a
//! person needs: how to group the files into steps, what to call each step, and
//! a caption that explains what the files do and why they come at that point. An
//! assembler agent names the whole model. The engine merges the pieces, checks
//! every joint and repairs anything placed too early, so the agents are free to
//! be imaginative about words and grouping but cannot make a model that falls
//! apart.
//!
//! Agents run through the Claude Code CLI (`claude -p`), with read-only tools and
//! a JSON schema for their answer.

use crate::build::{self, Design, Step, SubBuild};
use crate::model::*;
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";

#[derive(Debug, Clone)]
pub struct Options {
    pub model: String,
    /// `low` … `max`. Opus 5.5 defaults to medium; captions read better at high.
    pub effort: String,
    /// Agents running at once.
    pub parallel: usize,
    /// Spending cap per agent, in US dollars.
    pub budget_usd: f64,
    pub timeout: Duration,
    pub claude: PathBuf,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            model: std::env::var("TERRARIUM_DESIGN_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string()),
            effort: "high".into(),
            parallel: 4,
            budget_usd: 2.0,
            timeout: Duration::from_secs(600),
            claude: find_claude(),
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
    /// Sub-build id, or `assembler`.
    pub role: String,
    pub prompt: String,
    pub schema: Value,
    /// Whether the agent may read the repository.
    pub tools: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentRun {
    pub role: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub cost_usd: f64,
    pub secs: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Run {
    pub model: String,
    pub agents: Vec<AgentRun>,
    pub cost_usd: f64,
    pub secs: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Progress {
    Started { agents: usize, model: String },
    AgentDone { role: String, ok: bool, done: usize, of: usize, cost_usd: f64 },
    Assembling,
}

/// What runs an agent: the real CLI, or a stand-in in tests. Returns the agent's
/// structured answer and what it cost.
pub type Runner<'a> = dyn Fn(&AgentCall) -> Result<(Value, f64)> + Sync + 'a;

#[derive(Deserialize)]
struct SubAnswer {
    name: String,
    blurb: String,
    steps: Vec<StepAnswer>,
}

#[derive(Deserialize)]
struct StepAnswer {
    title: String,
    caption: String,
    files: Vec<String>,
}

#[derive(Deserialize)]
struct Assembled {
    title: String,
    summary: String,
}

fn sub_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["name", "blurb", "steps"],
        "properties": {
            "name": { "type": "string", "description": "2 to 4 words naming what this part of the codebase is" },
            "blurb": { "type": "string", "description": "one sentence on what it does for the rest of the repository" },
            "steps": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["title", "caption", "files"],
                    "properties": {
                        "title": { "type": "string" },
                        "caption": { "type": "string" },
                        "files": { "type": "array", "items": { "type": "string" } }
                    }
                }
            }
        }
    })
}

fn assembler_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["title", "summary"],
        "properties": {
            "title": { "type": "string" },
            "summary": { "type": "string" }
        }
    })
}

fn sub_prompt(g: &Graph, sb: &SubBuild, steps: &[&Step], deps: &build::Deps) -> String {
    let mut files = String::new();
    let mut n = 0;
    for s in steps {
        for p in &s.files {
            n += 1;
            let Some(node) = g.find_by_path(p) else { continue };
            let syms: Vec<&str> = g
                .nodes
                .iter()
                .filter(|x| x.kind == NodeKind::Symbol && x.parent == Some(node.id))
                .map(|x| x.name.as_str())
                .take(12)
                .collect();
            let rests: Vec<&str> = deps.on.get(&node.id).map(|v| v.iter().map(|b| g.node(*b).path.as_str()).collect()).unwrap_or_default();
            files.push_str(&format!(
                "{n}. {p} ({} lines)\n   parts: {}\n   rests on: {}\n",
                node.loc,
                if syms.is_empty() { "none".into() } else { syms.join(", ") },
                if rests.is_empty() { "nothing in the repository".into() } else { rests.join(", ") }
            ));
        }
    }
    format!(
        "You are writing one chapter of a build manual for a codebase. The codebase is shown as a brick model: each package is a district, each file a building, each function or type a brick. The manual adds files in an order where every file only rests on files built before it, so reading the manual in order is a good way to learn the code.\n\
\n\
This chapter is the package `{pkg}`. The repository is in the current directory; read any file you need to understand what it does. Do not change anything.\n\
\n\
Its files, in the order the engine has checked (a file may rest on files in other packages that are built earlier):\n\
{files}\n\
Write:\n\
- `name`: 2 to 4 words saying what this package is for a newcomer (not just its package name).\n\
- `blurb`: one sentence on what it does for the rest of the repository.\n\
- `steps`: group the files into steps of 1 to 3 files, keeping the order above. Put files together when they make sense as one idea. Every file above appears in exactly one step, spelled exactly as above.\n\
  - `title`: up to 8 words, starting with a verb, saying what the step adds (\"Define the graph every query reads\").\n\
  - `caption`: 1 or 2 plain sentences: what these files do, and why they come at this point (what they rest on, or what will rest on them).\n\
\n\
Be concrete and specific to this code. Avoid filler words like robust, seamless or powerful.",
        pkg = sb.package,
    )
}

fn assembler_prompt(g: &Graph, subs: &[(String, String, usize)]) -> String {
    let list: String = subs.iter().map(|(n, b, c)| format!("- {n} ({c} files): {b}\n")).collect();
    format!(
        "A codebase has been designed as a brick model: districts are packages, buildings are files, and a build manual adds them in dependency order. The districts are:\n\
{list}\n\
The repository folder is `{root}` with {files} files and {flows} places where data crosses between languages.\n\
\n\
Write:\n\
- `title`: a short, memorable name for the whole model (2 to 5 words) that says what the software is, the way a brick set is named after what it builds.\n\
- `summary`: 2 sentences a newcomer reads first: what the software does and how the districts fit together.",
        root = g.root.rsplit('/').next().unwrap_or(&g.root),
        files = g.stats.files,
        flows = g.stats.flows,
    )
}

/// Run the design agents and assemble their answers into a design that holds together.
pub fn design(g: &Graph, opts: &Options, runner: &Runner, progress: &(dyn Fn(Progress) + Sync)) -> Result<(Design, Run)> {
    let started = Instant::now();
    let base = build::engine_design(g);
    let d = build::deps(g);
    let jobs: Vec<(usize, AgentCall)> = base
        .sub_builds
        .iter()
        .enumerate()
        .map(|(i, sb)| {
            let steps: Vec<&Step> = base.steps.iter().filter(|s| s.sub_build == sb.id).collect();
            (i, AgentCall { role: sb.id.clone(), prompt: sub_prompt(g, sb, &steps, &d), schema: sub_schema(), tools: true })
        })
        .collect();
    progress(Progress::Started { agents: jobs.len() + 1, model: opts.model.clone() });
    let results: Mutex<Vec<(usize, AgentRun, Option<Value>)>> = Mutex::new(Vec::new());
    let queue = Mutex::new(jobs.clone());
    let total = jobs.len();
    std::thread::scope(|s| {
        for _ in 0..opts.parallel.max(1).min(total.max(1)) {
            s.spawn(|| {
                loop {
                    let Some((i, call)) = queue.lock().unwrap().pop() else { break };
                    let t0 = Instant::now();
                    let out = runner(&call);
                    let secs = t0.elapsed().as_secs_f64();
                    let (run, value) = match out {
                        Ok((v, cost)) => (AgentRun { role: call.role.clone(), ok: true, error: None, cost_usd: cost, secs }, Some(v)),
                        Err(e) => (AgentRun { role: call.role.clone(), ok: false, error: Some(e.to_string()), cost_usd: 0.0, secs }, None),
                    };
                    let mut r = results.lock().unwrap();
                    r.push((i, run.clone(), value));
                    progress(Progress::AgentDone { role: run.role, ok: run.ok, done: r.len(), of: total, cost_usd: run.cost_usd });
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by_key(|r| r.0);
    if !results.iter().any(|r| r.1.ok) {
        let why = results.iter().filter_map(|r| r.1.error.clone()).next().unwrap_or_else(|| "no agents ran".into());
        bail!("every design agent failed: {why}");
    }

    // Merge: each sub-build takes its agent's steps, or keeps the engine's.
    let pkg_files: HashMap<&str, HashSet<&str>> = base
        .sub_builds
        .iter()
        .map(|sb| (sb.id.as_str(), base.steps.iter().filter(|s| s.sub_build == sb.id).flat_map(|s| s.files.iter().map(|f| f.as_str())).collect()))
        .collect();
    let mut subs = base.sub_builds.clone();
    let mut steps: Vec<Step> = Vec::new();
    let mut runs: Vec<AgentRun> = Vec::new();
    for (i, mut run, value) in results {
        let sb = &base.sub_builds[i];
        let answer = value.and_then(|v| serde_json::from_value::<SubAnswer>(v).map_err(|e| run.error = Some(format!("answer did not match the schema: {e}"))).ok());
        match answer {
            Some(a) => {
                subs[i].name = a.name.trim().to_string();
                subs[i].blurb = a.blurb.trim().to_string();
                let own = &pkg_files[sb.id.as_str()];
                for s in a.steps {
                    // Files outside the package belong to another chapter; the engine places them.
                    let files: Vec<String> = s.files.into_iter().filter(|f| own.contains(f.as_str())).collect();
                    if !files.is_empty() {
                        steps.push(Step { sub_build: sb.id.clone(), title: s.title.trim().to_string(), caption: s.caption.trim().to_string(), files });
                    }
                }
            }
            None => {
                run.ok = false;
                steps.extend(base.steps.iter().filter(|s| s.sub_build == sb.id).cloned());
            }
        }
        runs.push(run);
    }
    // Chapters interleave in the engine's order: a step goes where its first file would.
    let rank: HashMap<&str, usize> = base.steps.iter().enumerate().flat_map(|(i, s)| s.files.iter().map(move |f| (f.as_str(), i))).collect();
    steps.sort_by_key(|s| s.files.iter().filter_map(|f| rank.get(f.as_str())).min().copied().unwrap_or(usize::MAX));

    progress(Progress::Assembling);
    let summaries: Vec<(String, String, usize)> = subs.iter().map(|s| (s.name.clone(), s.blurb.clone(), pkg_files[s.id.as_str()].len())).collect();
    let call = AgentCall { role: "assembler".into(), prompt: assembler_prompt(g, &summaries), schema: assembler_schema(), tools: false };
    let t0 = Instant::now();
    let (title, summary) = match runner(&call).and_then(|(v, cost)| Ok((serde_json::from_value::<Assembled>(v)?, cost))) {
        Ok((a, cost)) => {
            runs.push(AgentRun { role: "assembler".into(), ok: true, error: None, cost_usd: cost, secs: t0.elapsed().as_secs_f64() });
            (a.title.trim().to_string(), a.summary.trim().to_string())
        }
        Err(e) => {
            runs.push(AgentRun { role: "assembler".into(), ok: false, error: Some(e.to_string()), cost_usd: 0.0, secs: t0.elapsed().as_secs_f64() });
            (base.title.clone(), base.summary.clone())
        }
    };
    let design = Design { schema: build::DESIGN_SCHEMA, source: "claude".into(), model: Some(opts.model.clone()), scanned_at: g.scanned_at.clone(), title, summary, sub_builds: subs, steps };
    // Whatever the agents wrote, it has to hold together.
    let (design, _) = build::repair(g, &design);
    let cost = runs.iter().map(|r| r.cost_usd).sum();
    Ok((design, Run { model: opts.model.clone(), agents: runs, cost_usd: cost, secs: started.elapsed().as_secs_f64() }))
}

/// Run one agent through the Claude Code CLI, in the repository, read-only.
pub fn claude_runner<'a>(root: &'a Path, opts: &'a Options) -> impl Fn(&AgentCall) -> Result<(Value, f64)> + Sync + 'a {
    move |call: &AgentCall| {
        let mut cmd = Command::new(&opts.claude);
        cmd.current_dir(root)
            .arg("-p")
            .arg(&call.prompt)
            .args(["--output-format", "json", "--no-session-persistence", "--strict-mcp-config"])
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
        let child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("cannot start {} (install Claude Code, or set TERRARIUM_CLAUDE)", opts.claude.display()))?;
        let out = wait_with_timeout(child, opts.timeout)?;
        let v: Value = serde_json::from_slice(&out.stdout).map_err(|_| {
            let err = String::from_utf8_lossy(&out.stderr);
            anyhow!("claude exited without an answer: {}", err.lines().last().unwrap_or("no output"))
        })?;
        let cost = v["total_cost_usd"].as_f64().unwrap_or(0.0);
        if v["is_error"].as_bool() == Some(true) {
            bail!("claude reported an error: {}", v["result"].as_str().or(v["subtype"].as_str()).unwrap_or("unknown"));
        }
        let answer = v.get("structured_output").cloned().filter(|x| !x.is_null()).ok_or_else(|| anyhow!("claude answered without the structured output"))?;
        Ok((answer, cost))
    }
}

fn wait_with_timeout(mut child: std::process::Child, timeout: Duration) -> Result<std::process::Output> {
    use std::io::Read;
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out_t = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = stdout.read_to_end(&mut b);
        b
    });
    let err_t = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = stderr.read_to_end(&mut b);
        b
    });
    let t0 = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if t0.elapsed() > timeout {
            let _ = child.kill();
            bail!("agent took longer than {}s and was stopped", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    Ok(std::process::Output { status, stdout: out_t.join().unwrap_or_default(), stderr: err_t.join().unwrap_or_default() })
}
