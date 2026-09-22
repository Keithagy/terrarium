# Terrarium

A cozy macOS app for watching a repository's architecture and data flows, plus an
agent-friendly CLI that does the same work from a shell.

Terrarium scans a multi-language repository (Rust, TypeScript, JavaScript, Python, Go)
with tree-sitter, builds a graph of packages, files and symbols, resolves imports and
calls, and derives **cross-language data flows**: an HTTP call in TypeScript that lands
on a FastAPI route, a Tauri `invoke("cmd")` that reaches a `#[tauri::command]`, a Go
`http.HandleFunc` that a Rust client hits, queue producers and consumers that share a topic.

The layout runs as a wgpu compute shader on Metal; the renderer draws the graph with
instanced WebGL2 in a vibrancy-backed Tauri window.

```
crates/terrarium-core     scanning, graph model, queries, on-disk cache
crates/terrarium-layout   force-directed layout: wgpu compute (Metal) + rayon fallback
crates/terrarium-cli      `terrarium`: TOON output, structured errors, drives the app
app/src-tauri             the desktop app: commands, telemetry, agent bridge, screenshots
app/ui                    Vite + TypeScript frontend, WebGL2 renderer
fixtures/polyglot         a small four-language repo used by the tests
```

## Build

Requires Rust (the pinned toolchain in `rust-toolchain.toml` installs itself), Node 20+
and Xcode command line tools.

```sh
cd app/ui && npm install && npm run build && cd ../..
cargo build                     # terrarium (CLI) and terrarium-app (dev binary)
cargo test --workspace          # core, layout (GPU vs CPU parity) and CLI tests
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
terrarium traces                   # end-to-end paths: entry → calls → boundaries → db/fs/queue
terrarium trace web/src/app.ts#main  # one trace as a call tree
terrarium endpoints --gaps         # routes nothing calls, calls nothing serves
terrarium flows                    # the raw cross-language flow edges
terrarium hotspots                 # most connected files
terrarium boundaries --tag db      # everything touching a database
terrarium show src/api.ts          # one node with neighbours and tags
terrarium layout --backend gpu     # run the layout and report timings
terrarium doctor                   # GPU adapter, cache, app bridge
terrarium app launch .             # start the app on this repo
terrarium app screenshot           # PNG of the window
terrarium --json ...               # JSON instead of TOON
```

Output follows the AXI conventions: structured TOON on stdout, errors on stdout with a
`help` line, exit code 1 for errors and 2 for usage mistakes, nothing interactive.

## Driving the app as an agent

See [AGENTS.md](AGENTS.md). Short version: the app serves a localhost HTTP bridge
(`terrarium app ...` wraps it). It reports state, frame metrics, memory, span timings and
logs; it accepts selection, camera, filter and layout commands; it takes native
screenshots; it evaluates JavaScript in the webview; and it can click and type by
`data-testid`.

## Design

See [docs/DESIGN.md](docs/DESIGN.md) for the visual brief and tokens.
