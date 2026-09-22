#!/usr/bin/env bash
# End-to-end verification an agent can run unattended:
# build → unit tests → launch the app on the fixture → exercise the bridge → screenshot → quit.
set -uo pipefail
cd "$(dirname "$0")/.."

OUT=$(cd "$(dirname "${VERIFY_OUT:-target/verify}")" 2>/dev/null && pwd)/$(basename "${VERIFY_OUT:-target/verify}")
mkdir -p "$OUT"
export TERRARIUM_HOME=${TERRARIUM_HOME:-$OUT/home}
export TERRARIUM_PORT=${TERRARIUM_PORT:-47399}
T=target/debug/terrarium

step() { printf '\n== %s\n' "$*"; }
fail() { printf '\nVERIFY FAILED: %s\n' "$*"; "$T" app quit >/dev/null 2>&1; exit 1; }
# expect <label> <pattern> <command...>: run the command, save its output, require the pattern.
expect() {
  local label=$1 pattern=$2; shift 2
  local file="$OUT/${label// /_}.toon"
  "$@" > "$file" 2>&1
  if ! grep -qE "$pattern" "$file"; then
    cat "$file"
    fail "$label (expected /$pattern/)"
  fi
}

step "build ui"
(cd app/ui && npm run build > "$OUT/ui-build.log" 2>&1) || { cat "$OUT/ui-build.log"; fail "ui build"; }

step "build workspace"
cargo build --workspace > "$OUT/build.log" 2>&1 || { grep -E "^error" -A 8 "$OUT/build.log"; fail "cargo build"; }
if grep -qE "^warning: unused" "$OUT/build.log"; then grep -E "^warning" -A 4 "$OUT/build.log"; fail "build warnings"; fi

step "tests"
cargo test --workspace > "$OUT/test.log" 2>&1 || { grep -E "FAILED|panicked" -A 6 "$OUT/test.log"; fail "cargo test"; }
grep -E "^test result" "$OUT/test.log"

step "cli scan"
expect "scan" "^flows: 5" "$T" scan fixtures/polyglot
expect "flows" "ipc scan_repo" "$T" --repo fixtures/polyglot flows
expect "traces" "app.ts#main,4,typescript>python>rust>go" "$T" --repo fixtures/polyglot traces
expect "trace" "store.go#open:13\",call,db" "$T" --repo fixtures/polyglot trace web/src/app.ts#main
expect "endpoints" "api/reports,no-handler" "$T" --repo fixtures/polyglot endpoints --gaps
expect "layout" "^backend: (gpu|cpu)" "$T" --repo fixtures/polyglot layout --backend auto --iterations 50
grep -q "^backend: gpu" "$OUT/layout.toon" || echo "   (auto picked cpu for this small graph; gpu path covered by the layout crate's tests)"

step "launch app"
"$T" app quit >/dev/null 2>&1
sleep 0.5
expect "launch" "graph_loaded: true" "$T" app launch fixtures/polyglot

step "trace stage"
sleep 0.5
expect "stage" "stage: traces" "$T" app state
expect "trace ui" "Crosses 4 boundaries through TypeScript, Python, Rust and Go" "$T" app ui
expect "endpoint" "trace: native/src/lib.rs#run|trace: web/src/app.ts#main" "$T" app click "endpoint-http /api/jobs"
expect "show on map" "stage: map" "$T" app click trace-show-map

step "bridge checks"
expect "select" "selection_path: web/src/api.ts" "$T" app select web/src/api.ts
expect "ui" "title: api.ts" "$T" app ui
expect "focus" "^focus: [0-9]+" "$T" app focus web/src/api.ts
expect "level" "^level: symbol" "$T" app level symbol
expect "search" "fetchUsers" "$T" app search fetch
expect "filter" "nodes_visible" "$T" app filter --langs python --edges flow
expect "reset" "graph_loaded: true" "$T" app reset
expect "metrics" "frontend_errors: 0" "$T" app metrics
expect "warnings" "^count: 0" "$T" app logs --level warn
expect "profile" "layout_run" "$T" app profile
expect "screenshot" "^bytes: [0-9]{4,}" "$T" app screenshot --out "$OUT/app.png"
expect "eval" "result: [1-9]" "$T" app eval 'store.nodes.length'

step "quit"
"$T" app quit >/dev/null
sleep 0.7
expect "status" "running: false" "$T" app status

printf '\nVERIFY OK — artefacts in %s\n' "$OUT"
