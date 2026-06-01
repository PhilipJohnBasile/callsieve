use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::store::{CodeIndex, FileRecord, SymbolRecord};

use super::{formatter, path_tokens};

#[derive(Debug, Clone, Serialize)]
pub struct RankedMatch {
    pub file_id: String,
    pub symbol_id: Option<String>,
    pub score: i32,
    pub why: Vec<String>,
}

pub fn rank(index: &CodeIndex, question: &str, limit: usize) -> Vec<RankedMatch> {
    let query = question.to_ascii_lowercase();
    let query_tokens = formatter::tokenize(question);
    let files_by_id: BTreeMap<&str, &FileRecord> = index
        .files
        .iter()
        .map(|file| (file.id.as_str(), file))
        .collect();

    let mut matches = Vec::new();

    for symbol in &index.symbols {
        let Some(file) = files_by_id.get(symbol.file_id.as_str()) else {
            continue;
        };
        if let Some(match_) = score_symbol(symbol, file, &query, &query_tokens) {
            matches.push(match_);
        }
    }

    for file in &index.files {
        if let Some(match_) = score_file(file, &query, &query_tokens) {
            matches.push(match_);
        }
    }

    matches.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then(left.file_id.cmp(&right.file_id))
            .then(left.symbol_id.cmp(&right.symbol_id))
    });
    matches.truncate(limit);
    matches
}

fn score_symbol(
    symbol: &SymbolRecord,
    file: &FileRecord,
    query: &str,
    query_tokens: &[String],
) -> Option<RankedMatch> {
    let mut score = 0;
    let mut why = Vec::new();
    let symbol_lower = symbol.name.to_ascii_lowercase();
    let path_lower = file.path.to_ascii_lowercase();

    if query == symbol_lower || query_tokens.iter().any(|token| token == &symbol_lower) {
        score += 100;
        why.push(format!("exact symbol match: {}", symbol.name));
    }

    let file_stem = file_stem(&file.path).to_ascii_lowercase();
    if query == path_lower || query_tokens.iter().any(|token| token == &file_stem) {
        score += 80;
        why.push(format!("path or filename match: {}", file.path));
    }

    if query.contains(&symbol_lower) || symbol_lower.contains(query) {
        if symbol.visibility == "exported" || symbol.visibility == "public" {
            score += 60;
            why.push(format!("exported symbol substring match: {}", symbol.name));
        } else {
            score += 40;
            why.push(format!("local symbol substring match: {}", symbol.name));
        }
    }

    let terms = symbol_terms(symbol, file);
    let overlap: Vec<String> = query_tokens
        .iter()
        .filter(|token| terms.contains(token.as_str()))
        .cloned()
        .collect();
    if !overlap.is_empty() {
        score += 10 * overlap.len() as i32;
        why.push(format!("keyword overlap: {}", overlap.join(", ")));
    }

    if file.is_test
        && query_tokens
            .iter()
            .any(|token| token == "test" || token == "spec")
    {
        score += 25;
        why.push("test file match".to_string());
    }

    if file.is_config && query_tokens.iter().any(|token| token == "config") {
        score += 5;
        why.push("config file heuristic".to_string());
    }

    if file.is_config && has_config_intent(query_tokens) {
        score += 45;
        why.push("config/dependency intent".to_string());
    }

    if file.size_bytes > 250_000 {
        score -= 20;
        why.push("large file penalty".to_string());
    }

    (score > 0).then(|| RankedMatch {
        file_id: file.id.clone(),
        symbol_id: Some(symbol.id.clone()),
        score,
        why,
    })
}

fn score_file(file: &FileRecord, query: &str, query_tokens: &[String]) -> Option<RankedMatch> {
    let mut score = 0;
    let mut why = Vec::new();
    let path_lower = file.path.to_ascii_lowercase();
    let file_stem = file_stem(&file.path).to_ascii_lowercase();

    if query == path_lower || query_tokens.iter().any(|token| token == &file_stem) {
        score += 80;
        why.push(format!("path or filename match: {}", file.path));
    }

    let terms: BTreeSet<String> = path_tokens(&file.path).into_iter().collect();
    let overlap: Vec<String> = query_tokens
        .iter()
        .filter(|token| terms.contains(*token))
        .cloned()
        .collect();
    if !overlap.is_empty() {
        score += 10 * overlap.len() as i32;
        why.push(format!("path keyword overlap: {}", overlap.join(", ")));
    }

    let content_terms: BTreeSet<&str> = file.content_terms.iter().map(String::as_str).collect();
    let content_overlap: Vec<String> = query_tokens
        .iter()
        .filter(|token| content_terms.contains(token.as_str()))
        .take(content_overlap_limit(file))
        .cloned()
        .collect();
    if !content_overlap.is_empty() {
        let weight = if file.is_config {
            14
        } else if file.language.is_code() {
            8
        } else {
            10
        };
        score += weight * content_overlap.len() as i32;
        why.push(format!(
            "content keyword overlap: {}",
            content_overlap.join(", ")
        ));
    }

    if file.is_test
        && query_tokens
            .iter()
            .any(|token| token == "test" || token == "spec")
    {
        score += 25;
        why.push("test file match".to_string());
    }

    if file.is_config && query_tokens.iter().any(|token| token == "config") {
        score += 5;
        why.push("config file heuristic".to_string());
    }

    if file.is_config && has_config_intent(query_tokens) {
        score += 45;
        why.push("config/dependency intent".to_string());
    }

    if is_dependency_manifest(file) && has_dependency_manifest_intent(query_tokens) {
        score += 80;
        why.push("dependency manifest intent".to_string());
    }

    if file.size_bytes > 250_000 {
        score -= 20;
        why.push("large file penalty".to_string());
    }

    (score > 0).then(|| RankedMatch {
        file_id: file.id.clone(),
        symbol_id: None,
        score,
        why,
    })
}

const CONFIG_INTENT: &[&str] = &[
    "build",
    "ci",
    "dependencies",
    "dependency",
    "package",
    "setup",
    "toolchain",
    "workflow",
];

const DEPENDENCY_MANIFEST_INTENT: &[&str] = &[
    "dependencies",
    "dependency",
    "package",
    "setup",
    "toolchain",
];

const DEPENDENCY_MANIFESTS: &[&str] = &[
    "cargo.toml",
    "package.json",
    "pyproject.toml",
    "requirements.txt",
    "rust-toolchain.toml",
];

fn has_config_intent(query_tokens: &[String]) -> bool {
    query_tokens
        .iter()
        .any(|token| CONFIG_INTENT.contains(&token.as_str()))
}

fn has_dependency_manifest_intent(query_tokens: &[String]) -> bool {
    query_tokens
        .iter()
        .any(|token| DEPENDENCY_MANIFEST_INTENT.contains(&token.as_str()))
}

fn is_dependency_manifest(file: &FileRecord) -> bool {
    let lower = file.path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(lower.as_str());
    DEPENDENCY_MANIFESTS.contains(&name)
}

fn content_overlap_limit(file: &FileRecord) -> usize {
    if file.language.is_code() {
        6
    } else if file.is_config {
        5
    } else {
        4
    }
}

fn symbol_terms(symbol: &SymbolRecord, file: &FileRecord) -> BTreeSet<String> {
    let mut terms = BTreeSet::new();
    for source in [
        symbol.name.as_str(),
        symbol.kind.as_str(),
        symbol.signature.as_str(),
        symbol.doc.as_deref().unwrap_or_default(),
        file.path.as_str(),
        file.module_path.as_str(),
    ] {
        terms.extend(formatter::tokenize(source));
    }
    terms
}

fn file_stem(path: &str) -> &str {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .split('.')
        .next()
        .unwrap_or(path)
}
