//! Session-start hooks so agents see the repo's graph state without asking.

use anyhow::{Context, Result};
use clap::ValueEnum;
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Target {
    Claude,
    Codex,
    Opencode,
}

/// The command hooks should run: a PATH-resolvable name when it is this binary, else the absolute path.
fn hook_command() -> String {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("terrarium"));
    if let Ok(found) = which("terrarium")
        && found.canonicalize().ok() == exe.canonicalize().ok()
    {
        return "terrarium".into();
    }
    exe.to_string_lossy().to_string()
}

fn which(name: &str) -> Result<PathBuf> {
    let path = std::env::var("PATH").unwrap_or_default();
    for dir in path.split(':') {
        let p = PathBuf::from(dir).join(name);
        if p.is_file() {
            return Ok(p);
        }
    }
    anyhow::bail!("not on PATH")
}

fn read_json(p: &PathBuf) -> Value {
    std::fs::read(p)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| json!({}))
}

fn write_json(p: &PathBuf, v: &Value) -> Result<()> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(p, serde_json::to_vec_pretty(v)?)
        .with_context(|| format!("cannot write {}", p.display()))
}

pub fn install(target: Target, global: bool) -> Result<Value> {
    let cmd = hook_command();
    let home = dirs::home_dir().context("no home dir")?;
    let cwd = std::env::current_dir()?;
    match target {
        Target::Claude => {
            let file = if global {
                home.join(".claude/settings.json")
            } else {
                cwd.join(".claude/settings.json")
            };
            let mut v = read_json(&file);
            let hooks = v
                .as_object_mut()
                .unwrap()
                .entry("hooks")
                .or_insert(json!({}));
            let list = hooks
                .as_object_mut()
                .unwrap()
                .entry("SessionStart")
                .or_insert(json!([]));
            let arr = list.as_array_mut().unwrap();
            let marker = "terrarium";
            let status = upsert_hook(
                arr,
                marker,
                json!({ "hooks": [{ "type": "command", "command": cmd }] }),
                |h| {
                    h["hooks"][0]["command"]
                        .as_str()
                        .map(|c| c.contains(marker))
                        .unwrap_or(false)
                },
                |h| h["hooks"][0]["command"] == json!(cmd),
            );
            write_json(&file, &v)?;
            Ok(json!({ "installed": file.to_string_lossy(), "status": status, "command": cmd }))
        }
        Target::Codex => {
            let file = if global {
                home.join(".codex/hooks.json")
            } else {
                cwd.join(".codex/hooks.json")
            };
            let mut v = read_json(&file);
            let hooks = v
                .as_object_mut()
                .unwrap()
                .entry("hooks")
                .or_insert(json!({}));
            let list = hooks
                .as_object_mut()
                .unwrap()
                .entry("SessionStart")
                .or_insert(json!([]));
            let arr = list.as_array_mut().unwrap();
            let status = upsert_hook(
                arr,
                "terrarium",
                json!({ "hooks": [{ "type": "command", "command": cmd }] }),
                |h| {
                    h["hooks"][0]["command"]
                        .as_str()
                        .map(|c| c.contains("terrarium"))
                        .unwrap_or(false)
                },
                |h| h["hooks"][0]["command"] == json!(cmd),
            );
            write_json(&file, &v)?;
            Ok(
                json!({ "installed": file.to_string_lossy(), "status": status, "command": cmd, "note": "ensure `[features].hooks = true` in ~/.codex/config.toml" }),
            )
        }
        Target::Opencode => {
            let file = home.join(".config/opencode/plugins/terrarium.js");
            let src = format!(
                "// Installed by `terrarium setup opencode`.\nexport const TerrariumPlugin = async ({{ $ }}) => ({{\n  'experimental.chat.system.transform': async (_input, output) => {{\n    try {{\n      const out = await $`{cmd}`.text();\n      output.system.push('Terrarium repo state:\\n' + out);\n    }} catch (_) {{}}\n  }},\n}});\n"
            );
            let status = if std::fs::read_to_string(&file)
                .map(|s| s == src)
                .unwrap_or(false)
            {
                "unchanged"
            } else {
                "written"
            };
            if let Some(d) = file.parent() {
                std::fs::create_dir_all(d)?;
            }
            std::fs::write(&file, src)?;
            Ok(json!({ "installed": file.to_string_lossy(), "status": status, "command": cmd }))
        }
    }
}

fn upsert_hook(
    arr: &mut Vec<Value>,
    _marker: &str,
    entry: Value,
    is_ours: impl Fn(&Value) -> bool,
    is_current: impl Fn(&Value) -> bool,
) -> &'static str {
    if let Some(pos) = arr.iter().position(is_ours) {
        if is_current(&arr[pos]) {
            return "unchanged";
        }
        arr[pos] = entry;
        return "updated path";
    }
    arr.push(entry);
    "installed"
}
