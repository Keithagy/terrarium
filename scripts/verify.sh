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
expect "build" "check: \"holds together: 0 weak joints\"" "$T" --repo fixtures/polyglot build
expect "manual" "Add commands.rs" "$T" --repo fixtures/polyglot manual
expect "manual step" "rests on jobs.rs" "$T" --repo fixtures/polyglot manual --step 4
expect "doctor" "^claude:" "$T" doctor

step "launch app"
"$T" app quit >/dev/null 2>&1
sleep 0.5
expect "launch" "graph_loaded: true" "$T" app launch fixtures/polyglot

step "brick model"
sleep 0.5
expect "state" "tab: model" "$T" app state
expect "chips" "holds together" "$T" app ui
expect "step" "step_title: Add commands.rs" "$T" app step 4
expect "empty plate" "^step: 0" "$T" app step 0
expect "manual" "tab: manual" "$T" app tab manual
expect "manual page" "page-title" "$T" app ui
expect "finished" "^step: 10" "$T" app step
expect "view" "^view: top" "$T" app view top
expect "parts" "tab: parts" "$T" app tab parts
expect "parts ui" "parts-table" "$T" app ui
expect "design tab" "tab: design" "$T" app tab design
expect "design ui" "engine design" "$T" app ui

step "traces"
expect "endpoint" "tab: traces" "$T" app click "endpoint-http /api/jobs"
expect "trace ui" "Crosses 4 boundaries through TypeScript, Python, Rust and Go" "$T" app ui
expect "show on model" "trace_on_model: true" "$T" app click trace-show-model

step "bridge checks"
expect "select" "selection_path: web/src/api.ts" "$T" app select web/src/api.ts
expect "ui" "title: api.ts" "$T" app ui
expect "search" "fetchUsers" "$T" app search fetch
expect "reset" "trace_on_model: false" "$T" app reset
expect "metrics" "frontend_errors: 0" "$T" app metrics
expect "warnings" "^count: 0" "$T" app logs --level warn
expect "profile" "assemble_build" "$T" app profile
expect "screenshot" "^bytes: [0-9]{4,}" "$T" app screenshot --out "$OUT/app.png"
expect "eval" "result: [1-9]" "$T" app eval 'store.build.model.bricks.length'

step "quit"
"$T" app quit >/dev/null
sleep 0.7
expect "status" "running: false" "$T" app status

printf '\nVERIFY OK — artefacts in %s\n' "$OUT"
