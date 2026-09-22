# Terrarium

Rust workspace (edition 2024, toolchain pinned in `rust-toolchain.toml`) + Tauri 2 app +
Vite/TypeScript UI. macOS only.

- Build the UI before the app: `cd app/ui && npm run build`. `cargo build` then embeds `app/ui/dist`.
- `scripts/verify.sh` is the end-to-end check. `AGENTS.md` documents how to drive and inspect the app.
- Keep the CLI AXI-compliant: TOON on stdout, structured errors on stdout, no prompts.
- Frontend logs must go through `log()` in `app/ui/src/tauri.ts` so they reach the bridge.
- New UI controls need a `data-testid`; the bridge's `/ui` and `/ui/click` rely on them.
- Language extraction changes need a case in `fixtures/polyglot` and `crates/terrarium-core/tests/polyglot.rs`.
