pub mod json_store;

use serde::{Deserialize, Serialize};

use crate::indexer::language::Language;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeIndex {
    pub schema_version: u32,
    pub root: String,
    pub files: Vec<FileRecord>,
    pub symbols: Vec<SymbolRecord>,
    pub imports: Vec<ImportRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<ReferenceRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
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
}
