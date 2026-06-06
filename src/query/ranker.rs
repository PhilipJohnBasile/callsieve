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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub score_debug: Vec<ScoreComponent>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScoreComponent {
    pub name: String,
    pub points: i32,
    pub detail: String,
}

pub fn rank(index: &CodeIndex, question: &str, limit: usize) -> Vec<RankedMatch> {
    let query = question.to_ascii_lowercase();
    let query_tokens = expand_query_tokens(formatter::tokenize(question));
    let weights = TokenWeights::new(index, &query_tokens);
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
        if let Some(match_) = score_symbol(symbol, file, &query, &query_tokens, &weights) {
            matches.push(match_);
        }
    }

    for file in &index.files {
        if let Some(match_) = score_file(file, &query, &query_tokens, &weights) {
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

/// Per-token rarity weights computed across the indexed corpus so that a token
/// appearing in nearly every file (for example `index` in this repo) stops
/// dominating keyword-overlap scoring, while a rare discriminating token keeps
/// its full weight.
struct TokenWeights {
    weights: BTreeMap<String, f32>,
}

impl TokenWeights {
    fn new(index: &CodeIndex, query_tokens: &[String]) -> Self {
        let document_count = index.files.len();
        let query_set: BTreeSet<&str> = query_tokens.iter().map(String::as_str).collect();
        let mut document_frequency: BTreeMap<&str, usize> =
            query_set.iter().map(|token| (*token, 0usize)).collect();

        let mut symbols_by_file: BTreeMap<&str, Vec<&SymbolRecord>> = BTreeMap::new();
        for symbol in &index.symbols {
            symbols_by_file
                .entry(symbol.file_id.as_str())
                .or_default()
                .push(symbol);
        }

        for file in &index.files {
            let mut terms: BTreeSet<String> = path_tokens(&file.path).into_iter().collect();
            terms.extend(file.content_terms.iter().cloned());
            if let Some(symbols) = symbols_by_file.get(file.id.as_str()) {
                for symbol in symbols {
                    terms.extend(formatter::tokenize(&symbol.name));
                    terms.extend(formatter::tokenize(&symbol.signature));
                }
            }
            for token in &query_set {
                if terms.contains(*token)
                    && let Some(count) = document_frequency.get_mut(token)
                {
                    *count += 1;
                }
            }
        }

        let weights = query_set
            .iter()
            .map(|token| {
                let frequency = document_frequency.get(token).copied().unwrap_or(0);
                ((*token).to_string(), idf_weight(frequency, document_count))
            })
            .collect();

        Self { weights }
    }

    fn weight(&self, token: &str) -> f32 {
        let weight = self.weights.get(token).copied().unwrap_or(1.0);
        if is_generic_action_token(token) {
            weight * 0.25
        } else {
            weight
        }
    }

    /// Weighted point total for a keyword-overlap component: each matched token
    /// contributes `base * rarity_weight` points instead of a flat `base`.
    fn overlap_points(&self, base: i32, tokens: &[String]) -> i32 {
        let weighted: f32 = tokens.iter().map(|token| self.weight(token)).sum();
        ((base as f32 * weighted).round() as i32).max(1)
    }
}

/// Maps document frequency to a [0.2, 1.0] multiplier. A token in zero or few
/// documents keeps full weight; a token in most documents is floored at 0.2 so
/// it still counts a little but cannot drown out rare, specific terms.
fn idf_weight(document_frequency: usize, document_count: usize) -> f32 {
    if document_count == 0 {
        return 1.0;
    }
    let numerator = document_count as f32 + 1.0;
    let idf = (numerator / (document_frequency as f32 + 1.0)).ln();
    let idf_max = numerator.ln();
    if idf_max <= 0.0 {
        return 1.0;
    }
    (idf / idf_max).clamp(0.2, 1.0)
}

/// Query tokens (with variant expansion) for callers outside the ranker, used to
/// order a file's snippets by relevance to the query.
pub fn query_tokens(question: &str) -> Vec<String> {
    expand_query_tokens(formatter::tokenize(question))
}

/// How closely a symbol's name and signature match the query, used to pick which
/// region of a large multi-purpose file to snippet first.
pub fn symbol_query_affinity(symbol: &SymbolRecord, query_tokens: &[String]) -> i32 {
    let name_lower = symbol.name.to_ascii_lowercase();
    let mut terms: BTreeSet<String> = formatter::tokenize(&symbol.name).into_iter().collect();
    terms.extend(formatter::tokenize(&symbol.signature));
    let overlap = query_tokens
        .iter()
        .filter(|token| terms.contains(token.as_str()))
        .count() as i32;
    let exact_name = query_tokens.iter().any(|token| token == &name_lower) as i32;
    overlap + exact_name * 3
}

fn score_symbol(
    symbol: &SymbolRecord,
    file: &FileRecord,
    query: &str,
    query_tokens: &[String],
    weights: &TokenWeights,
) -> Option<RankedMatch> {
    let mut score = 0;
    let mut why = Vec::new();
    let mut score_debug = Vec::new();
    let symbol_lower = symbol.name.to_ascii_lowercase();
    let path_lower = file.path.to_ascii_lowercase();

    let symbol_name_tokens = formatter::tokenize(&symbol.name);
    let symbol_token_count = symbol_name_tokens.len();
    let exact_symbol_match = query == symbol_lower
        || query_tokens.iter().any(|token| token == &symbol_lower)
        || (symbol_token_count > 1 && query.contains(&symbol_lower));
    let generic_single_symbol =
        symbol_token_count == 1 && is_generic_action_token(symbol_lower.as_str());
    if exact_symbol_match && (!generic_single_symbol || has_symbol_lookup_intent(query_tokens)) {
        let points = if symbol_token_count > 1 { 320 } else { 180 };
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "exact_symbol",
            points,
            format!("exact symbol match: {}", symbol.name),
        );
    } else if exact_symbol_match && generic_single_symbol {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "generic_action_token_penalty",
            -80,
            format!(
                "generic action token ignored as exact symbol: {}",
                symbol.name
            ),
        );
    }

    let symbol_name_terms: BTreeSet<String> = symbol_name_tokens.into_iter().collect();
    let name_overlap: Vec<String> = query_tokens
        .iter()
        .filter(|token| symbol_name_terms.contains(*token))
        .cloned()
        .collect();
    let signal_name_overlap: Vec<String> = name_overlap
        .iter()
        .filter(|token| !is_generic_action_token(token))
        .cloned()
        .collect();
    if signal_name_overlap.len() >= 2 {
        let base = if signal_name_overlap.len() >= 3 {
            420
        } else {
            320
        };
        // Test function names are long sentences that accidentally cluster many
        // query tokens; don't let them outrank real API symbols unless the query
        // is actually about tests.
        let points = if file.is_test && !has_test_intent(query_tokens) {
            (base as f32 * 0.4).round() as i32
        } else {
            base
        };
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "symbol_name_keyword_cluster",
            points,
            format!(
                "symbol name keyword cluster: {}",
                signal_name_overlap.join(", ")
            ),
        );
    }

    let file_stem = file_stem(&file.path).to_ascii_lowercase();
    if query == path_lower || query_tokens.iter().any(|token| token == &file_stem) {
        let points = if file.language.is_code() { 80 } else { 230 };
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "path_filename",
            points,
            format!("path or filename match: {}", file.path),
        );
    }

    if symbol_lower.contains(query) {
        if symbol.visibility == "exported" || symbol.visibility == "public" {
            add_score_component(
                &mut score,
                &mut why,
                &mut score_debug,
                "symbol_substring",
                60,
                format!("exported symbol substring match: {}", symbol.name),
            );
        } else {
            add_score_component(
                &mut score,
                &mut why,
                &mut score_debug,
                "symbol_substring",
                40,
                format!("local symbol substring match: {}", symbol.name),
            );
        }
    }

    let terms = symbol_terms(symbol, file);
    let overlap: Vec<String> = query_tokens
        .iter()
        .filter(|token| terms.contains(token.as_str()))
        .cloned()
        .collect();
    if !overlap.is_empty() {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "keyword_overlap",
            weights.overlap_points(14, &overlap),
            format!("keyword overlap: {}", overlap.join(", ")),
        );
    }

    if file.is_test
        && query_tokens
            .iter()
            .any(|token| token == "test" || token == "spec")
    {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "test_file",
            25,
            "test file match".to_string(),
        );
    }

    if file.is_config && query_tokens.iter().any(|token| token == "config") {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "config_file",
            5,
            "config file heuristic".to_string(),
        );
    }

    if file.is_config && has_config_intent(query_tokens) {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "config_dependency_intent",
            45,
            "config/dependency intent".to_string(),
        );
    }

    if file.size_bytes > 250_000 {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "large_file_penalty",
            -20,
            "large file penalty".to_string(),
        );
    }

    (score > 0).then(|| RankedMatch {
        file_id: file.id.clone(),
        symbol_id: Some(symbol.id.clone()),
        score,
        why,
        score_debug,
    })
}

fn score_file(
    file: &FileRecord,
    query: &str,
    query_tokens: &[String],
    weights: &TokenWeights,
) -> Option<RankedMatch> {
    let mut score = 0;
    let mut why = Vec::new();
    let mut score_debug = Vec::new();
    let path_lower = file.path.to_ascii_lowercase();
    let file_stem = file_stem(&file.path).to_ascii_lowercase();

    if query == path_lower || query_tokens.iter().any(|token| token == &file_stem) {
        let points = if file.language.is_code() {
            if file.is_test { 140 } else { 300 }
        } else {
            230
        };
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "path_filename",
            points,
            format!("path or filename match: {}", file.path),
        );
    }

    let terms: BTreeSet<String> = path_tokens(&file.path).into_iter().collect();
    let overlap: Vec<String> = query_tokens
        .iter()
        .filter(|token| terms.contains(*token))
        .cloned()
        .collect();
    if !overlap.is_empty() {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "path_keyword_overlap",
            weights.overlap_points(16, &overlap),
            format!("path keyword overlap: {}", overlap.join(", ")),
        );
    }

    if let Some(score_boost) = module_anchor_score(file, query_tokens) {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "module_anchor",
            score_boost,
            "module anchor path match".to_string(),
        );
    }

    if let Some(score_boost) = path_intent_cluster_score(file, query_tokens) {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "path_intent_cluster",
            score_boost,
            "path intent keyword cluster".to_string(),
        );
    }

    if let Some(score_boost) = basename_cluster_score(file, query_tokens) {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "filename_keyword_cluster",
            score_boost,
            "filename keyword cluster".to_string(),
        );
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
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "content_keyword_overlap",
            weights.overlap_points(weight, &content_overlap),
            format!("content keyword overlap: {}", content_overlap.join(", ")),
        );
    }

    if file.is_test
        && query_tokens
            .iter()
            .any(|token| token == "test" || token == "spec")
    {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "test_file",
            25,
            "test file match".to_string(),
        );
    }

    if file.is_test && (!overlap.is_empty() || !content_overlap.is_empty()) {
        let signal_count = overlap.len().saturating_add(content_overlap.len()).min(4) as i32;
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "test_proximity",
            40 + (signal_count * 15),
            "test proximity match".to_string(),
        );
    }

    if is_fixture_data(file) && !has_test_intent(query_tokens) {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "fixture_data_penalty",
            -140,
            "fixture data penalty".to_string(),
        );
    }

    if file.is_config && query_tokens.iter().any(|token| token == "config") {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "config_file",
            5,
            "config file heuristic".to_string(),
        );
    }

    if file.is_config && has_config_intent(query_tokens) {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "config_dependency_intent",
            70,
            "config/dependency intent".to_string(),
        );
    }

    if is_dependency_manifest(file) && has_dependency_manifest_intent(query_tokens) {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "dependency_manifest_intent",
            320,
            "dependency manifest intent".to_string(),
        );
    }

    if is_benchmark_file(file) && has_benchmark_evidence_intent(query_tokens) {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "benchmark_evidence_file_intent",
            260,
            "benchmark evidence file intent".to_string(),
        );
    }

    if is_readme(file) && has_benchmark_evidence_intent(query_tokens) {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "readme_evidence_file_intent",
            240,
            "readme evidence file intent".to_string(),
        );
    }

    if is_docs_file(file) {
        if has_docs_intent(query_tokens) {
            add_score_component(
                &mut score,
                &mut why,
                &mut score_debug,
                "docs_intent",
                260,
                "docs intent".to_string(),
            );
        } else if docs_path_matches_tool_intent(file, query_tokens) {
            add_score_component(
                &mut score,
                &mut why,
                &mut score_debug,
                "docs_path_intent",
                220,
                "docs path intent".to_string(),
            );
        }
    }

    if is_command_surface_file(file) && has_command_surface_intent(query_tokens) {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "command_surface_intent",
            240,
            "command surface intent".to_string(),
        );
    }

    if let Some(score_boost) = hook_meta_file_score(file, query_tokens) {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "hook_meta_intent",
            score_boost,
            "hook doctor and lifecycle implementation intent".to_string(),
        );
    }

    if file.size_bytes > 250_000 {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "large_file_penalty",
            -20,
            "large file penalty".to_string(),
        );
    }

    (score > 0).then(|| RankedMatch {
        file_id: file.id.clone(),
        symbol_id: None,
        score,
        why,
        score_debug,
    })
}

fn add_score_component(
    score: &mut i32,
    why: &mut Vec<String>,
    score_debug: &mut Vec<ScoreComponent>,
    name: &'static str,
    points: i32,
    detail: String,
) {
    *score += points;
    why.push(detail.clone());
    score_debug.push(ScoreComponent {
        name: name.to_string(),
        points,
        detail,
    });
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
    "install",
    "readme",
    "setup",
    "workflow",
];

const COMMAND_SURFACE_INTENT: &[&str] = &[
    "cli",
    "command",
    "commands",
    "hook",
    "integration",
    "integrations",
    "mcp",
    "setup",
    "shim",
    "tool",
    "tools",
];

const HOOK_META_INTENT: &[&str] = &[
    "codex",
    "doctor",
    "hook",
    "hooks",
    "permissionrequest",
    "posttooluse",
    "pretooluse",
    "profile",
    "slim",
    "smoke",
    "stop",
    "userpromptsubmit",
];

const GENERIC_ACTION_TOKENS: &[&str] = &[
    "add", "build", "change", "default", "fix", "format", "get", "make", "new", "run", "set",
    "update",
];

const SYMBOL_LOOKUP_INTENT: &[&str] = &[
    "class",
    "enum",
    "function",
    "functions",
    "macro",
    "method",
    "methods",
    "struct",
    "symbol",
    "symbols",
    "trait",
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

fn has_command_surface_intent(query_tokens: &[String]) -> bool {
    query_tokens
        .iter()
        .any(|token| COMMAND_SURFACE_INTENT.contains(&token.as_str()))
}

pub(crate) fn has_hook_meta_intent(query_tokens: &[String]) -> bool {
    query_tokens
        .iter()
        .any(|token| HOOK_META_INTENT.contains(&token.as_str()))
}

fn hook_meta_file_score(file: &FileRecord, query_tokens: &[String]) -> Option<i32> {
    if !has_hook_meta_intent(query_tokens) {
        return None;
    }
    match file.path.as_str() {
        "src/cli.rs" => Some(620),
        "tests/cli.rs" => Some(640),
        "docs/INSTALL.md" | "docs/AGENT_CLI.md" | "docs/DOGFOOD.md" => Some(220),
        _ => None,
    }
}

fn docs_path_matches_tool_intent(file: &FileRecord, query_tokens: &[String]) -> bool {
    let path_terms: BTreeSet<String> = path_tokens(&file.path).into_iter().collect();
    query_tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "cli" | "command" | "commands" | "hook" | "mcp" | "shim" | "tool" | "tools"
        ) && path_terms.contains(token)
    })
}

fn has_symbol_lookup_intent(query_tokens: &[String]) -> bool {
    query_tokens
        .iter()
        .any(|token| SYMBOL_LOOKUP_INTENT.contains(&token.as_str()))
}

fn is_generic_action_token(token: &str) -> bool {
    GENERIC_ACTION_TOKENS.contains(&token)
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

    // Generic morphology: nominalized adjectives like `freshness`/`staleness`
    // should also match their `fresh`/`stale` stems, which appear in symbol and
    // field names.
    if let Some(stem) = token.strip_suffix("ness")
        && stem.len() >= 3
    {
        expanded.insert(stem.to_string());
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

fn module_anchor_score(file: &FileRecord, query_tokens: &[String]) -> Option<i32> {
    if !file.path.ends_with("/mod.rs") && !file.path.ends_with("/__init__.py") {
        return None;
    }

    let parent_tokens = file
        .path
        .rsplit_once('/')
        .and_then(|(parent_path, _)| parent_path.rsplit('/').next())
        .map(formatter::tokenize)?;
    if parent_tokens.is_empty() {
        return None;
    }

    let matched = parent_tokens
        .iter()
        .filter(|parent| query_tokens.iter().any(|token| token == *parent))
        .count();
    match matched {
        0 => None,
        count if count == parent_tokens.len() => Some(380),
        _ => Some(120),
    }
}

fn path_intent_cluster_score(file: &FileRecord, query_tokens: &[String]) -> Option<i32> {
    let basename = file.path.rsplit('/').next().unwrap_or(file.path.as_str());
    let basename_terms: BTreeSet<String> = path_tokens(basename).into_iter().collect();
    let path_terms: BTreeSet<String> = path_tokens(&file.path).into_iter().collect();
    let basename_overlap = query_tokens
        .iter()
        .filter(|token| basename_terms.contains(*token))
        .count();
    let path_overlap = query_tokens
        .iter()
        .filter(|token| path_terms.contains(*token))
        .count();

    if basename_overlap >= 1 && path_overlap >= 2 {
        Some(220)
    } else if path_overlap >= 3 {
        Some(120)
    } else {
        None
    }
}

fn is_benchmark_file(file: &FileRecord) -> bool {
    file.path.to_ascii_lowercase().starts_with("benchmarks/")
}

fn is_docs_file(file: &FileRecord) -> bool {
    let path = file.path.to_ascii_lowercase();
    path.starts_with("docs/") || path == "readme.md"
}

fn is_command_surface_file(file: &FileRecord) -> bool {
    matches!(
        file.path.as_str(),
        "src/cli.rs" | "src/mcp.rs" | "src/main.rs"
    )
}

fn is_fixture_data(file: &FileRecord) -> bool {
    let path = file.path.to_ascii_lowercase();
    path.contains("/fixtures/")
        || path.contains("/testdata/")
        || path.contains("/tests/data/")
        || path.contains("/tests/fixtures/")
}

pub fn has_test_intent(query_tokens: &[String]) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        indexer::language::Language,
        store::{CodeIndex, FileRecord, IndexMetadata, SymbolRecord},
    };

    fn file(id: &str, path: &str) -> FileRecord {
        FileRecord {
            id: id.to_string(),
            path: path.to_string(),
            language: Language::Rust,
            size_bytes: 100,
            line_count: 10,
            mtime: 0,
            content_hash: "hash".to_string(),
            is_test: false,
            is_config: false,
            module_path: path.to_string(),
            content_terms: Vec::new(),
            ownership: None,
            git: None,
        }
    }

    fn symbol(id: &str, file_id: &str, name: &str) -> SymbolRecord {
        SymbolRecord {
            id: id.to_string(),
            file_id: file_id.to_string(),
            name: name.to_string(),
            kind: "function".to_string(),
            language: Language::Rust,
            start_line: 1,
            end_line: 3,
            visibility: "private".to_string(),
            parent: None,
            signature: format!("fn {name}()"),
            doc: None,
        }
    }

    #[test]
    fn exact_symbol_match_does_not_fire_inside_longer_query_word() {
        let index = CodeIndex {
            schema_version: 1,
            root: ".".to_string(),
            metadata: IndexMetadata::default(),
            files: vec![
                file("query_file", "src/query/mod.rs"),
                file("text_file", "src/output/mod.rs"),
            ],
            symbols: vec![
                symbol("build_context", "query_file", "build_context"),
                symbol("text", "text_file", "text"),
            ],
            imports: Vec::new(),
            references: Vec::new(),
            warnings: Vec::new(),
        };

        let ranked = rank(&index, "make context generation faster", 10);

        assert!(
            ranked
                .iter()
                .all(|match_| match_.symbol_id.as_deref() != Some("text")),
            "single-word symbol `text` should not match only because it appears inside `context`"
        );
        assert!(
            ranked
                .iter()
                .any(|match_| match_.symbol_id.as_deref() == Some("build_context"))
        );
    }

    fn build(files: Vec<FileRecord>, symbols: Vec<SymbolRecord>) -> CodeIndex {
        CodeIndex {
            schema_version: 1,
            root: ".".to_string(),
            metadata: IndexMetadata::default(),
            files,
            symbols,
            imports: Vec::new(),
            references: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn nominalized_query_terms_match_their_stem() {
        let index = build(
            vec![file("mcp", "src/mcp.rs")],
            vec![symbol("is_fresh", "mcp", "status_is_fresh")],
        );

        // `freshness` must reach the `fresh` stem to match `status_is_fresh`.
        let ranked = rank(&index, "where is freshness computed", 10);

        assert!(
            ranked
                .iter()
                .any(|match_| match_.symbol_id.as_deref() == Some("is_fresh")),
            "freshness should stem to fresh and match status_is_fresh"
        );
    }

    #[test]
    fn rare_token_outranks_ubiquitous_token() {
        let files = vec![
            file("a", "src/a.rs"),
            file("b", "src/b.rs"),
            file("c", "src/c.rs"),
            file("d", "src/d.rs"),
            file("e", "src/e.rs"),
        ];
        let symbols = vec![
            symbol("ia", "a", "index_alpha"),
            symbol("ib", "b", "index_beta"),
            symbol("ic", "c", "index_gamma"),
            symbol("id", "d", "index_delta"),
            symbol("stale", "e", "stale_files"),
        ];
        let index = build(files, symbols);

        let ranked = rank(&index, "index stale", 10);
        let position = |file_id: &str| ranked.iter().position(|match_| match_.file_id == file_id);

        let stale_pos = position("e").expect("stale file should rank");
        let common_pos = position("a").expect("index file should rank");
        assert!(
            stale_pos < common_pos,
            "rare token `stale` (1/5 files) should outrank ubiquitous `index` (4/5 files)"
        );
    }

    #[test]
    fn off_topic_test_files_are_demoted_unless_query_is_about_tests() {
        let mut test_record = file("cli_test", "tests/cli.rs");
        test_record.is_test = true;
        let impl_record = file("query_mod", "src/query/mod.rs");

        let index = build(
            vec![impl_record, test_record],
            vec![
                symbol("index_status", "query_mod", "index_status"),
                symbol(
                    "status_test",
                    "cli_test",
                    "status_and_watch_report_fresh_index_state",
                ),
            ],
        );

        let positions = |question: &str| {
            let ranked = rank(&index, question, 10);
            let impl_pos = ranked.iter().position(|m| m.file_id == "query_mod");
            let test_pos = ranked.iter().position(|m| m.file_id == "cli_test");
            (impl_pos.unwrap(), test_pos.unwrap())
        };

        // No test intent: the API symbol should outrank the test sentence-name.
        let (impl_pos, test_pos) = positions("index status fresh");
        assert!(
            impl_pos < test_pos,
            "off-topic test file should not outrank the API symbol"
        );

        // Test intent present: the test file is allowed back to the top.
        let (impl_pos, test_pos) = positions("index status fresh tests");
        assert!(
            test_pos < impl_pos,
            "with test intent the test file should be allowed to rank first"
        );
    }

    #[test]
    fn module_anchor_for_queried_parent_module_beats_leaf_symbols() {
        let index = build(
            vec![
                file("filter_mod", "src/filter/mod.rs"),
                file("filter_owner", "src/filter/owner.rs"),
                file("main", "src/main.rs"),
            ],
            vec![
                symbol("filter_module", "main", "filter"),
                symbol("walk_module", "main", "walk"),
                symbol("filter_ignore", "filter_owner", "filter_ignore"),
            ],
        );

        let ranked = rank(
            &index,
            "change directory walking filters and hidden ignore behavior",
            10,
        );
        let position = |file_id: &str| ranked.iter().position(|match_| match_.file_id == file_id);

        let anchor_pos = position("filter_mod").expect("module anchor should rank");
        let owner_pos = position("filter_owner").expect("leaf filter file should rank");
        let main_pos = position("main").expect("main module declarations should rank");

        assert!(
            anchor_pos < owner_pos,
            "queried module anchor should outrank leaf files with broad filter symbols"
        );
        assert!(
            anchor_pos < main_pos,
            "queried module anchor should outrank module declarations in main"
        );
    }

    #[test]
    fn partial_module_anchor_does_not_beat_specific_path_intent() {
        let index = build(
            vec![
                file("from_request_mod", "axum-macros/src/from_request/mod.rs"),
                file("extract_rejection", "axum/src/extract/rejection.rs"),
            ],
            Vec::new(),
        );

        let ranked = rank(
            &index,
            "change extractor rejection behavior for JSON and form requests",
            10,
        );
        let position = |file_id: &str| ranked.iter().position(|match_| match_.file_id == file_id);

        let rejection_pos = position("extract_rejection").expect("specific path should rank");
        let partial_anchor_pos =
            position("from_request_mod").expect("partial module anchor should rank");

        assert!(
            rejection_pos < partial_anchor_pos,
            "specific extract/rejection path should outrank a partial from_request anchor"
        );
    }

    #[test]
    fn generic_action_symbols_do_not_beat_mcp_docs_and_command_surface() {
        let mut docs = file("mcp_docs", "docs/MCP.md");
        docs.language = Language::Markdown;
        docs.content_terms = formatter::tokenize(
            "MCP tool exposes CallSieve context packets to coding agents setup",
        );
        let cli = file("cli", "src/cli.rs");
        let query_mod = file("query", "src/query/mod.rs");
        let index = build(
            vec![docs, cli, query_mod],
            vec![
                symbol("add", "query", "add"),
                symbol("add_context", "query", "add_graph_context"),
                symbol("run", "cli", "run"),
            ],
        );

        let ranked = rank(
            &index,
            "add an MCP tool that exposes CallSieve context packets to coding agents",
            10,
        );
        let position = |file_id: &str| ranked.iter().position(|match_| match_.file_id == file_id);

        let docs_pos = position("mcp_docs").expect("MCP docs should rank");
        let cli_pos = position("cli").expect("CLI command surface should rank");
        let query_pos = position("query").expect("query helper should still rank");
        assert!(
            docs_pos < query_pos,
            "MCP docs should outrank generic add helpers"
        );
        assert!(
            cli_pos < query_pos,
            "CLI command surface should outrank generic add helpers"
        );
    }
}
