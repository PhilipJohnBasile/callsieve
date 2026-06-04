pub mod json_store;

use serde::{Deserialize, Serialize};

use crate::indexer::{language::Language, ownership::Ownership};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeIndex {
    pub schema_version: u32,
    pub root: String,
    #[serde(default)]
    pub metadata: IndexMetadata,
    pub files: Vec<FileRecord>,
    pub symbols: Vec<SymbolRecord>,
    pub imports: Vec<ImportRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<ReferenceRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexMetadata {
    #[serde(default)]
    pub indexed_at: u64,
    #[serde(default)]
    pub watch_status: String,
    #[serde(default)]
    pub watcher_mode: String,
    #[serde(default)]
    pub index_generation: u64,
    #[serde(default)]
    pub lsp_enriched: bool,
    #[serde(default)]
    pub lsp_enriched_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lsp_servers: Vec<LspServerStatus>,
}

impl Default for IndexMetadata {
    fn default() -> Self {
        Self {
            indexed_at: 0,
            watch_status: "unwatched".to_string(),
            watcher_mode: "none".to_string(),
            index_generation: 0,
            lsp_enriched: false,
            lsp_enriched_at: 0,
            last_error: None,
            lsp_servers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspServerStatus {
    pub language: String,
    pub command: String,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub id: String,
    pub path: String,
    pub language: Language,
    pub size_bytes: u64,
    pub line_count: usize,
    pub mtime: u64,
    pub content_hash: String,
    pub is_test: bool,
    pub is_config: bool,
    pub module_path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_terms: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ownership: Option<Ownership>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolRecord {
    pub id: String,
    pub file_id: String,
    pub name: String,
    pub kind: String,
    pub language: Language,
    pub start_line: usize,
    pub end_line: usize,
    pub visibility: String,
    pub parent: Option<String>,
    pub signature: String,
    pub doc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportRecord {
    pub file_id: String,
    pub source_path: String,
    pub imported: String,
    pub resolved_path: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<ImportAliasRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportAliasRecord {
    pub local: String,
    pub imported: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceRecord {
    pub file_id: String,
    pub source_path: String,
    pub source_symbol_id: Option<String>,
    pub target_name: String,
    pub target_symbol_id: Option<String>,
    pub target_path: Option<String>,
    pub kind: String,
    pub line: usize,
    #[serde(default = "default_edge_source")]
    pub edge_source: String,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lsp_method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_range: Option<[usize; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_range: Option<[usize; 2]>,
}

fn default_edge_source() -> String {
    "heuristic".to_string()
}
