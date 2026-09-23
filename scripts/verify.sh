#!/usr/bin/env bash
# End-to-end verification an agent can run unattended:
# build → unit tests → CLI on the fixture → launch the app → exercise the bridge →
# the scout's proposals and a discovery with the stand-in claude (nothing spent) →
# screenshot → quit.
set -uo pipefail
cd "$(dirname "$0")/.."

OUT=$(cd "$(dirname "${VERIFY_OUT:-target/verify}")" 2>/dev/null && pwd)/$(basename "${VERIFY_OUT:-target/verify}")
mkdir -p "$OUT"
export TERRARIUM_HOME=${TERRARIUM_HOME:-$OUT/home}
export TERRARIUM_PORT=${TERRARIUM_PORT:-47399}
export TERRARIUM_CLAUDE=${TERRARIUM_CLAUDE:-$PWD/scripts/stand-in-claude.py}
export STANDIN_DELAY=${STANDIN_DELAY:-0}
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
# expect_soon <label> <pattern> <command...>: like expect, retried for up to 10s.
expect_soon() {
  local label=$1 pattern=$2; shift 2
  local file="$OUT/${label// /_}.toon"
  for _ in $(seq 1 20); do
    "$@" > "$file" 2>&1
    if grep -qE "$pattern" "$file"; then return 0; fi
    sleep 0.5
  done
  cat "$file"
  fail "$label (expected /$pattern/)"
}

step "build ui"
(cd app/ui && npm run build > "$OUT/ui-build.log" 2>&1) || { cat "$OUT/ui-build.log"; fail "ui build"; }

step "build workspace"
cargo build --workspace > "$OUT/build.log" 2>&1 || { grep -E "^error" -A 8 "$OUT/build.log"; fail "cargo build"; }
if grep -qE "^warning: unused" "$OUT/build.log"; then grep -E "^warning" -A 4 "$OUT/build.log"; fail "build warnings"; fi

step "tests"
cargo test --workspace > "$OUT/test.log" 2>&1 || { grep -E "FAILED|panicked" -A 6 "$OUT/test.log"; fail "cargo test"; }
grep -E "^test result" "$OUT/test.log"

step "cli scan and queries"
expect "scan" "^flows: 5" "$T" scan fixtures/polyglot
expect "flows" "ipc scan_repo" "$T" --repo fixtures/polyglot flows
expect "traces" "app.ts#main,4,typescript>python>rust>go" "$T" --repo fixtures/polyglot traces
expect "trace" "store.go#open:13\",call,db" "$T" --repo fixtures/polyglot trace web/src/app.ts#main
expect "endpoints" "api/reports,no-handler" "$T" --repo fixtures/polyglot endpoints --gaps
expect "doctor" "^claude:" "$T" doctor

step "cli atlas"
"$T" --repo fixtures/polyglot discover --reset > /dev/null
expect "atlas" "check: \"24 relationships backed by code, 2 from the survey, 0 claimed\"" "$T" --repo fixtures/polyglot atlas
expect "atlas containers" "\"c:polyglot-native\",Polyglot Native,desktop" "$T" --repo fixtures/polyglot atlas
expect "atlas context" "Database \(DATABASE_URL\),database" "$T" --repo fixtures/polyglot atlas --level context
expect "atlas components" "Polyglot Api / Services,Queue \(REDIS_URL\),puts work on" "$T" --repo fixtures/polyglot atlas --level components --container polyglot-api
expect "atlas dsl" "systemContext s" "$T" --repo fixtures/polyglot atlas --dsl

step "cli journeys as sequences"
expect "journey" "Polyglot Web,Polyglot Api,flow,http /api/users,code" "$T" --repo fixtures/polyglot atlas --journey j:web-src-app-ts-main
expect "journey components" "Polyglot Web / Api,Polyglot Api" "$T" --repo fixtures/polyglot atlas --journey j:web-src-app-ts-main --level components --container polyglot-web
expect "journey mermaid" "sequenceDiagram" "$T" --repo fixtures/polyglot atlas --journey j:web-src-app-ts-main --mermaid

step "cli discover (stand-in claude, nothing spent)"
expect "propose" "^proposed: 4 flows" "$T" --repo fixtures/polyglot propose
expect "propose traced" "Follow main to the end,web/src/app.ts#main,trace,4" "$T" --repo fixtures/polyglot propose
expect "propose endpoint" "Serve http /api/health,api/main.py#health,endpoint,0,Polyglot Api" "$T" --repo fixtures/polyglot propose
expect "propose untraced" "Nightly cleanup,\(the narrator finds it\),none" "$T" --repo fixtures/polyglot propose
expect "propose help" "terrarium discover --flow" "$T" --repo fixtures/polyglot propose
expect "discover" "system: Polyglot Town" "$T" --repo fixtures/polyglot discover
expect "discover scouted" "\"j:api-main-py-health\",.*api/main.py#health" "$T" --repo fixtures/polyglot atlas
expect "discover scouted untraced" "\"j:nightly-cleanup\"" "$T" --repo fixtures/polyglot atlas
expect "discover scouted why" "why: Something outside this repository calls it." "$T" --repo fixtures/polyglot atlas --journey j:api-main-py-health
expect "discover check" "claimed" "$T" --repo fixtures/polyglot atlas
expect "discover steered" "\"j:nightly-cleanup\",Follow one request \(there is no trace\)" "$T" --repo fixtures/polyglot discover --flow "web/src/app.ts#main :: watch the users list" --flow "Nightly cleanup :: there is no trace"
expect "discover kept proposal" "\"j:api-main-py-health\",Follow one request \(say who polls it\)" "$T" --repo fixtures/polyglot discover --flow "Serve the health check @ api/main.py#health :: say who polls it"
expect "narrate" "source: claude" "$T" --repo fixtures/polyglot narrate j:web-src-app-ts-main --note "say more"
expect "discover reset" "reset to the engine's atlas" "$T" --repo fixtures/polyglot discover --reset

step "launch app"
"$T" app quit >/dev/null 2>&1
sleep 0.5
expect "launch" "graph_loaded: true" "$T" app launch fixtures/polyglot

step "diagrams"
sleep 0.5
expect "state" "level: containers" "$T" app state
expect "chips" "engine draft" "$T" app ui
expect "context" "level: context" "$T" app level context
expect "components" "focus_name: Polyglot Api" "$T" app level components --focus polyglot-api
expect "select" "selection_name: Services" "$T" app select Services
expect "card" "title: Services" "$T" app ui
expect "code" "level: code" "$T" app level code --focus "c:polyglot-api/services"
expect "code ui" "code-title" "$T" app ui
expect "journey" "journey_step: 2" "$T" app journey "j:web-src-app-ts-main" --step 2
expect "journey ui" "journey-caption" "$T" app ui
expect "sequence" "view: sequence" "$T" app journey "j:web-src-app-ts-main" --sequence
expect "sequence ui" "seq-c:polyglot-web" "$T" app ui
expect "sequence components" "level: components" "$T" app level components --focus polyglot-web
expect "sequence components ui" "seq-c:polyglot-web/api" "$T" app ui
expect "sequence msg" "msg-1" "$T" app ui
expect "reset" "journey: null" "$T" app reset

step "edit a journey in the app"
cat > "$OUT/journey.json" <<'JSON'
{ "id": "j:mine", "name": "My own journey", "summary": "Written by hand.", "entry": "", "source": "user", "note": "",
  "messages": [
    { "from": "p:user", "to": "c:polyglot-web/app", "label": "opens the page", "caption": "The user opens the page.", "kind": "call", "depth": 0, "source": "", "by": "user" },
    { "from": "c:polyglot-web/api", "to": "c:polyglot-api/core", "label": "GET /api/users", "caption": "The page asks for users.", "kind": "flow", "depth": 1, "source": "", "by": "user" },
    { "from": "c:polyglot-api/core", "to": "c:worker", "label": "pings", "caption": "Not in the code.", "kind": "call", "depth": 1, "source": "", "by": "user" }
  ], "steps": [] }
JSON
expect "journey save" "\"j:mine\",My own journey,user,3" "$T" app journey-save --file "$OUT/journey.json"
expect "journey saved seq" "GET /api/users,code" "$T" app journey "j:mine" --sequence
expect "journey claimed" "pings,claimed" "$T" app journey "j:mine" --sequence
expect "journey delete" "deleted: \"?j:mine" "$T" app journey-delete j:mine
"$T" app reset > /dev/null

step "plan sheet: the scout proposes, the person steers (stand-in claude)"
expect "plan open" "clicked: discover-claude" "$T" app click discover-claude
expect "plan ui" "plan-propose" "$T" app ui
expect "plan propose" "clicked: plan-propose" "$T" app click plan-propose
expect_soon "plan proposed" "Nightly cleanup,\"\",A stand-in guesses .*,true" "$T" app ui
expect "plan why" "plan-flow-3-why" "$T" app ui
expect "plan rows" "Follow main to the end,web/src/app.ts#main,.*,true" "$T" app ui
expect "plan leftover" "\"\",web/src/api.ts#fetchUser,\"\",false" "$T" app ui
expect "app propose" "^proposed: 4 flows" "$T" app propose
expect "app propose plan" "open: true" "$T" app propose
expect "plan cancel" "clicked: plan-cancel" "$T" app click plan-cancel

step "discover in the app (stand-in claude)"
expect "app discover" "backed: 2[0-9]" "$T" app discover
expect "discovered" "discovered by" "$T" app ui
expect "notes" "Field notes" "$T" app ui
expect "notes scout" "stage-scout" "$T" app ui
expect "notes scout agent" "agent-scout" "$T" app ui
expect "app scouted" "\"j:nightly-cleanup\"" "$T" app atlas
expect "app discover steered" "backed: 2[0-9]" "$T" app discover --flow "web/src/app.ts#main :: watch the users list"
expect "app narrate" "source: claude" "$T" app narrate j:web-src-app-ts-main --note "say more"
expect "app reset atlas" "source: engine" "$T" app discover --reset

step "bridge checks"
expect "search" "fetchUsers" "$T" app search fetch
expect "metrics" "frontend_errors: 0" "$T" app metrics
expect "warnings" "^count: 0" "$T" app logs --level warn
expect "profile" "assemble_atlas" "$T" app profile
expect "screenshot" "^bytes: [0-9]{4,}" "$T" app screenshot --out "$OUT/app.png"
expect "eval" "result: 4" "$T" app eval 'terrarium.containers().length'
expect "atlas api" "system: Polyglot" "$T" app atlas

step "quit"
"$T" app quit >/dev/null
sleep 0.7
expect "status" "running: false" "$T" app status

printf '\nVERIFY OK — artefacts in %s\n' "$OUT"
