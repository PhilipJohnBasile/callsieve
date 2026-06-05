use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::{
    indexer::SCHEMA_VERSION,
    store::{CodeIndex, FileRecord, SymbolRecord},
};

use super::embed::{EmbedCache, LocalEmbedder, index_fingerprint, write_embeds};

const MAX_CONTENT_TERMS: usize = 256;
const MAX_DOCUMENT_CHARS: usize = 4096;

pub fn compose_file_document(file: &FileRecord, symbols: &[&SymbolRecord]) -> String {
    let mut doc = String::new();
    push_line(&mut doc, &file.path);
    push_line(&mut doc, &file.module_path);

    let mut sorted = symbols.to_vec();
    sorted.sort_by(|left, right| {
        left.start_line
            .cmp(&right.start_line)
            .then(left.name.cmp(&right.name))
            .then(left.id.cmp(&right.id))
    });
    for symbol in sorted {
        let first_doc_line = symbol
            .doc
            .as_deref()
            .and_then(|doc| doc.lines().map(str::trim).find(|line| !line.is_empty()))
            .unwrap_or("");
        if first_doc_line.is_empty() {
            push_line(&mut doc, &format!("{} {}", symbol.name, symbol.signature));
        } else {
            push_line(
                &mut doc,
                &format!("{} {} {}", symbol.name, symbol.signature, first_doc_line),
            );
        }
    }

    if !file.content_terms.is_empty() {
        push_line(
            &mut doc,
            &file
                .content_terms
                .iter()
                .take(MAX_CONTENT_TERMS)
                .cloned()
                .collect::<Vec<_>>()
                .join(" "),
        );
    }

    truncate_on_char_boundary(&mut doc, MAX_DOCUMENT_CHARS);
    doc
}

pub fn build_and_write_embeds(
    root: &Path,
    index: &CodeIndex,
    embedder: &dyn LocalEmbedder,
    quantize_f16: bool,
) -> Result<PathBuf> {
    let mut symbols_by_file: std::collections::BTreeMap<&str, Vec<&SymbolRecord>> =
        std::collections::BTreeMap::new();
    for symbol in &index.symbols {
        symbols_by_file
            .entry(symbol.file_id.as_str())
            .or_default()
            .push(symbol);
    }

    let documents: Vec<String> = index
        .files
        .iter()
        .map(|file| {
            compose_file_document(
                file,
                symbols_by_file
                    .get(file.id.as_str())
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
            )
        })
        .collect();
    let document_refs: Vec<&str> = documents.iter().map(String::as_str).collect();
    let vectors = embedder.embed(&document_refs)?;
    if vectors.len() != index.files.len() {
        bail!(
            "embedder returned {} vectors for {} indexed files",
            vectors.len(),
            index.files.len()
        );
    }
    let dim = vectors.first().map(Vec::len).unwrap_or(0);
    if dim == 0 && !vectors.is_empty() {
        bail!("embedder returned zero-dimensional vectors");
    }
    for (i, vector) in vectors.iter().enumerate() {
        if vector.len() != dim {
            bail!(
                "embedder returned vector {i} with dim {}, expected {dim}",
                vector.len()
            );
        }
    }

    let cache = EmbedCache {
        embedder: embedder.id(),
        index_schema_version: SCHEMA_VERSION,
        fingerprint: index_fingerprint(index),
        dim,
        vectors,
    };
    write_embeds(root, &cache, quantize_f16)
}

fn push_line(doc: &mut String, line: &str) {
    if doc.len() >= MAX_DOCUMENT_CHARS {
        return;
    }
    doc.push_str(line.trim());
    doc.push('\n');
}

fn truncate_on_char_boundary(text: &mut String, max_chars: usize) {
    if text.chars().count() <= max_chars {
        return;
    }
    let byte_index = text
        .char_indices()
        .nth(max_chars)
        .map(|(index, _)| index)
        .unwrap_or_else(|| text.len());
    text.truncate(byte_index);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        indexer,
        query::embed::{ExpectedCache, LocalEmbedder, read_embeds},
    };
    use std::fs;

    struct FakeEmbedder;

    impl LocalEmbedder for FakeEmbedder {
        fn id(&self) -> super::super::embed::EmbedderId {
            super::super::embed::EmbedderId::new("fake", "v1")
        }

        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
            Ok(texts
                .iter()
                .map(|text| vec![text.len() as f32, text.bytes().next().unwrap_or(0) as f32])
                .collect())
        }
    }

    #[test]
    fn compose_file_document_is_deterministic() {
        let file = FileRecord {
            id: "file:src/lib.rs".to_string(),
            path: "src/lib.rs".to_string(),
            language: crate::indexer::language::Language::Rust,
            size_bytes: 10,
            line_count: 1,
            mtime: 0,
            content_hash: "fnv1a64:1".to_string(),
            is_test: false,
            is_config: false,
            module_path: "src".to_string(),
            content_terms: vec!["beta".to_string(), "alpha".to_string()],
            ownership: None,
        };
        let late = SymbolRecord {
            id: "b".to_string(),
            file_id: file.id.clone(),
            name: "late".to_string(),
            kind: "function".to_string(),
            language: file.language,
            start_line: 20,
            end_line: 21,
            visibility: "public".to_string(),
            parent: None,
            signature: "fn late()".to_string(),
            doc: Some("Late docs\nsecond line".to_string()),
        };
        let early = SymbolRecord {
            id: "a".to_string(),
            file_id: file.id.clone(),
            name: "early".to_string(),
            kind: "function".to_string(),
            language: file.language,
            start_line: 1,
            end_line: 2,
            visibility: "public".to_string(),
            parent: None,
            signature: "fn early()".to_string(),
            doc: None,
        };

        let first = compose_file_document(&file, &[&late, &early]);
        let second = compose_file_document(&file, &[&early, &late]);
        assert_eq!(first, second);
        assert!(first.starts_with("src/lib.rs\nsrc\nearly fn early()\nlate fn late() Late docs\n"));
    }

    #[test]
    fn build_and_write_embeds_keeps_index_order() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("a.rs"), "pub fn alpha() {}\n").unwrap();
        fs::write(temp.path().join("b.rs"), "pub fn beta() {}\n").unwrap();
        let index = indexer::build_index(temp.path()).unwrap();
        let embedder = FakeEmbedder;
        let path = build_and_write_embeds(temp.path(), &index, &embedder, false).unwrap();
        assert!(path.exists());

        let embedder_id = embedder.id();
        let fingerprint = super::super::embed::index_fingerprint(&index);
        let expected = ExpectedCache {
            embedder: &embedder_id,
            index_schema_version: SCHEMA_VERSION,
            fingerprint: &fingerprint,
            expected_count: index.files.len(),
        };
        let cache = read_embeds(temp.path(), &expected)
            .unwrap()
            .expect("cache should read");
        let docs: Vec<String> = index
            .files
            .iter()
            .map(|file| {
                let symbols: Vec<&SymbolRecord> = index
                    .symbols
                    .iter()
                    .filter(|symbol| symbol.file_id == file.id)
                    .collect();
                compose_file_document(file, &symbols)
            })
            .collect();
        let doc_refs: Vec<&str> = docs.iter().map(String::as_str).collect();
        assert_eq!(cache.vectors, embedder.embed(&doc_refs).unwrap());
    }
}
