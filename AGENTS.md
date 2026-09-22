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
terrarium app profile                   # span timings: scan, assemble_build, design_with_claude, screenshot
terrarium app quit
```

A healthy run shows `graph_loaded: true`, `build.weak: 0`,
`counters.frontend_errors: 0` and no `WARN`/`ERROR` events.

## Interacting

```sh
terrarium app select web/src/api.ts        # file or symbol by path, id, or unique name; returns state
terrarium app step 4                       # scrub the build: 0 = empty plate, omit = finished
terrarium app tab manual                   # model | manual | parts | traces | design
terrarium app view top --spin false        # iso | front | top, --spin, --fit
terrarium app search fetch                 # types into the search box, returns the results
terrarium app design                       # Claude designs the manual (blocks; spends money); --reset
terrarium app click stage-parts            # any data-testid
terrarium app type search "handle"         # any input by data-testid
terrarium app eval 'store.build.check'     # JavaScript in the webview; `store`, `scene`, `terrarium` are in scope
terrarium app reset                        # finished model, no selection or highlight, fit
```

Every `data-testid` in the UI is listed by `terrarium app ui`. Stable ones:
`search`, `stage-model|manual|parts|traces|design`, `view-iso|front|top|spin|fit`,
`timeline-first|play|last|range|title`, `speed-0.5|1|2|4`, `chip-pieces|steps|check|source`,
`design-claude`, `design-run`, `design-reset`, `check-weak`, `check-gaps`,
`tab-sub-builds|traces|endpoints`, `district-<sub-build id>`, `building-<file id>`,
`page-title`, `page-caption`, `page-prev`, `page-next`, `chapter-<id>`, `part-<file id>`,
`trace-<entry id>`, `step-<index>` (trace lane steps), `trace-show-model`, `trace-from`,
`clear-trace`, `endpoint-<key>` (e.g. `endpoint-http /api/users`), `parts-table`,
`card`, `card-title`, `card-path`, `card-step`, `card-close`, `open-file`, `nb-<id>`,
`canvas`, `bridge`, `open-repo`, `recent-repo`, `toast`.

Bridge ops that change the picture (`step`, `view`, `tab`, `click`) answer after the
frame and any drop-in animation, so a screenshot taken straight after shows the result.

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
  `scene`, `actions`, `snapshot()`, `ui()`, `select(id)`.
- `TERRARIUM_OPEN=<path>` (or a path as the first argument) scans a repo at startup.

## Profiling

- `terrarium app profile` aggregates every tracing span since launch (`scan`, `walk`,
  `parse`, `resolve_imports`, `resolve_calls`, `derive_flows`, `assemble_build`,
  `design_with_claude`, `screenshot`) with count, total, mean, max and last durations.
- `terrarium app metrics` reports the scene's frame time percentiles, draw calls (about
  ten, whatever the repo size: bricks and studs are instanced), pieces drawn, RSS memory.
  The scene renders on demand, so an idle window costs nothing.
- `logs/app.jsonl` includes span close events with `time.busy`/`time.idle`.

## Code map

- Scanner: `crates/terrarium-core/src/scan.rs` (walk → packages → parse → nodes → tags →
  imports → calls → flows). Language extractors: `src/lang/*.rs`. Boundary heuristics:
  `src/tags.rs`. Queries and traces: `src/query.rs`. Cache (graphs and saved designs):
  `src/cache.rs`.
- Build: `crates/terrarium-core/src/build.rs` (dependency order with cycles collapsed,
  engine design, joint check, repair, brick geometry). Designer:
  `src/designer.rs` (one `claude -p` agent per sub-build + an assembler; the runner is
  injectable, and `tests/designer.rs` drives it with stand-ins).
- App: `app/src-tauri/src/{commands,bridge,telemetry,snapshot}.rs`.
- UI: `app/ui/src/{main,bricks,store,panels,manual,parts,design,traces,bridge,tauri}.ts`.
  `bricks.ts` is the three.js scene.

## Tests

```sh
cargo test --workspace            # ~10s; no GPU or Claude needed
cargo test -p terrarium-core      # scanner against fixtures/polyglot
```

Add fixture cases to `fixtures/polyglot` and assertions to
`crates/terrarium-core/tests/polyglot.rs` when touching extraction or flow detection, and
to `tests/build.rs` when touching the order, check, repair or geometry.

`terrarium design` spends real money (the fixture: about $0.40 on `claude-opus-5-5`), so
verify.sh never runs it. Set `TERRARIUM_DESIGN_MODEL` to change the model and
`TERRARIUM_CLAUDE` to point at a `claude` binary.
