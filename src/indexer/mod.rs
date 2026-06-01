pub mod imports;
pub mod language;
pub mod references;
pub mod symbols;
pub mod tree_sitter_symbols;
pub mod walker;

use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use anyhow::{Context, Result};

use crate::store::{CodeIndex, FileRecord, ImportRecord, ReferenceRecord, SymbolRecord};

use self::language::Language;

pub const SCHEMA_VERSION: u32 = 2;

pub fn build_index(root: &Path) -> Result<CodeIndex> {
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
        };

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
                resolved_path: resolve_import(&root, &relative_path, language, &import.imported),
                imported: import.imported,
            });
        }

        files.push(file);
        file_contents.push((file_id, path_string, content));
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
    references.sort_by(|left, right| {
        left.source_path
            .cmp(&right.source_path)
            .then(left.line.cmp(&right.line))
            .then(left.target_name.cmp(&right.target_name))
            .then(left.kind.cmp(&right.kind))
    });

    Ok(CodeIndex {
        schema_version: SCHEMA_VERSION,
        root: ".".to_string(),
        files,
        symbols,
        imports,
        references,
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
    let file = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    file.contains("config") || file == "build.rs"
}

fn resolve_import(
    root: &Path,
    source: &Path,
    language: Language,
    imported: &str,
) -> Option<String> {
    if !imported.starts_with('.') {
        return None;
    }

    let base = source.parent().unwrap_or_else(|| Path::new(""));
    let candidate = normalize_relative(base.join(imported));
    let extensions: &[&str] = match language {
        Language::TypeScript => &["ts", "tsx", "js", "jsx"],
        Language::JavaScript => &["js", "jsx", "ts", "tsx"],
        Language::Python => &["py"],
        Language::Rust => &["rs"],
    };

    resolve_candidate(root, &candidate, extensions).map(|path| path_to_string(&path))
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

fn build_references(
    file_contents: &[(String, String, String)],
    files: &[FileRecord],
    symbols: &[SymbolRecord],
    imports: &[ImportRecord],
) -> Vec<ReferenceRecord> {
    let candidate_names: std::collections::BTreeSet<String> =
        symbols.iter().map(|symbol| symbol.name.clone()).collect();
    let mut references = Vec::new();

    for (file_id, path, content) in file_contents {
        for raw in references::extract_references(content, &candidate_names) {
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

        let index = build_index(temp.path()).unwrap();

        assert_eq!(index.files.len(), 2);
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
    }
}
