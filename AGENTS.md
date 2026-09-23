# Working on Terrarium as an agent

This file is the runbook for autonomous verification, debugging and profiling of the
Terrarium app. Everything here is scriptable; nothing needs a human at the screen.

## Orientation

```sh
terrarium                # home view: cached repo, app status, next steps
terrarium doctor         # toolchain, Claude Code, cache dir, bridge status
```

Binaries: `target/debug/terrarium` (CLI) and `target/debug/terrarium-app` (app). The CLI
finds the app binary next to itself; override with `--bin` or `TERRARIUM_APP`.

Shared state lives in `~/Library/Application Support/terrarium/` (override with
`TERRARIUM_HOME`, useful for isolated test runs):

```
graphs/<key>.json        cached graphs, one per scanned repo, plus index.json
graphs/<key>.atlas.json  the atlas Claude discovered for that repo, if any
logs/app.jsonl           every tracing event and span close, as JSON lines
bridge.json              {port, token, pid, started_at} of the running app
```

## The verify loop

```sh
scripts/verify.sh                       # full loop, exits non-zero on any failure
terrarium app launch fixtures/polyglot  # starts the app, waits for the scan
terrarium app state                     # backend + frontend state in one call
terrarium app screenshot --out s.png    # native WKWebView snapshot, includes the panels
terrarium app ui                        # semantic snapshot: panels, testids, texts, diagram nodes and edges
terrarium app logs --level warn         # anything that went wrong, backend and frontend
terrarium app metrics                   # fps, elements drawn, memory, counters
terrarium app profile                   # span timings: scan, assemble_atlas, discover_with_claude, screenshot
terrarium app quit
```

A healthy run shows `graph_loaded: true`, `counters.frontend_errors: 0` and no
`WARN`/`ERROR` events. `verify.sh` runs a discovery through `scripts/stand-in-claude.py`
(set as `TERRARIUM_CLAUDE`), which answers every agent prompt in a moment for free, so
the live discovery UI and the check are covered without spending anything.

## The atlas from the shell

```sh
terrarium atlas                                  # containers and their relationships
terrarium atlas --level context                  # people and outside systems
terrarium atlas --level components --container api
terrarium atlas --dsl                            # Structurizr DSL
terrarium discover                               # Claude discovers the atlas (spends money)
terrarium discover --reset                       # back to the engine's atlas
```

## Interacting with the app

```sh
terrarium app level context                      # context | containers | components | code
terrarium app level components --focus "Users API"
terrarium app level code --focus "HTTP routes"
terrarium app select "Users API"                 # a container, component, person, external, or a file path
terrarium app journey "Sign up" --step 3         # play a journey; --stop clears it
terrarium app search fetch                       # types into the search box, returns the results
terrarium app discover                           # Claude discovers the atlas in the app (blocks; spends money)
terrarium app discover --reset
terrarium app atlas                              # the atlas the app is showing (--dsl for DSL)
terrarium app click discover-claude              # any data-testid
terrarium app type search "handle"               # any input by data-testid
terrarium app eval 'terrarium.containers()'      # JavaScript in the webview; `store`, `terrarium` are in scope
terrarium app reset                              # containers level, no selection, no journey, fit
```

Every `data-testid` in the UI is listed by `terrarium app ui`. Stable ones:
`search`, `tab-map|journeys|guide`, `map-<element id>`, `journey-<journey id>`,
`level-context|containers|components|code`, `crumb-context|containers|components|code`,
`fit`, `c4-<element id>` (a box on the diagram), `discover-claude`, `copy-dsl`, `notes`,
`notes-close`, `notes-reset`, `stage-survey|field|editor|verify`, `agent-<role>:<target>`,
`card`, `card-title`, `card-path`, `card-zoom`, `card-close`, `rel-<from>><to>`,
`nb-<element id>`, `code-title`, `files`, `file-<path>`, `open-<path>`, `journey-bar`,
`journey-prev|next|all|close|range|title|count|caption`, `bridge`, `open-repo`,
`recent-repo`, `toast`.

Element ids: `s` (the system), `p:<slug>` (a person), `x:<slug>` (an outside system),
`c:<package slug>` (a container), `c:<package slug>/<component slug>` (a component),
`j:<entry slug>` (a journey). `select` and `level --focus` also accept names.

Bridge ops that change the picture (`level`, `select`, `journey`, `click`) answer after
the frame and the level ease, so a screenshot taken straight after shows the result.
`terrarium app ui` includes `diagram.nodes` (id, title, live state) and `diagram.edges`
(from, to, source, journey), so a test can check what is drawn without a screenshot.

## The bridge directly

`GET http://127.0.0.1:47311/` lists every endpoint. Send the token from `bridge.json`
as `x-terrarium-token` (or set `TERRARIUM_BRIDGE_NO_AUTH=1` before launching for local
experiments). Port and token can be forced with `TERRARIUM_PORT` / `TERRARIUM_TOKEN`.

```sh
TOKEN=$(jq -r .token ~/Library/Application\ Support/terrarium/bridge.json)
curl -s -H "x-terrarium-token: $TOKEN" localhost:47311/state | jq .ui.level
curl -s -H "x-terrarium-token: $TOKEN" localhost:47311/atlas | jq .atlas.report
curl -s -H "x-terrarium-token: $TOKEN" localhost:47311/atlas/dsl
curl -s -H "x-terrarium-token: $TOKEN" -X POST localhost:47311/level -d '{"level":"components","focus":"c:polyglot-api"}' -H 'content-type: application/json'
```

## Discovery, and what it costs

`terrarium discover` and `terrarium app discover` run Claude Code agents (`claude -p`
with read-only tools, a JSON schema per answer, streamed output) on
`claude-opus-5-5` by default. The fixture takes a few minutes and a few dollars; a real
repository scales with its containers. Set `TERRARIUM_DISCOVERY_MODEL` to change the
model and `TERRARIUM_CLAUDE` to point at a `claude` binary or a stand-in. The app emits
`discover:progress` events (`started`, `stage`, `agent_started`, `agent_activity`,
`agent_done`, `survey_done`, `container_done`, `journey_done`, `verified`) which the
field notes panel renders; `terrarium app ui` reports `discovery` and `notes` while a
run is on.

## Debugging

- Frontend logs go through `log()` in `app/ui/src/tauri.ts` and land in the same ring
  buffer and JSONL file as backend logs, target `ui`. Uncaught errors and unhandled
  rejections are captured automatically.
- `TERRARIUM_LOG=debug` (env filter syntax) raises verbosity for the app and the CLI;
  every agent activity event is logged at debug.
- `TERRARIUM_DEVTOOLS=1` opens the WebKit inspector on launch.
- `terrarium app eval` runs inside the page; `window.__terrarium` exposes `store`,
  `actions`, `snapshot()`, `ui()`, `select(id)`, `svg()` (the current diagram as SVG),
  `containers()`.
- `TERRARIUM_OPEN=<path>` (or a path as the first argument) scans a repo at startup.

## Code map

- Scanner: `crates/terrarium-core/src/scan.rs` (walk → packages → parse → nodes → tags →
  imports → calls → flows). Language extractors: `src/lang/*.rs`. Boundary heuristics:
  `src/tags.rs`. Queries and traces: `src/query.rs`. Cache (graphs and saved atlases):
  `src/cache.rs`.
- Atlas: `crates/terrarium-core/src/atlas.rs` (the C4 model, the engine's draft, the
  check against the graph, journeys from traces, Structurizr export). Discovery:
  `src/discovery.rs` (survey, field, editor; prompts and schemas; the streaming
  `claude -p` runner; the runner is injectable, and `tests/discovery.rs` drives it with
  stand-ins).
- App: `app/src-tauri/src/{commands,bridge,telemetry,snapshot}.rs`.
- UI: `app/ui/src/{main,atlas,layout,discovery,panels,bridge,store,types,tauri}.ts`.
  `layout.ts` is the layered layout; `atlas.ts` draws the diagrams and the code page;
  `discovery.ts` is the live run.

## Tests

```sh
cargo test --workspace            # ~10s; no Claude needed
cargo test -p terrarium-core      # scanner, atlas and discovery against fixtures/polyglot
```

Add fixture cases to `fixtures/polyglot` and assertions to
`crates/terrarium-core/tests/polyglot.rs` when touching extraction or flow detection, to
`tests/atlas.rs` when touching the engine's draft, the check or the export, and to
`tests/discovery.rs` when touching the orchestration or the merge of agent answers.
