# Working on Terrarium as an agent

This file is the runbook for autonomous verification, debugging and profiling of the
Terrarium app. Everything here is scriptable; nothing needs a human at the screen.

## Orientation

```sh
terrarium                # home view: cached repo, app status, next steps
terrarium doctor         # toolchain, GPU adapter, cache dir, bridge status
```

Binaries: `target/debug/terrarium` (CLI) and `target/debug/terrarium-app` (app). The CLI
finds the app binary next to itself; override with `--bin` or `TERRARIUM_APP`.

Shared state lives in `~/Library/Application Support/terrarium/` (override with
`TERRARIUM_HOME`, useful for isolated test runs):

```
graphs/<key>.json     cached graphs, one per scanned repo, plus index.json
logs/app.jsonl        every tracing event and span close, as JSON lines
bridge.json           {port, token, pid, started_at} of the running app
```

## The verify loop

```sh
scripts/verify.sh                       # full loop, exits non-zero on any failure
terrarium app launch fixtures/polyglot  # starts the app, waits for the scan
terrarium app state                     # backend + frontend state in one call
terrarium app screenshot --out s.png    # native WKWebView snapshot, includes the panels
terrarium app ui                        # semantic snapshot: panels, testids, texts, toasts
terrarium app logs --level warn         # anything that went wrong, backend and frontend
terrarium app metrics                   # fps, frame p50/p95, draw calls, memory, counters
terrarium app profile                   # span timings: scan, build_view, layout_run, screenshot
terrarium app quit
```

A healthy run shows `graph_loaded: true`, `layout.backend` of `gpu` or `cpu`,
`counters.frontend_errors: 0` and no `WARN`/`ERROR` events.

## Interacting

```sh
terrarium app select web/src/api.ts        # by path, id, or unique name; returns state
terrarium app focus web/src/api.ts         # expand a file into its symbols and centre it
terrarium app level symbol                 # package | file | symbol
terrarium app search fetch                 # types into the search box, returns the results
terrarium app filter --langs python,go --edges flow
terrarium app camera --fit                 # or --x --y --zoom
terrarium app layout --iterations 300 --backend gpu
terrarium app click level-package          # any data-testid
terrarium app type search "handle"         # any input by data-testid
terrarium app eval 'store.nodes.length'    # JavaScript in the webview; `store`, `renderer`, `terrarium` are in scope
terrarium app reset
```

Every `data-testid` in the UI is listed by `terrarium app ui`. Stable ones:
`search`, `tab-packages|flows|boundaries`, `level-package|file|symbol`, `lang-<lang>`,
`edge-imports|calls|flow`, `toggle-externals`, `node-<id>`, `flow-<from>-<to>`,
`card`, `card-title`, `card-path`, `card-close`, `open-file`, `expand`, `nb-<id>`,
`layout-status`, `bridge`, `open-repo`, `recent-repo`, `toast`.

## The bridge directly

`GET http://127.0.0.1:47311/` lists every endpoint. Send the token from `bridge.json`
as `x-terrarium-token` (or set `TERRARIUM_BRIDGE_NO_AUTH=1` before launching for local
experiments). Port and token can be forced with `TERRARIUM_PORT` / `TERRARIUM_TOKEN`.

```sh
TOKEN=$(jq -r .token ~/Library/Application\ Support/terrarium/bridge.json)
curl -s -H "x-terrarium-token: $TOKEN" localhost:47311/state | jq .ui.selection
curl -s -H "x-terrarium-token: $TOKEN" localhost:47311/screenshot > shot.png
curl -s -H "x-terrarium-token: $TOKEN" -X POST localhost:47311/select -d '{"node":"src/main.rs"}' -H 'content-type: application/json'
```

## Debugging

- Frontend logs go through `log()` in `app/ui/src/tauri.ts` and land in the same ring
  buffer and JSONL file as backend logs, target `ui`. Uncaught errors and unhandled
  rejections are captured automatically.
- `TERRARIUM_LOG=debug` (env filter syntax) raises verbosity for the app and the CLI.
- `TERRARIUM_DEVTOOLS=1` opens the WebKit inspector on launch.
- `terrarium app eval` runs inside the page; `window.__terrarium` exposes `store`,
  `renderer`, `actions`, `snapshot()`, `ui()`, `select(id)`.
- `TERRARIUM_OPEN=<path>` (or a path as the first argument) scans a repo at startup.

## Profiling

- `terrarium app profile` aggregates every tracing span since launch (`scan`, `walk`,
  `parse`, `resolve_imports`, `resolve_calls`, `derive_flows`, `build_view`,
  `layout_run`, `screenshot`) with count, total, mean, max and last durations.
- `terrarium app metrics` reports the renderer's frame time percentiles, draw calls
  (three per frame: patches, edges, nodes), nodes/edges/labels drawn, RSS memory.
- `terrarium layout --backend gpu|cpu --iterations N` benchmarks the layout outside the
  app; `--level symbol` on a big repo is the stress case.
- `logs/app.jsonl` includes span close events with `time.busy`/`time.idle`.

## Code map

- Scanner: `crates/terrarium-core/src/scan.rs` (walk → packages → parse → nodes → tags →
  imports → calls → flows). Language extractors: `src/lang/*.rs`. Boundary heuristics:
  `src/tags.rs`. Queries: `src/query.rs`. Cache: `src/cache.rs`.
- Layout: `crates/terrarium-layout/src/force.wgsl` is the algorithm; `cpu.rs` mirrors it.
- App: `app/src-tauri/src/{commands,bridge,layout_runner,telemetry,snapshot}.rs`.
- UI: `app/ui/src/{main,renderer,store,panels,bridge,tauri}.ts`.

## Tests

```sh
cargo test --workspace            # ~10s; layout parity test needs a GPU and skips without one
cargo test -p terrarium-core      # scanner against fixtures/polyglot
```

Add fixture cases to `fixtures/polyglot` and assertions to
`crates/terrarium-core/tests/polyglot.rs` when touching extraction or flow detection.
