//! terrarium-core: turn a multi-language repository into an architecture graph.
//!
//! ```no_run
//! let graph = terrarium_core::scan(std::path::Path::new("."), &Default::default()).unwrap();
//! println!("{} files, {} flows", graph.stats.files, graph.stats.flows);
//! ```

pub mod build;
pub mod cache;
pub mod designer;
pub mod lang;
pub mod model;
pub mod query;
pub mod scan;
pub mod tags;

pub use model::*;
pub use scan::{ScanOptions, scan};

use anyhow::{Context, Result};
use std::path::Path;

impl Graph {
    pub fn save(&self, path: &Path) -> Result<()> {
        let f = std::fs::File::create(path)
            .with_context(|| format!("cannot write {}", path.display()))?;
        serde_json::to_writer(std::io::BufWriter::new(f), self)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Graph> {
        let f =
            std::fs::File::open(path).with_context(|| format!("cannot read {}", path.display()))?;
        let g: Graph = serde_json::from_reader(std::io::BufReader::new(f))?;
        anyhow::ensure!(
            g.schema == SCHEMA_VERSION,
            "graph schema {} is not supported (expected {})",
            g.schema,
            SCHEMA_VERSION
        );
        Ok(g)
    }
}
