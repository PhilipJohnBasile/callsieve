use std::{fs, path::Path};

use anyhow::{Context, Result};

use crate::store::CodeIndex;

pub const INDEX_DIR: &str = ".callsieve";
pub const INDEX_FILE: &str = "index.json";

pub fn index_path(root: &Path) -> std::path::PathBuf {
    root.join(INDEX_DIR).join(INDEX_FILE)
}

pub fn save_index(root: &Path, index: &CodeIndex) -> Result<std::path::PathBuf> {
    let root = root
        .canonicalize()
        .with_context(|| format!("failed to resolve repo root {}", root.display()))?;
    let dir = root.join(INDEX_DIR);
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create index directory {}", dir.display()))?;

    let path = dir.join(INDEX_FILE);
    let data = serde_json::to_vec_pretty(index)?;
    fs::write(&path, data).with_context(|| format!("failed to write index {}", path.display()))?;
    Ok(path)
}

pub fn load_index(root: &Path) -> Result<CodeIndex> {
    let path = index_path(root);
    let data = fs::read(&path).with_context(|| {
        format!(
            "missing CallSieve index at {}; run `callsieve index {}` first",
            path.display(),
            root.display()
        )
    })?;
    serde_json::from_slice(&data).with_context(|| format!("failed to parse {}", path.display()))
}
