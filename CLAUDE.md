# Terrarium

Rust workspace (edition 2024, toolchain pinned in `rust-toolchain.toml`) + Tauri 2 app +
Vite/TypeScript UI. macOS only.

- Build the UI before the app: `cd app/ui && npm run build`. `cargo build` then embeds `app/ui/dist`.
- `scripts/verify.sh` is the end-to-end check. `AGENTS.md` documents how to drive and inspect the app.
- Keep the CLI AXI-compliant: TOON on stdout, structured errors on stdout, no prompts.
- Frontend logs must go through `log()` in `app/ui/src/tauri.ts` so they reach the bridge.
- New UI controls need a `data-testid`; the bridge's `/ui` and `/ui/click` rely on them.
- Language extraction changes need a case in `fixtures/polyglot` and `crates/terrarium-core/tests/polyglot.rs`.
- Atlas changes (engine draft, check, export) need a case in `crates/terrarium-core/tests/atlas.rs`;
  orchestration changes need one in `tests/discovery.rs` (stand-in runner, nothing spent).
- Journeys are lists of messages between atlas element ids; every level is a projection of
  that list (`atlas::project` in Rust, `project` in `app/ui/src/store.ts`). Keep the two in
  step: the map's numbers and the sequence view must agree with the boxes.
- Agent prompts and schemas live in `crates/terrarium-core/src/discovery.rs`; `scripts/stand-in-claude.py`
  parses those prompts, so keep its role detection in step when the opening words change. The roles
  open with "You are surveying", "You are scouting", "You are describing one container",
  "You are narrating" and "You are editing".
- Never run a real discovery in tests or `verify.sh`; it spends money. Use the stand-in.
