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
    let query_tokens = expand_query_tokens(formatter::tokenize(question));
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

    if query == symbol_lower
        || query.contains(&symbol_lower)
        || query_tokens.iter().any(|token| token == &symbol_lower)
    {
        let symbol_token_count = formatter::tokenize(&symbol.name).len();
        score += if symbol_token_count > 1 { 320 } else { 180 };
        why.push(format!("exact symbol match: {}", symbol.name));
    }

    let file_stem = file_stem(&file.path).to_ascii_lowercase();
    if query == path_lower || query_tokens.iter().any(|token| token == &file_stem) {
        score += if file.language.is_code() { 80 } else { 230 };
        why.push(format!("path or filename match: {}", file.path));
    }

    if symbol_lower.contains(query) {
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
        score += 14 * overlap.len() as i32;
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
        score += if file.language.is_code() {
            if file.is_test { 140 } else { 300 }
        } else {
            230
        };
        why.push(format!("path or filename match: {}", file.path));
    }

    let terms: BTreeSet<String> = path_tokens(&file.path).into_iter().collect();
    let overlap: Vec<String> = query_tokens
        .iter()
        .filter(|token| terms.contains(*token))
        .cloned()
        .collect();
    if !overlap.is_empty() {
        score += 16 * overlap.len() as i32;
        why.push(format!("path keyword overlap: {}", overlap.join(", ")));
    }

    if is_module_anchor_match(file, query_tokens) {
        score += 180;
        why.push("module anchor path match".to_string());
    }

    if let Some(score_boost) = basename_cluster_score(file, query_tokens) {
        score += score_boost;
        why.push("filename keyword cluster".to_string());
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

    if file.is_test && (!overlap.is_empty() || !content_overlap.is_empty()) {
        let signal_count = overlap.len().saturating_add(content_overlap.len()).min(4) as i32;
        score += 40 + (signal_count * 15);
        why.push("test proximity match".to_string());
    }

    if is_fixture_data(file) && !has_test_intent(query_tokens) {
        score -= 140;
        why.push("fixture data penalty".to_string());
    }

    if file.is_config && query_tokens.iter().any(|token| token == "config") {
        score += 5;
        why.push("config file heuristic".to_string());
    }

    if file.is_config && has_config_intent(query_tokens) {
        score += 70;
        why.push("config/dependency intent".to_string());
    }

    if is_dependency_manifest(file) && has_dependency_manifest_intent(query_tokens) {
        score += 170;
        why.push("dependency manifest intent".to_string());
    }

    if is_benchmark_file(file) && has_benchmark_evidence_intent(query_tokens) {
        score += 260;
        why.push("benchmark evidence file intent".to_string());
    }

    if is_readme(file) && has_benchmark_evidence_intent(query_tokens) {
        score += 240;
        why.push("readme evidence file intent".to_string());
    }

    if is_docs_file(file) && has_docs_intent(query_tokens) {
        score += 260;
        why.push("docs intent".to_string());
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

const BENCHMARK_EVIDENCE_INTENT: &[&str] = &[
    "benchmark",
    "benchmarks",
    "evidence",
    "expected",
    "missed",
    "recall",
    "report",
    "suite",
    "trace",
];

const DOCS_INTENT: &[&str] = &[
    "doc",
    "docs",
    "documentation",
    "guide",
    "readme",
    "workflow",
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

fn has_benchmark_evidence_intent(query_tokens: &[String]) -> bool {
    query_tokens
        .iter()
        .any(|token| BENCHMARK_EVIDENCE_INTENT.contains(&token.as_str()))
}

fn has_docs_intent(query_tokens: &[String]) -> bool {
    query_tokens
        .iter()
        .any(|token| DOCS_INTENT.contains(&token.as_str()))
}

fn expand_query_tokens(tokens: Vec<String>) -> Vec<String> {
    let mut expanded = BTreeSet::new();
    for token in tokens {
        push_token_variant(&mut expanded, &token);
    }
    expanded.into_iter().collect()
}

fn push_token_variant(expanded: &mut BTreeSet<String>, token: &str) {
    if token.is_empty() {
        return;
    }

    expanded.insert(token.to_string());

    match token {
        "configuration" | "configure" | "configured" | "configuring" => {
            expanded.insert("config".to_string());
        }
        "config" => {
            expanded.insert("configuration".to_string());
        }
        "file" => {
            expanded.insert("files".to_string());
        }
        "files" => {
            expanded.insert("file".to_string());
        }
        "import" => {
            expanded.insert("imports".to_string());
        }
        "imports" => {
            expanded.insert("import".to_string());
        }
        "reference" => {
            expanded.insert("references".to_string());
        }
        "references" => {
            expanded.insert("reference".to_string());
        }
        "filter" => {
            expanded.insert("filters".to_string());
        }
        "filters" => {
            expanded.insert("filter".to_string());
        }
        "symbol" => {
            expanded.insert("symbols".to_string());
        }
        "symbols" => {
            expanded.insert("symbol".to_string());
        }
        "request" => {
            expanded.insert("requests".to_string());
        }
        "requests" => {
            expanded.insert("request".to_string());
        }
        "test" => {
            expanded.insert("tests".to_string());
        }
        "tests" => {
            expanded.insert("test".to_string());
        }
        "timeout" => {
            expanded.insert("timeouts".to_string());
        }
        "timeouts" => {
            expanded.insert("timeout".to_string());
        }
        "string" => {
            expanded.insert("strings".to_string());
        }
        "strings" => {
            expanded.insert("string".to_string());
        }
        "routing" | "routes" | "route" => {
            expanded.insert("router".to_string());
            expanded.insert("route".to_string());
            expanded.insert("routing".to_string());
        }
        "walking" => {
            expanded.insert("walk".to_string());
        }
        "extractor" | "extractors" | "extraction" => {
            expanded.insert("extract".to_string());
        }
        "rejections" | "reject" | "rejected" => {
            expanded.insert("rejection".to_string());
        }
        "statistics" | "statistic" => {
            expanded.insert("stats".to_string());
            expanded.insert("stat".to_string());
        }
        "stats" => {
            expanded.insert("statistics".to_string());
            expanded.insert("statistic".to_string());
        }
        _ => {}
    }
}

fn basename_cluster_score(file: &FileRecord, query_tokens: &[String]) -> Option<i32> {
    let basename = file.path.rsplit('/').next().unwrap_or(file.path.as_str());
    let basename_terms: BTreeSet<String> = path_tokens(basename).into_iter().collect();
    let overlap = query_tokens
        .iter()
        .filter(|token| basename_terms.contains(*token))
        .count();

    match overlap {
        count if count >= 3 => Some(180),
        2 => Some(140),
        1 if file.is_test => Some(80),
        _ => None,
    }
}

fn is_module_anchor_match(file: &FileRecord, query_tokens: &[String]) -> bool {
    if !file.path.ends_with("/mod.rs") && !file.path.ends_with("/__init__.py") {
        return false;
    }

    file.path
        .rsplit_once('/')
        .and_then(|(parent_path, _)| parent_path.rsplit('/').next())
        .map(|segment| {
            let parent_tokens = formatter::tokenize(segment);
            parent_tokens
                .iter()
                .any(|parent| query_tokens.iter().any(|token| token == parent))
        })
        .unwrap_or(false)
}

fn is_benchmark_file(file: &FileRecord) -> bool {
    file.path.to_ascii_lowercase().starts_with("benchmarks/")
}

fn is_docs_file(file: &FileRecord) -> bool {
    let path = file.path.to_ascii_lowercase();
    path.starts_with("docs/") || path == "readme.md"
}

fn is_fixture_data(file: &FileRecord) -> bool {
    let path = file.path.to_ascii_lowercase();
    path.contains("/fixtures/")
        || path.contains("/testdata/")
        || path.contains("/tests/data/")
        || path.contains("/tests/fixtures/")
}

fn has_test_intent(query_tokens: &[String]) -> bool {
    query_tokens
        .iter()
        .any(|token| matches!(token.as_str(), "test" | "tests" | "fixture" | "fixtures"))
}

fn is_readme(file: &FileRecord) -> bool {
    file.path.eq_ignore_ascii_case("README.md")
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
