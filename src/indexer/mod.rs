pub mod imports;
pub mod language;
pub mod lsp;
pub mod references;
pub mod symbols;
pub mod tree_sitter_symbols;
pub mod walker;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};

use crate::store::{
    CodeIndex, FileRecord, ImportAliasRecord, ImportRecord, IndexMetadata, ReferenceRecord,
    SymbolRecord,
};

use self::language::Language;

pub const SCHEMA_VERSION: u32 = 6;

#[derive(Debug, Clone)]
pub struct IndexOptions {
    pub lsp: bool,
    pub watch_status: String,
    pub watcher_mode: String,
    pub index_generation: u64,
    pub last_error: Option<String>,
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            lsp: false,
            watch_status: "unwatched".to_string(),
            watcher_mode: "none".to_string(),
            index_generation: 0,
            last_error: None,
        }
    }
}

pub fn build_index(root: &Path) -> Result<CodeIndex> {
    build_index_with_options(root, IndexOptions::default())
}

pub fn build_index_with_options(root: &Path, options: IndexOptions) -> Result<CodeIndex> {
    let root = root
        .canonicalize()
        .with_context(|| format!("failed to resolve repo root {}", root.display()))?;
    let mut warnings = Vec::new();
    let mut files = Vec::new();
    let mut symbols = Vec::new();
    let mut imports = Vec::new();
    let mut file_contents = Vec::new();

    for relative_path in walker::source_files(&root)? {
        let absolute_path = root.join(&relative_path);
        let path_string = path_to_string(&relative_path);
        let language = Language::from_path(&relative_path).expect("walker only returns languages");
        let bytes = match fs::read(&absolute_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                warnings.push(format!("skipped unreadable file {path_string}: {error}"));
                continue;
            }
        };
        let content = match String::from_utf8(bytes.clone()) {
            Ok(content) => content,
            Err(_) => {
                warnings.push(format!("skipped non-UTF8 file {path_string}"));
                continue;
            }
        };
        let metadata = fs::metadata(&absolute_path)
            .with_context(|| format!("failed to stat {}", absolute_path.display()))?;
        let file_id = file_id(&path_string);
        let file = FileRecord {
            id: file_id.clone(),
            path: path_string.clone(),
            language,
            size_bytes: metadata.len(),
            line_count: content.lines().count(),
            mtime: metadata_mtime(&metadata),
            content_hash: stable_content_hash(&bytes),
            is_test: is_test_file(&path_string),
            is_config: is_config_file(&path_string),
            module_path: module_path(&relative_path),
            content_terms: content_terms(&content, language),
        };

        if language.is_code() {
            for symbol in symbols::extract_symbols(&content, language) {
                symbols.push(SymbolRecord {
                    id: symbol_id(&path_string, symbol.start_line, &symbol.name),
                    file_id: file_id.clone(),
                    name: symbol.name,
                    kind: symbol.kind,
                    language,
                    start_line: symbol.start_line,
                    end_line: symbol.end_line,
                    visibility: symbol.visibility,
                    parent: symbol.parent,
                    signature: symbol.signature,
                    doc: symbol.doc,
                });
            }

            for import in imports::extract_imports(&content, language) {
                imports.push(ImportRecord {
                    file_id: file_id.clone(),
                    source_path: path_string.clone(),
                    resolved_path: resolve_import(
                        &root,
                        &relative_path,
                        language,
                        &import.imported,
                    ),
                    imported: import.imported,
                    aliases: import
                        .aliases
                        .into_iter()
                        .map(|alias| ImportAliasRecord {
                            local: alias.local,
                            imported: alias.imported,
                        })
                        .collect(),
                });
            }
        }

        files.push(file);
        file_contents.push((file_id, path_string, language, content));
    }

    files.sort_by(|left, right| left.path.cmp(&right.path));
    symbols.sort_by(|left, right| {
        left.file_id
            .cmp(&right.file_id)
            .then(left.start_line.cmp(&right.start_line))
            .then(left.name.cmp(&right.name))
    });
    imports.sort_by(|left, right| {
        left.source_path
            .cmp(&right.source_path)
            .then(left.imported.cmp(&right.imported))
    });
    let mut references = build_references(&file_contents, &files, &symbols, &imports);
    let mut lsp_enriched = false;
    if options.lsp {
        let lsp_references = lsp::enrich_references(&root, &file_contents, &files, &symbols);
        lsp_enriched = !lsp_references.is_empty();
        references = merge_references(references, lsp_references);
    }
    references.sort_by(|left, right| {
        left.source_path
            .cmp(&right.source_path)
            .then(left.line.cmp(&right.line))
            .then(left.target_name.cmp(&right.target_name))
            .then(left.kind.cmp(&right.kind))
    });

    let lsp_servers = lsp::server_statuses(&files);
    let indexed_at = now_unix_seconds();

    Ok(CodeIndex {
        schema_version: SCHEMA_VERSION,
        root: ".".to_string(),
        files,
        symbols,
        imports,
        references,
        metadata: IndexMetadata {
            indexed_at,
            watch_status: options.watch_status,
            watcher_mode: options.watcher_mode,
            index_generation: options.index_generation,
            lsp_enriched,
            lsp_enriched_at: if lsp_enriched { indexed_at } else { 0 },
            last_error: options.last_error,
            lsp_servers,
        },
        warnings,
    })
}

pub fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn file_id(path: &str) -> String {
    format!("file:{path}")
}

fn symbol_id(path: &str, line: usize, name: &str) -> String {
    format!("symbol:{path}:{line}:{name}")
}

fn metadata_mtime(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn stable_content_hash(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}")
}

fn module_path(path: &Path) -> String {
    path.parent().map(path_to_string).unwrap_or_default()
}

fn is_test_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("/__tests__/")
        || lower.contains("/tests/")
        || lower.contains("_test.")
        || lower.contains(".test.")
        || lower.contains(".spec.")
        || lower.ends_with("_test.rs")
}

fn is_config_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let file = lower.rsplit('/').next().unwrap_or(lower.as_str());
    lower.starts_with(".github/workflows/")
        || file.contains("config")
        || matches!(
            file,
            "build.rs"
                | "cargo.toml"
                | "package.json"
                | "pyproject.toml"
                | "requirements.txt"
                | "rust-toolchain.toml"
                | "rustfmt.toml"
                | "clippy.toml"
        )
}

fn content_terms(content: &str, language: Language) -> Vec<String> {
    if language.is_code() {
        return Vec::new();
    }

    let mut counts = BTreeMap::new();
    for token in tokenize_content(content) {
        *counts.entry(token).or_insert(0usize) += 1;
    }

    let mut ranked_terms: Vec<(String, usize)> = counts.into_iter().collect();
    ranked_terms.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
    ranked_terms
        .into_iter()
        .take(120)
        .map(|(term, _)| term)
        .collect()
}

fn tokenize_content(input: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "how", "in", "is", "it",
        "of", "on", "or", "that", "the", "this", "to", "with",
    ];

    split_camel_case(input)
        .split(|character: char| !character.is_ascii_alphanumeric())
        .map(|token| token.trim().to_ascii_lowercase())
        .filter(|token| token.len() > 1)
        .filter(|token| !STOP_WORDS.contains(&token.as_str()))
        .collect()
}

fn split_camel_case(input: &str) -> String {
    let mut output = String::with_capacity(input.len() + 8);
    let mut previous_lowercase = false;

    for character in input.chars() {
        if previous_lowercase && character.is_ascii_uppercase() {
            output.push(' ');
        }
        previous_lowercase = character.is_ascii_lowercase() || character.is_ascii_digit();
        output.push(character);
    }

    output
}

fn resolve_import(
    root: &Path,
    source: &Path,
    language: Language,
    imported: &str,
) -> Option<String> {
    match language {
        Language::TypeScript | Language::JavaScript => {
            if !imported.starts_with('.') {
                return None;
            }
            let base = source.parent().unwrap_or_else(|| Path::new(""));
            let candidate = normalize_relative(base.join(imported));
            let extensions: &[&str] = if language == Language::TypeScript {
                &["ts", "tsx", "js", "jsx"]
            } else {
                &["js", "jsx", "ts", "tsx"]
            };
            resolve_candidate(root, &candidate, extensions).map(|path| path_to_string(&path))
        }
        Language::Python => resolve_python_import(root, source, imported),
        Language::Rust => resolve_rust_import(root, source, imported),
        Language::Markdown | Language::Json | Language::Toml | Language::Yaml | Language::Text => {
            None
        }
    }
}

fn resolve_candidate(root: &Path, candidate: &Path, extensions: &[&str]) -> Option<PathBuf> {
    if root.join(candidate).is_file() {
        return Some(candidate.to_path_buf());
    }

    for extension in extensions {
        let with_extension = candidate.with_extension(extension);
        if root.join(&with_extension).is_file() {
            return Some(with_extension);
        }
    }

    for extension in extensions {
        let index_file = candidate.join(format!("index.{extension}"));
        if root.join(&index_file).is_file() {
            return Some(index_file);
        }
    }

    if extensions.contains(&"py") {
        let init_file = candidate.join("__init__.py");
        if root.join(&init_file).is_file() {
            return Some(init_file);
        }
    }

    None
}

fn normalize_relative(path: PathBuf) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn resolve_python_import(root: &Path, source: &Path, imported: &str) -> Option<String> {
    let raw_imported = imported.trim();
    let imported = raw_imported.trim_start_matches('.');
    if imported.is_empty() {
        return None;
    }

    let module_path = PathBuf::from(imported.replace('.', "/"));
    let mut candidates = vec![module_path.clone(), PathBuf::from("src").join(&module_path)];

    if let Some(top_level) = source.components().next() {
        candidates.push(PathBuf::from(top_level.as_os_str()).join(&module_path));
    }

    if raw_imported.starts_with('.') {
        let base = source.parent().unwrap_or_else(|| Path::new(""));
        candidates.push(normalize_relative(base.join(&module_path)));
    }

    candidates.into_iter().find_map(|candidate| {
        resolve_candidate(root, &candidate, &["py"]).map(|path| path_to_string(&path))
    })
}

fn resolve_rust_import(root: &Path, source: &Path, imported: &str) -> Option<String> {
    let imported = imported
        .trim()
        .trim_end_matches(';')
        .split_once(" as ")
        .map(|(path, _)| path)
        .unwrap_or(imported)
        .trim();
    let imported = imported
        .split_once('{')
        .map(|(prefix, _)| prefix.trim_end_matches("::"))
        .unwrap_or(imported);
    let parts: Vec<&str> = imported
        .split("::")
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }

    let base = rust_import_base(source, parts[0]);
    let mut module_parts = match parts[0] {
        "crate" | "self" | "super" => parts[1..].to_vec(),
        _ => parts.clone(),
    };
    if module_parts.is_empty() {
        return None;
    }

    let mut candidates = Vec::new();
    candidates.push(base.join(module_parts.join("/")));
    module_parts.pop();
    if !module_parts.is_empty() {
        candidates.push(base.join(module_parts.join("/")));
    }

    candidates.into_iter().find_map(|candidate| {
        resolve_candidate(root, &candidate, &["rs"]).map(|path| path_to_string(&path))
    })
}

fn rust_import_base(source: &Path, prefix: &str) -> PathBuf {
    match prefix {
        "self" => source
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_path_buf(),
        "super" => source
            .parent()
            .and_then(Path::parent)
            .unwrap_or_else(|| Path::new(""))
            .to_path_buf(),
        _ => {
            let mut base = PathBuf::new();
            for component in source.components() {
                base.push(component.as_os_str());
                if component.as_os_str().to_string_lossy() == "src" {
                    return base;
                }
            }
            source
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .to_path_buf()
        }
    }
}

fn build_references(
    file_contents: &[(String, String, Language, String)],
    files: &[FileRecord],
    symbols: &[SymbolRecord],
    imports: &[ImportRecord],
) -> Vec<ReferenceRecord> {
    let candidate_names: std::collections::BTreeSet<String> = symbols
        .iter()
        .map(|symbol| symbol.name.clone())
        .chain(
            imports
                .iter()
                .flat_map(|import| import.aliases.iter().map(|alias| alias.local.clone())),
        )
        .collect();
    let mut references = Vec::new();

    for (file_id, path, language, content) in file_contents {
        for raw in references::extract_references(content, *language, &candidate_names) {
            let target =
                resolve_reference_target(file_id, path, &raw.target_name, files, symbols, imports);
            references.push(ReferenceRecord {
                file_id: file_id.clone(),
                source_path: path.clone(),
                source_symbol_id: source_symbol_for_line(file_id, raw.line, symbols)
                    .map(|symbol| symbol.id.clone()),
                target_name: raw.target_name,
                target_symbol_id: target.map(|symbol| symbol.id.clone()),
                target_path: target.and_then(|symbol| {
                    files
                        .iter()
                        .find(|file| file.id == symbol.file_id)
                        .map(|file| file.path.clone())
                }),
                kind: raw.kind,
                line: raw.line,
                edge_source: raw.edge_source,
                confidence: raw.confidence,
                lsp_method: None,
                source_range: Some([raw.line, raw.line]),
                target_range: target.map(|symbol| [symbol.start_line, symbol.end_line]),
            });
        }
    }

    references
}

fn source_symbol_for_line<'a>(
    file_id: &str,
    line: usize,
    symbols: &'a [SymbolRecord],
) -> Option<&'a SymbolRecord> {
    symbols
        .iter()
        .filter(|symbol| {
            symbol.file_id == file_id && symbol.start_line <= line && symbol.end_line >= line
        })
        .min_by_key(|symbol| symbol.end_line.saturating_sub(symbol.start_line))
}

fn resolve_reference_target<'a>(
    file_id: &str,
    source_path: &str,
    target_name: &str,
    files: &[FileRecord],
    symbols: &'a [SymbolRecord],
    imports: &[ImportRecord],
) -> Option<&'a SymbolRecord> {
    let same_file: Vec<&SymbolRecord> = symbols
        .iter()
        .filter(|symbol| symbol.file_id == file_id && symbol.name == target_name)
        .collect();
    if same_file.len() == 1 {
        return same_file.first().copied();
    }

    let source_imports: Vec<&ImportRecord> = imports
        .iter()
        .filter(|import| import.source_path == source_path)
        .collect();
    for import in &source_imports {
        for alias in &import.aliases {
            if alias.local == target_name
                && let Some(imported_path) = import.resolved_path.as_deref()
            {
                let imported_name = import_alias_target_name(&alias.imported, target_name);
                let imported_symbols: Vec<&SymbolRecord> = symbols
                    .iter()
                    .filter(|symbol| {
                        symbol.name == imported_name
                            && files
                                .iter()
                                .find(|file| file.id == symbol.file_id)
                                .is_some_and(|file| file.path == imported_path)
                    })
                    .collect();
                if imported_symbols.len() == 1 {
                    return imported_symbols.first().copied();
                }
            }
        }
    }

    let imported_paths: std::collections::BTreeSet<&str> = imports
        .iter()
        .filter(|import| import.source_path == source_path)
        .filter_map(|import| import.resolved_path.as_deref())
        .collect();
    let imported_symbols: Vec<&SymbolRecord> = symbols
        .iter()
        .filter(|symbol| {
            symbol.name == target_name
                && files
                    .iter()
                    .find(|file| file.id == symbol.file_id)
                    .is_some_and(|file| imported_paths.contains(file.path.as_str()))
        })
        .collect();
    if imported_symbols.len() == 1 {
        return imported_symbols.first().copied();
    }

    let global_symbols: Vec<&SymbolRecord> = symbols
        .iter()
        .filter(|symbol| symbol.name == target_name)
        .collect();
    (global_symbols.len() == 1).then(|| global_symbols[0])
}

fn import_alias_target_name<'a>(imported: &'a str, fallback: &'a str) -> &'a str {
    if imported == "*" {
        return fallback;
    }
    imported
        .rsplit("::")
        .next()
        .unwrap_or(imported)
        .rsplit('.')
        .next()
        .unwrap_or(imported)
}

fn merge_references(
    existing: Vec<ReferenceRecord>,
    lsp_references: Vec<ReferenceRecord>,
) -> Vec<ReferenceRecord> {
    let mut merged: BTreeMap<ReferenceKey, ReferenceRecord> = BTreeMap::new();
    for reference in existing.into_iter().chain(lsp_references) {
        let key = ReferenceKey::from(&reference);
        match merged.get(&key) {
            Some(current)
                if current.confidence > reference.confidence
                    || (current.confidence == reference.confidence
                        && is_lsp_edge_source(&current.edge_source)) => {}
            _ => {
                merged.insert(key, reference);
            }
        }
    }
    merged.into_values().collect()
}

fn is_lsp_edge_source(edge_source: &str) -> bool {
    edge_source == "lsp" || edge_source.starts_with("lsp_")
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
struct ReferenceKey {
    source_path: String,
    target_name: String,
    target_path: Option<String>,
    kind: String,
    line: usize,
}

impl From<&ReferenceRecord> for ReferenceKey {
    fn from(reference: &ReferenceRecord) -> Self {
        Self {
            source_path: reference.source_path.clone(),
            target_name: reference.target_name.clone(),
            target_path: reference.target_path.clone(),
            kind: reference.kind.clone(),
            line: reference.line,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn builds_basic_index() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::write(
            temp.path().join("src/session.ts"),
            "import { token } from './token';\nexport function createSession() {\n  return token;\n}\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("src/token.ts"),
            "export const token = 'x';\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("README.md"),
            "# Session docs\n\nThe createSession token flow lives in src/session.ts.\n",
        )
        .unwrap();

        let index = build_index(temp.path()).unwrap();

        assert_eq!(index.files.len(), 3);
        assert!(
            index
                .symbols
                .iter()
                .any(|symbol| symbol.name == "createSession")
        );
        assert!(
            index
                .imports
                .iter()
                .any(|import| import.resolved_path.as_deref() == Some("src/token.ts"))
        );
        assert!(index.references.iter().any(|reference| {
            reference.target_name == "token"
                && reference.target_path.as_deref() == Some("src/token.ts")
        }));
        let readme = index
            .files
            .iter()
            .find(|file| file.path == "README.md")
            .unwrap();
        assert_eq!(readme.language, Language::Markdown);
        assert!(readme.content_terms.contains(&"session".to_string()));
    }

    #[test]
    fn resolves_alias_import_references() {
        let temp = tempfile::tempdir().unwrap();

        fs::create_dir_all(temp.path().join("ts")).unwrap();
        fs::write(
            temp.path().join("ts/token.ts"),
            "export function tokenFor() { return 'token'; }\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("ts/session.ts"),
            "import { tokenFor as makeToken } from './token';\nexport function createSession() {\n  return makeToken();\n}\n",
        )
        .unwrap();

        fs::create_dir_all(temp.path().join("pkg")).unwrap();
        fs::write(
            temp.path().join("pkg/token.py"),
            "def token_for():\n    return 'token'\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("pkg/session.py"),
            "from pkg.token import token_for as make_token\n\ndef create_session():\n    return make_token()\n",
        )
        .unwrap();

        fs::create_dir_all(temp.path().join("rust/src")).unwrap();
        fs::write(
            temp.path().join("rust/src/auth.rs"),
            "pub fn token_for() -> &'static str { \"token\" }\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("rust/src/lib.rs"),
            "mod auth;\nuse crate::auth::token_for as make_token;\npub fn create_session() -> &'static str {\n    make_token()\n}\n",
        )
        .unwrap();

        let index = build_index(temp.path()).unwrap();

        assert!(index.references.iter().any(|reference| {
            reference.target_name == "makeToken"
                && reference.target_path.as_deref() == Some("ts/token.ts")
                && reference.target_symbol_id.is_some()
        }));
        assert!(index.references.iter().any(|reference| {
            reference.target_name == "make_token"
                && reference.target_path.as_deref() == Some("pkg/token.py")
                && reference.target_symbol_id.is_some()
        }));
        assert!(index.references.iter().any(|reference| {
            reference.source_path == "rust/src/lib.rs"
                && reference.target_name == "make_token"
                && reference.target_path.as_deref() == Some("rust/src/auth.rs")
                && reference.target_symbol_id.is_some()
        }));
    }

    #[test]
    fn merge_references_prefers_lsp_edges() {
        let heuristic = ReferenceRecord {
            file_id: "file:src/session.ts".to_string(),
            source_path: "src/session.ts".to_string(),
            source_symbol_id: None,
            target_name: "tokenFor".to_string(),
            target_symbol_id: Some("symbol:src/token.ts:1:tokenFor".to_string()),
            target_path: Some("src/token.ts".to_string()),
            kind: "call".to_string(),
            line: 4,
            edge_source: "heuristic".to_string(),
            confidence: 0.45,
            lsp_method: None,
            source_range: None,
            target_range: None,
        };
        let lsp = ReferenceRecord {
            edge_source: "lsp".to_string(),
            confidence: 1.0,
            ..heuristic.clone()
        };

        let references = merge_references(vec![heuristic], vec![lsp]);

        assert_eq!(references.len(), 1);
        assert_eq!(references[0].edge_source, "lsp");
        assert_eq!(references[0].confidence, 1.0);
    }

    #[test]
    fn lsp_flag_only_marks_enriched_when_lsp_edges_exist() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "# docs only\n").unwrap();

        let index = build_index_with_options(
            temp.path(),
            IndexOptions {
                lsp: true,
                ..IndexOptions::default()
            },
        )
        .unwrap();

        assert!(!index.metadata.lsp_enriched);
        assert!(
            index
                .references
                .iter()
                .all(|edge| edge.edge_source != "lsp")
        );
    }
}
