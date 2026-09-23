# Terrarium

A cozy macOS app that builds a repository as a brick model, with a step-by-step manual
for reading it, plus an agent-friendly CLI that does the same work from a shell.

Terrarium scans a multi-language repository (Rust, TypeScript, JavaScript, Python, Go)
with tree-sitter, builds a graph of packages, files and symbols, resolves imports and
calls, and derives **cross-language data flows**: an HTTP call in TypeScript that lands
on a FastAPI route, a Tauri `invoke("cmd")` that reaches a `#[tauri::command]`, a Go
`http.HandleFunc` that a Rust client hits, queue producers and consumers that share a topic.

The graph then becomes a **brick model**. Each package is a district on a baseplate,
each file a building, and each function or type a brick in it, coloured by language.
Cross-language flows are amber bridges between districts. The model comes with a
**manual**: files go in dependency order, so every step only adds pieces that rest on
pieces already built, and playing it back is a reading order for the codebase. The
engine checks every joint (import or call) and reports weak joints, files that depend
on each other in a cycle ("interlocked"), loose files, and endpoints with no caller or
no handler.

**Design with Claude** hands the words to agents. One Claude agent per sub-build reads
that package's code (read-only tools) and writes its chapter: step groupings, titles and
captions. An assembler names the model. The engine then checks and repairs what they
wrote, so the manual can be imaginative and still hold together. Agents run through the
Claude Code CLI (`claude -p`) on `claude-opus-5-5` by default. The fixture costs about
$0.40 and takes 20 seconds.

The model is drawn with three.js (instanced bricks and studs, rendered on demand) in a
vibrancy-backed Tauri window.

```
crates/terrarium-core     scanning, graph model, queries, the build (order, check, repair,
                          geometry), the Claude designer, on-disk cache
crates/terrarium-cli      `terrarium`: TOON output, structured errors, drives the app
app/src-tauri             the desktop app: commands, telemetry, agent bridge, screenshots
app/ui                    Vite + TypeScript frontend, three.js brick renderer
fixtures/polyglot         a small four-language repo used by the tests
```

## Build

Requires Rust (the pinned toolchain in `rust-toolchain.toml` installs itself), Node 20+
and Xcode command line tools.

```sh
cd app/ui && npm install && npm run build && cd ../..
cargo build                     # terrarium (CLI) and terrarium-app (dev binary)
cargo test --workspace          # scanner, traces, build engine, designer (stand-in agents), CLI
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
terrarium build                    # the brick model: size, sub-builds, joint check
terrarium manual                   # the steps in reading order (--step N for one in full)
terrarium design                   # have Claude write the manual (spends money; --reset to undo)
terrarium traces                   # end-to-end paths: entry → calls → boundaries → db/fs/queue
terrarium trace web/src/app.ts#main  # one trace as a call tree
terrarium endpoints --gaps         # routes nothing calls, calls nothing serves
terrarium flows                    # the raw cross-language flow edges
terrarium hotspots                 # most connected files
terrarium boundaries --tag db      # everything touching a database
terrarium show src/api.ts          # one node with neighbours and tags
terrarium doctor                   # Claude Code, cache, app bridge
terrarium app launch .             # start the app on this repo
terrarium app screenshot           # PNG of the window
terrarium --json ...               # JSON instead of TOON
```

Output follows the AXI conventions: structured TOON on stdout, errors on stdout with a
`help` line, exit code 1 for errors and 2 for usage mistakes, nothing interactive.

## Driving the app as an agent

See [AGENTS.md](AGENTS.md). Short version: the app serves a localhost HTTP bridge
(`terrarium app ...` wraps it). It reports state, frame metrics, memory, span timings and
logs; it accepts selection, step, camera view, tab and design commands; it takes native
screenshots; it evaluates JavaScript in the webview; and it can click and type by
`data-testid`.

## Design

See [docs/DESIGN.md](docs/DESIGN.md) for the visual brief and tokens.

## License

MIT. See [LICENSE](LICENSE).
