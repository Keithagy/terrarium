#!/usr/bin/env bash
# Hot-reloading development: Vite dev server + cargo run, via the Tauri CLI.
set -euo pipefail
cd "$(dirname "$0")/../app/ui"
npm run tauri -- dev --config src-tauri/tauri.dev.conf.json "$@"
