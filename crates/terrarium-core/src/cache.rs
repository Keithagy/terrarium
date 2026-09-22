//! On-disk graph cache shared by the CLI and the app.
//!
//! Layout: `<data dir>/terrarium/graphs/<sha256(abs path)[..16]>.json` plus an
//! `index.json` mapping repo paths to cache entries. On macOS the data dir is
//! `~/Library/Application Support`. Override everything with `TERRARIUM_HOME`.

use crate::model::Graph;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub fn home_dir() -> PathBuf {
    if let Ok(h) = std::env::var("TERRARIUM_HOME") {
        return PathBuf::from(h);
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("terrarium")
}

pub fn graphs_dir() -> PathBuf {
    home_dir().join("graphs")
}

pub fn logs_dir() -> PathBuf {
    home_dir().join("logs")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub root: String,
    pub file: String,
    pub scanned_at: String,
    pub files: u32,
    pub symbols: u32,
    pub flows: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Index {
    pub entries: Vec<CacheEntry>,
}

pub fn key_for(root: &Path) -> String {
    let abs = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut h = Sha256::new();
    h.update(abs.to_string_lossy().as_bytes());
    let hex = format!("{:x}", h.finalize());
    hex[..16].to_string()
}

pub fn graph_file(root: &Path) -> PathBuf {
    graphs_dir().join(format!("{}.json", key_for(root)))
}

pub fn load_index() -> Index {
    let p = graphs_dir().join("index.json");
    std::fs::read(&p)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_index(idx: &Index) -> Result<()> {
    std::fs::create_dir_all(graphs_dir())?;
    std::fs::write(
        graphs_dir().join("index.json"),
        serde_json::to_vec_pretty(idx)?,
    )?;
    Ok(())
}

pub fn store(graph: &Graph) -> Result<PathBuf> {
    let root = Path::new(&graph.root);
    let file = graph_file(root);
    std::fs::create_dir_all(graphs_dir()).context("cannot create cache dir")?;
    graph.save(&file)?;
    let mut idx = load_index();
    idx.entries.retain(|e| e.root != graph.root);
    idx.entries.push(CacheEntry {
        root: graph.root.clone(),
        file: file.to_string_lossy().to_string(),
        scanned_at: graph.scanned_at.clone(),
        files: graph.stats.files,
        symbols: graph.stats.symbols,
        flows: graph.stats.flows,
    });
    idx.entries.sort_by(|a, b| b.scanned_at.cmp(&a.scanned_at));
    save_index(&idx)?;
    Ok(file)
}

pub fn load(root: &Path) -> Option<Graph> {
    let file = graph_file(root);
    file.exists().then(|| Graph::load(&file).ok()).flatten()
}

/// Cached entry for `root` or the nearest cached ancestor of it (so running the
/// CLI from a subdirectory still finds the repo's graph).
pub fn find_entry(root: &Path) -> Option<CacheEntry> {
    let idx = load_index();
    let abs = root.canonicalize().ok()?;
    let mut cur: Option<&Path> = Some(abs.as_path());
    while let Some(p) = cur {
        let s = p.to_string_lossy();
        if let Some(e) = idx.entries.iter().find(|e| e.root == s) {
            return Some(e.clone());
        }
        cur = p.parent();
    }
    None
}

/// The saved design for `root`: `<key>.design.json` next to its graph.
pub fn design_file(root: &Path) -> PathBuf {
    graphs_dir().join(format!("{}.design.json", key_for(root)))
}

pub fn store_design(root: &Path, design: &crate::build::Design) -> Result<PathBuf> {
    let file = design_file(root);
    std::fs::create_dir_all(graphs_dir()).context("cannot create cache dir")?;
    std::fs::write(&file, serde_json::to_vec_pretty(design)?)?;
    Ok(file)
}

pub fn load_design(root: &Path) -> Option<crate::build::Design> {
    std::fs::read(design_file(root)).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

/// Forget the saved design, so the engine's is used again. False when there was none.
pub fn clear_design(root: &Path) -> Result<bool> {
    let file = design_file(root);
    if !file.exists() {
        return Ok(false);
    }
    std::fs::remove_file(file)?;
    Ok(true)
}
