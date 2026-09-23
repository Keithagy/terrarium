# Terrarium

A macOS app that draws a repository as C4 diagrams you can read without reading the
code, with Claude agents writing the words and an engine checking every arrow, plus an
agent-friendly CLI that does the same work from a shell.

Terrarium scans a multi-language repository (Rust, TypeScript, JavaScript, Python, Go)
with tree-sitter, builds a graph of packages, files and symbols, resolves imports and
calls, and derives **cross-language data flows**: an HTTP call in TypeScript that lands
on a FastAPI route, a Tauri `invoke("cmd")` that reaches a `#[tauri::command]`, a Go
`http.HandleFunc` that a Rust client hits, queue producers and consumers that share a topic.

The graph then becomes an **atlas**: the system in its context (who uses it, what it
talks to), the containers it runs as, the components inside each, and the code. The
engine draws it from the graph alone, so it is always available and always right about
what connects to what; names come from folders. **Journeys** follow one request across
the diagrams, numbered step by step.

**Discover with Claude** hands the words to agents. A surveyor reads the manifests and
entry points and decides what the system is, who uses it and what each package runs as.
One agent per container reads its code and groups it into components named by what they
do. One agent per journey narrates the crossings. An editor writes the summary, where to
start and what is worth knowing. You watch it happen: which file each agent is reading,
what it found as it lands on the diagram. Then the engine verifies every relationship
against the code: backed (with the evidence), from the survey, or claimed and marked as
such. Agents run through the Claude Code CLI (`claude -p`) on `claude-opus-5-5` by
default, with read-only tools.

The diagrams are SVG in a vibrancy-backed Tauri window, and export as Structurizr DSL.

```
crates/terrarium-core     scanning, graph model, queries, the atlas (engine draft, check,
                          journeys, export), the discovery orchestration, on-disk cache
crates/terrarium-cli      `terrarium`: TOON output, structured errors, drives the app
app/src-tauri             the desktop app: commands, telemetry, agent bridge, screenshots
app/ui                    Vite + TypeScript frontend: layout, diagrams, live discovery
fixtures/polyglot         a small four-language repo used by the tests
scripts/stand-in-claude.py  answers the agents' prompts for free, for tests and demos
```

## Build

Requires Rust (the pinned toolchain in `rust-toolchain.toml` installs itself), Node 20+
and Xcode command line tools.

```sh
cd app/ui && npm install && npm run build && cd ../..
cargo build                     # terrarium (CLI) and terrarium-app (dev binary)
cargo test --workspace          # scanner, atlas, discovery (stand-in agents), CLI
scripts/verify.sh               # builds, tests, launches the app, checks it over the bridge
```

Release bundle (`Terrarium.app` and a `.dmg` under `target/release/bundle/`):

```sh
cd app/ui && npm run tauri build
```

Hot-reloading development (Vite dev server + `cargo run`):

```sh
scripts/dev.sh
```

## CLI

```sh
terrarium                          # home: repo state, app status, next steps
terrarium scan .                   # scan and cache the graph
terrarium atlas                    # containers and relationships (--level context|components, --dsl)
terrarium discover                 # have Claude discover the atlas (spends money; --reset to undo)
terrarium traces                   # end-to-end paths: entry → calls → boundaries → db/fs/queue
terrarium trace web/src/app.ts#main  # one trace as a call tree
terrarium endpoints --gaps         # routes nothing calls, calls nothing serves
terrarium flows                    # the raw cross-language flow edges
terrarium hotspots                 # most connected files
terrarium boundaries --tag db      # everything touching a database
terrarium show src/api.ts          # one node with neighbours and tags
terrarium doctor                   # Claude Code, cache, app bridge
terrarium app launch .             # start the app on this repo
terrarium app discover             # discover in the app, watching it live
terrarium app screenshot           # PNG of the window
terrarium --json ...               # JSON instead of TOON
```

Output follows the AXI conventions: structured TOON on stdout, errors on stdout with a
`help` line, exit code 1 for errors and 2 for usage mistakes, nothing interactive.

## Driving the app as an agent

See [AGENTS.md](AGENTS.md). Short version: the app serves a localhost HTTP bridge
(`terrarium app ...` wraps it). It reports state, the atlas, frame metrics, memory, span
timings and logs; it accepts level, selection, journey and discovery commands; it takes
native screenshots; it evaluates JavaScript in the webview; and it can click and type by
`data-testid`.

## Design

See [docs/DESIGN.md](docs/DESIGN.md) for the visual brief, the tokens, and how an atlas
is made and checked.

## License

MIT. See [LICENSE](LICENSE).
