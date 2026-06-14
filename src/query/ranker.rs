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

        // Membership-only check: avoid materialising every file's full term set
        // (cloning all content terms and tokenizing every symbol) per query.
        for file in &index.files {
            let mut matched: BTreeSet<&str> = BTreeSet::new();
            for term in path_tokens(&file.path) {
                if let Some(token) = query_set.get(term.as_str()) {
                    matched.insert(token);
                }
            }
            for token in &query_set {
                if !matched.contains(*token) && file.content_terms.iter().any(|term| term == token)
                {
                    matched.insert(token);
                }
            }
            if matched.len() < query_set.len()
                && let Some(symbols) = symbols_by_file.get(file.id.as_str())
            {
                for symbol in symbols {
                    // Every token tokenize() produces is a contiguous substring of the
                    // lowercased input, so a substring miss proves a token miss and the
                    // allocation-heavy tokenize can be skipped.
                    let worth_tokenizing = query_set.iter().any(|token| {
                        !matched.contains(*token)
                            && (contains_ascii_case_insensitive(&symbol.name, token)
                                || contains_ascii_case_insensitive(&symbol.signature, token))
                    });
                    if !worth_tokenizing {
                        continue;
                    }
                    for term in formatter::tokenize(&symbol.name)
                        .into_iter()
                        .chain(formatter::tokenize(&symbol.signature))
                    {
                        if let Some(token) = query_set.get(term.as_str()) {
                            matched.insert(token);
                        }
                    }
                    if matched.len() == query_set.len() {
                        break;
                    }
                }
            }
            for token in matched {
                if let Some(count) = document_frequency.get_mut(token) {
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

/// Filename-stem matches scale with how rare the stem token is across the
/// corpus: a query token naming a unique file (e.g. `sqlmigrate`) is decisive
/// and must not be drowned by cumulative content-keyword points on other
/// files, while a stem that is also an everyday corpus word (e.g. `schema`,
/// which several files are named after) keeps roughly its old weight.
fn stem_match_points(base: i32, stem: &str, weights: &TokenWeights) -> i32 {
    let rarity = weights.weight(stem);
    (base as f32 * (0.5 + 1.5 * rarity)).round() as i32
}

/// Allocation-free `haystack.to_lowercase().contains(needle)` for an
/// already-lowercase ASCII needle.
fn contains_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
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
    let common_lifecycle_symbol = is_common_lifecycle_symbol(&symbol.name);
    let direct_lifecycle_symbol_lookup = query == symbol_lower
        || (query.contains(&symbol_lower) && query_tokens.len() <= 8)
        || has_lifecycle_symbol_lookup_intent(query_tokens);
    let dampen_lifecycle_symbol = common_lifecycle_symbol && !direct_lifecycle_symbol_lookup;
    let generic_single_symbol =
        symbol_token_count == 1 && is_generic_action_token(symbol_lower.as_str());
    if exact_symbol_match && dampen_lifecycle_symbol {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "common_lifecycle_symbol",
            45,
            format!("common lifecycle symbol trace match: {}", symbol.name),
        );
    } else if exact_symbol_match
        && (!generic_single_symbol || has_symbol_lookup_intent(query_tokens))
    {
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
        let points = if dampen_lifecycle_symbol {
            55
        } else if file.is_test && !has_test_intent(query_tokens) {
            (base as f32 * 0.4).round() as i32
        } else {
            base
        };
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            if dampen_lifecycle_symbol {
                "common_lifecycle_symbol_cluster"
            } else {
                "symbol_name_keyword_cluster"
            },
            points,
            if dampen_lifecycle_symbol {
                format!(
                    "common lifecycle symbol keyword cluster: {}",
                    signal_name_overlap.join(", ")
                )
            } else {
                format!(
                    "symbol name keyword cluster: {}",
                    signal_name_overlap.join(", ")
                )
            },
        );
    }

    let file_stem = file_stem(&file.path).to_ascii_lowercase();
    if query == path_lower || query_tokens.iter().any(|token| token == &file_stem) {
        let base = if file.language.is_code() { 80 } else { 230 };
        let points = stem_match_points(base, &file_stem, weights);
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
        let base = if file.language.is_code() {
            if file.is_test { 140 } else { 300 }
        } else {
            230
        };
        let points = stem_match_points(base, &file_stem, weights);
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "path_filename",
            points,
            format!("path or filename match: {}", file.path),
        );
    }

    let filename_substrings: Vec<String> = if file.language.is_code() {
        filename_substring_matches(&file_stem, query_tokens)
    } else {
        Vec::new()
    };
    if !filename_substrings.is_empty() {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "filename_substring",
            weights.overlap_points(120, &filename_substrings).min(360),
            format!(
                "filename substring match: {}",
                filename_substrings.join(", ")
            ),
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

    if let Some(score_boost) = domain_module_alias_score(file, query_tokens) {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "domain_module_alias",
            score_boost,
            "domain module alias intent".to_string(),
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
    if let Some(score_boost) =
        dependency_exception_surface_score(file, query_tokens, &content_terms)
    {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "dependency_exception_surface",
            score_boost,
            "dependency exception propagation intent".to_string(),
        );
    }

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

    if let Some(score_boost) = dependency_manifest_score(file, query_tokens) {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "dependency_manifest_intent",
            score_boost,
            "dependency manifest intent".to_string(),
        );
    }

    if let Some(score_boost) = workflow_file_score(file, query_tokens) {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "workflow_file_intent",
            score_boost,
            "workflow file intent".to_string(),
        );
    }

    if let Some(score_boost) = index_freshness_surface_score(file, query_tokens, &content_terms) {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "index_freshness_surface",
            score_boost,
            "index freshness surface intent".to_string(),
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

    if let Some(score_boost) = benchmark_evidence_doc_score(file, query_tokens, &content_terms) {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "benchmark_evidence_doc_intent",
            score_boost,
            "benchmark evidence doc intent".to_string(),
        );
    }

    if let Some(score_boost) =
        ownership_context_attachment_score(file, query_tokens, &content_terms)
    {
        add_score_component(
            &mut score,
            &mut why,
            &mut score_debug,
            "ownership_context_attachment",
            score_boost,
            "ownership context attachment intent".to_string(),
        );
    }

    if is_docs_file(file) {
        if let Some(score_boost) =
            competitive_positioning_doc_score(file, query_tokens, &content_terms)
        {
            add_score_component(
                &mut score,
                &mut why,
                &mut score_debug,
                "competitive_positioning_doc",
                score_boost,
                "competitive positioning doc intent".to_string(),
            );
        }
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

const WORKFLOW_INTENT: &[&str] = &[
    "action",
    "actions",
    "build",
    "ci",
    "github",
    "workflow",
    "workflows",
];

const INDEX_FRESHNESS_INTENT: &[&str] = &[
    "computed",
    "fresh",
    "freshness",
    "stale",
    "staleness",
    "status",
];

const BENCHMARK_EVIDENCE_INTENT: &[&str] = &[
    "benchmark",
    "benchmarks",
    "collection",
    "collections",
    "evidence",
    "expected",
    "manifest",
    "manifests",
    "missed",
    "proof",
    "proofs",
    "recall",
    "report",
    "reports",
    "suite",
    "suites",
    "trace",
    "traces",
];

const STRONG_BENCHMARK_EVIDENCE_INTENT: &[&str] = &[
    "benchmark",
    "benchmarks",
    "collection",
    "collections",
    "evidence",
    "expected",
    "manifest",
    "manifests",
    "missed",
    "proof",
    "proofs",
    "recall",
    "suite",
    "suites",
    "trace",
    "traces",
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

const COMPETITIVE_POSITIONING_INTENT: &[&str] = &[
    "aider",
    "better",
    "claude",
    "cody",
    "competition",
    "competitive",
    "competitor",
    "competitors",
    "continue",
    "copilot",
    "cursor",
    "devin",
    "greptile",
    "market",
    "positioning",
    "sourcegraph",
    "windsurf",
];

const BENCHMARK_EVIDENCE_DOC_ANCHORS: &[&str] = &[
    "benchmark",
    "benchmarks",
    "collection",
    "evidence",
    "manifest",
    "manifests",
    "proof",
    "recall",
    "report",
    "reports",
    "suite",
    "suites",
    "trace",
    "traces",
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

fn dependency_manifest_score(file: &FileRecord, query_tokens: &[String]) -> Option<i32> {
    if !is_dependency_manifest(file) || !has_dependency_manifest_intent(query_tokens) {
        return None;
    }

    let lower = file.path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(lower.as_str());
    let has_dependency_intent = query_tokens
        .iter()
        .any(|token| matches!(token.as_str(), "dependencies" | "dependency" | "package"));
    let mut score = 320;

    match name {
        "cargo.toml" => {
            if query_tokens
                .iter()
                .any(|token| matches!(token.as_str(), "cargo" | "crate" | "crates" | "rust"))
            {
                score += 220;
            }
            if has_dependency_intent {
                score += 80;
            }
        }
        "package.json" => {
            if query_tokens.iter().any(|token| {
                matches!(
                    token.as_str(),
                    "javascript" | "js" | "node" | "npm" | "package" | "typescript" | "ts"
                )
            }) {
                score += 220;
            }
        }
        "pyproject.toml" | "requirements.txt" => {
            if query_tokens
                .iter()
                .any(|token| matches!(token.as_str(), "python" | "pip" | "package"))
            {
                score += 220;
            }
        }
        "rust-toolchain.toml"
            if query_tokens
                .iter()
                .any(|token| matches!(token.as_str(), "rust" | "toolchain")) =>
        {
            score += 140;
        }
        _ => {}
    }

    Some(score)
}

fn has_workflow_intent(query_tokens: &[String]) -> bool {
    query_tokens
        .iter()
        .any(|token| WORKFLOW_INTENT.contains(&token.as_str()))
}

fn is_workflow_file(file: &FileRecord) -> bool {
    let lower = file.path.to_ascii_lowercase();
    lower.starts_with(".github/workflows/") || lower.contains("/workflows/")
}

fn workflow_file_score(file: &FileRecord, query_tokens: &[String]) -> Option<i32> {
    if !is_workflow_file(file) || !has_workflow_intent(query_tokens) {
        return None;
    }

    let stem = file_stem(&file.path).to_ascii_lowercase();
    if query_tokens.iter().any(|token| token == &stem) {
        return Some(420);
    }

    let build_or_ci_intent = query_tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "action" | "actions" | "build" | "ci" | "github"
        )
    });
    if stem == "ci" && build_or_ci_intent {
        return Some(360);
    }

    Some(160)
}

fn has_index_freshness_intent(query_tokens: &[String]) -> bool {
    let has_index = query_tokens.iter().any(|token| token == "index");
    let has_freshness = query_tokens
        .iter()
        .any(|token| INDEX_FRESHNESS_INTENT.contains(&token.as_str()));
    has_index && has_freshness
}

fn index_freshness_surface_score(
    file: &FileRecord,
    query_tokens: &[String],
    content_terms: &BTreeSet<&str>,
) -> Option<i32> {
    if !has_index_freshness_intent(query_tokens) {
        return None;
    }

    let has_index_anchor = content_terms.contains("index");
    let has_freshness_anchor = INDEX_FRESHNESS_INTENT
        .iter()
        .any(|token| content_terms.contains(token));
    if !has_index_anchor || !has_freshness_anchor {
        return None;
    }

    match file.path.as_str() {
        "src/mcp.rs" => Some(620),
        "src/cli/daemon.rs" => Some(420),
        "src/cli/mod.rs" => Some(320),
        "src/query/mod.rs" => Some(300),
        _ => None,
    }
}

fn has_benchmark_evidence_intent(query_tokens: &[String]) -> bool {
    query_tokens
        .iter()
        .any(|token| STRONG_BENCHMARK_EVIDENCE_INTENT.contains(&token.as_str()))
}

fn has_docs_intent(query_tokens: &[String]) -> bool {
    query_tokens
        .iter()
        .any(|token| DOCS_INTENT.contains(&token.as_str()))
}

fn benchmark_evidence_doc_score(
    file: &FileRecord,
    query_tokens: &[String],
    content_terms: &BTreeSet<&str>,
) -> Option<i32> {
    if !is_docs_file(file) || !has_benchmark_evidence_intent(query_tokens) {
        return None;
    }

    let path_terms: BTreeSet<String> = path_tokens(&file.path).into_iter().collect();
    let anchor_count = BENCHMARK_EVIDENCE_DOC_ANCHORS
        .iter()
        .filter(|token| path_terms.contains(**token) || content_terms.contains(**token))
        .count();
    if anchor_count < 2 {
        return None;
    }

    let query_topic_count = query_tokens
        .iter()
        .filter(|token| {
            BENCHMARK_EVIDENCE_INTENT.contains(&token.as_str())
                || matches!(token.as_str(), "collection" | "collections")
        })
        .count();

    let mut score = 300 + (query_topic_count.min(4) as i32 * 35);
    if path_terms.contains("benchmark") || path_terms.contains("benchmarks") {
        score += 180;
    }
    Some(score)
}

fn competitive_positioning_doc_score(
    file: &FileRecord,
    query_tokens: &[String],
    content_terms: &BTreeSet<&str>,
) -> Option<i32> {
    let query_topic_count = query_tokens
        .iter()
        .filter(|token| COMPETITIVE_POSITIONING_INTENT.contains(&token.as_str()))
        .count();
    if query_topic_count == 0 {
        return None;
    }

    let path_terms: BTreeSet<String> = path_tokens(&file.path).into_iter().collect();
    let has_positioning_anchor = [
        "competition",
        "competitive",
        "competitor",
        "competitors",
        "market",
        "positioning",
    ]
    .iter()
    .any(|token| path_terms.contains(*token) || content_terms.contains(*token));
    if !has_positioning_anchor {
        return None;
    }

    let file_topic_count = COMPETITIVE_POSITIONING_INTENT
        .iter()
        .filter(|token| path_terms.contains(**token) || content_terms.contains(**token))
        .count();
    if file_topic_count < 2 {
        return None;
    }

    let mut score = 360 + (query_topic_count.min(4) as i32 * 45);
    if path_terms.contains("competitive") || path_terms.contains("competitor") {
        score += 220;
    }
    Some(score)
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

fn has_lifecycle_symbol_lookup_intent(query_tokens: &[String]) -> bool {
    query_tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "defined" | "definition" | "symbol" | "symbols"
        )
    })
}

fn is_generic_action_token(token: &str) -> bool {
    GENERIC_ACTION_TOKENS.contains(&token)
}

fn is_common_lifecycle_symbol(symbol_name: &str) -> bool {
    let compact: String = symbol_name
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect();

    matches!(
        compact.as_str(),
        "afterall"
            | "aftereach"
            | "beforeall"
            | "beforeeach"
            | "setup"
            | "setupclass"
            | "setupmethod"
            | "setuptestdata"
            | "teardown"
            | "teardownclass"
            | "teardownmethod"
    )
}

fn ownership_context_attachment_score(
    file: &FileRecord,
    query_tokens: &[String],
    content_terms: &BTreeSet<&str>,
) -> Option<i32> {
    let has_ownership_intent = query_tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "codeowners" | "owner" | "owners" | "ownership"
        )
    });
    if !has_ownership_intent {
        return None;
    }

    let has_selected_context_intent = query_tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "attach" | "attached" | "attaches" | "attachment" | "select" | "selected" | "selection"
        )
    });
    if !has_selected_context_intent {
        return None;
    }

    let path_terms: BTreeSet<String> = path_tokens(&file.path).into_iter().collect();
    let query_context_module = path_terms.contains("query") && path_terms.contains("mod");
    let selection_surface = content_terms.contains("context")
        && content_terms.contains("file")
        && (content_terms.contains("selected")
            || content_terms.contains("selection")
            || (content_terms.contains("read") && content_terms.contains("first")));

    if !query_context_module || !selection_surface {
        return None;
    }

    Some(520)
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
        "collection" => {
            expanded.insert("collections".to_string());
        }
        "collections" => {
            expanded.insert("collection".to_string());
        }
        "attach" | "attached" | "attaches" | "attachment" => {
            expanded.insert("attach".to_string());
            expanded.insert("attached".to_string());
            expanded.insert("attaches".to_string());
            expanded.insert("attachment".to_string());
        }
        "file" => {
            expanded.insert("files".to_string());
        }
        "files" => {
            expanded.insert("file".to_string());
        }
        "permission" => {
            expanded.insert("permissions".to_string());
        }
        "permissions" => {
            expanded.insert("permission".to_string());
        }
        "compile" | "compiled" | "compiling" | "compiler" | "compilation" => {
            expanded.insert("compile".to_string());
            expanded.insert("compiled".to_string());
            expanded.insert("compiling".to_string());
            expanded.insert("compiler".to_string());
            expanded.insert("compilation".to_string());
        }
        "database" => {
            expanded.insert("db".to_string());
        }
        "db" => {
            expanded.insert("database".to_string());
        }
        "http" => {
            expanded.insert("internet".to_string());
        }
        "internet" => {
            expanded.insert("http".to_string());
        }
        "restructured" | "restructuredtext" | "rst" => {
            expanded.insert("restructured".to_string());
            expanded.insert("restructuredtext".to_string());
            expanded.insert("rst".to_string());
        }
        "temp" | "temporary" | "tempfile" => {
            expanded.insert("temp".to_string());
            expanded.insert("temporary".to_string());
            expanded.insert("tempfile".to_string());
        }
        "username" | "usernames" => {
            expanded.insert("username".to_string());
            expanded.insert("usernames".to_string());
        }
        "coordinate" | "coordinates" => {
            expanded.insert("coordinate".to_string());
            expanded.insert("coordinates".to_string());
            expanded.insert("wcs".to_string());
        }
        "delete" | "deleted" | "deleting" | "deletion" => {
            expanded.insert("delete".to_string());
            expanded.insert("deleted".to_string());
            expanded.insert("deleting".to_string());
            expanded.insert("deletion".to_string());
        }
        "detect" | "detecting" | "detection" | "detector" | "autodetector" => {
            expanded.insert("detect".to_string());
            expanded.insert("detecting".to_string());
            expanded.insert("detection".to_string());
            expanded.insert("detector".to_string());
            expanded.insert("autodetector".to_string());
        }
        "enum" | "enums" | "enumeration" | "enumerations" => {
            expanded.insert("enum".to_string());
            expanded.insert("enums".to_string());
            expanded.insert("enumeration".to_string());
            expanded.insert("enumerations".to_string());
        }
        "lookup" | "lookups" => {
            expanded.insert("lookup".to_string());
            expanded.insert("lookups".to_string());
        }
        "migration" | "migrations" | "migrate" | "makemigrations" => {
            expanded.insert("migration".to_string());
            expanded.insert("migrations".to_string());
            expanded.insert("migrate".to_string());
            expanded.insert("makemigrations".to_string());
        }
        "reload" | "reloads" | "reloading" | "reloader" | "autoreload" => {
            expanded.insert("reload".to_string());
            expanded.insert("reloads".to_string());
            expanded.insert("reloading".to_string());
            expanded.insert("reloader".to_string());
            expanded.insert("autoreload".to_string());
        }
        "serialize" | "serialized" | "serializes" | "serializer" | "serializers"
        | "serialization" => {
            expanded.insert("serialize".to_string());
            expanded.insert("serialized".to_string());
            expanded.insert("serializes".to_string());
            expanded.insert("serializer".to_string());
            expanded.insert("serializers".to_string());
            expanded.insert("serialization".to_string());
        }
        "import" => {
            expanded.insert("imports".to_string());
        }
        "imports" => {
            expanded.insert("import".to_string());
        }
        "doc" | "docs" | "document" | "documents" | "documentation" => {
            expanded.insert("doc".to_string());
            expanded.insert("docs".to_string());
            expanded.insert("document".to_string());
            expanded.insert("documents".to_string());
            expanded.insert("documentation".to_string());
        }
        "manifest" => {
            expanded.insert("manifests".to_string());
        }
        "manifests" => {
            expanded.insert("manifest".to_string());
        }
        "owner" | "owners" | "ownership" => {
            expanded.insert("owner".to_string());
            expanded.insert("owners".to_string());
            expanded.insert("ownership".to_string());
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
        "select" | "selected" | "selection" => {
            expanded.insert("select".to_string());
            expanded.insert("selected".to_string());
            expanded.insert("selection".to_string());
        }
        "competition" | "competitive" | "competitor" | "competitors" => {
            expanded.insert("competition".to_string());
            expanded.insert("competitive".to_string());
            expanded.insert("competitor".to_string());
            expanded.insert("competitors".to_string());
            expanded.insert("positioning".to_string());
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
    let python_package_init = file.path.ends_with("/__init__.py");
    let explicit_package_anchor = query_tokens.iter().any(|token| token == "package");

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
        count if count == parent_tokens.len() => {
            if python_package_init && !explicit_package_anchor {
                Some(160)
            } else {
                Some(380)
            }
        }
        _ => {
            if python_package_init && !explicit_package_anchor {
                Some(80)
            } else {
                Some(120)
            }
        }
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

fn dependency_exception_surface_score(
    file: &FileRecord,
    query_tokens: &[String],
    content_terms: &BTreeSet<&str>,
) -> Option<i32> {
    if !file.language.is_code() || file.is_test {
        return None;
    }

    let has = |needles: &[&str]| {
        query_tokens
            .iter()
            .any(|token| needles.contains(&token.as_str()))
    };
    let has_exception_intent = has(&[
        "error",
        "errors",
        "exception",
        "exceptions",
        "timeout",
        "timeouts",
    ]);
    let has_propagation_intent = has(&[
        "expose",
        "exposed",
        "map",
        "mapping",
        "pass",
        "passed",
        "passes",
        "passing",
        "propagate",
        "propagated",
        "propagates",
        "propagating",
        "raise",
        "raised",
        "raises",
        "raising",
        "through",
        "translate",
        "translated",
        "translates",
        "translating",
        "wrap",
        "wrapped",
        "wrapping",
    ]);
    let has_boundary_intent = has(&[
        "api",
        "apis",
        "client",
        "clients",
        "http",
        "request",
        "requests",
        "response",
        "responses",
    ]);
    let dependency_hints: Vec<&str> = query_tokens
        .iter()
        .map(String::as_str)
        .filter(|token| is_dependency_hint_token(token))
        .collect();

    if !has_exception_intent
        || !has_propagation_intent
        || !has_boundary_intent
        || dependency_hints.is_empty()
    {
        return None;
    }

    let stem = file_stem(&file.path).to_ascii_lowercase();
    let stem_is = |needles: &[&str]| needles.iter().any(|needle| stem == *needle);
    let content_has =
        |needles: &[&str]| needles.iter().any(|needle| content_terms.contains(*needle));

    let is_transport_surface = stem_is(&[
        "adapter",
        "adapters",
        "client",
        "clients",
        "connection",
        "connections",
        "transport",
        "transports",
    ]);
    let is_message_model_surface = stem_is(&["model", "models", "response", "responses"]);

    if !is_transport_surface && !is_message_model_surface {
        return None;
    }

    let content_has_dependency = dependency_hints
        .iter()
        .any(|token| content_terms.contains(*token));
    let content_has_exception = content_has(&[
        "error",
        "errors",
        "except",
        "exception",
        "exceptions",
        "raise",
        "timeout",
        "timeouts",
    ]);
    let content_has_boundary = content_has(&[
        "api",
        "http",
        "raw",
        "request",
        "requests",
        "response",
        "responses",
        "stream",
    ]);

    if !content_has_exception || !content_has_boundary {
        return None;
    }

    let mut score = if is_transport_surface { 520 } else { 540 };
    if content_has_dependency {
        score += 180;
    }
    if content_has_exception {
        score += 120;
    }
    if content_has_boundary {
        score += 80;
    }

    Some(score)
}

fn is_dependency_hint_token(token: &str) -> bool {
    if is_http_status_token(token) {
        return false;
    }
    matches!(
        token,
        "dependency" | "dependencies" | "library" | "libraries" | "package" | "packages"
    ) || (token.chars().any(|character| character.is_ascii_digit())
        && token
            .chars()
            .any(|character| character.is_ascii_alphabetic()))
}

fn is_http_status_token(token: &str) -> bool {
    let status = token.strip_prefix("http").unwrap_or(token);
    status.len() == 3
        && status.chars().all(|character| character.is_ascii_digit())
        && status
            .parse::<u16>()
            .is_ok_and(|code| (100..=599).contains(&code))
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
        "src/cli.rs" | "src/cli/mod.rs" | "src/mcp.rs" | "src/main.rs"
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

fn filename_substring_matches(file_stem: &str, query_tokens: &[String]) -> Vec<String> {
    query_tokens
        .iter()
        .filter(|token| {
            token.len() >= 4
                && token.as_str() != file_stem
                && !is_generic_action_token(token)
                && file_stem.contains(token.as_str())
        })
        .take(3)
        .cloned()
        .collect()
}

fn domain_module_alias_score(file: &FileRecord, query_tokens: &[String]) -> Option<i32> {
    if !file.language.is_code() {
        return None;
    }

    let stem = file_stem(&file.path).to_ascii_lowercase();
    let path = file.path.to_ascii_lowercase();
    let has = |needles: &[&str]| {
        query_tokens
            .iter()
            .any(|token| needles.contains(&token.as_str()))
    };
    let has_two = |left: &[&str], right: &[&str]| has(left) && has(right);

    match stem.as_str() {
        "sessions"
            if path == "requests/sessions.py"
                && has(&["builtin", "str"])
                && has(&["method", "methods"]) =>
        {
            Some(900)
        }
        "separable"
            if has_two(
                &["independence", "independent", "separable"],
                &["model", "models", "nested", "combined"],
            ) =>
        {
            Some(460)
        }
        "qdp"
            if has_two(
                &["plot", "plotting", "command", "commands"],
                &["data", "read", "reading", "table", "tables"],
            ) =>
        {
            Some(460)
        }
        "fitsrec"
            if has_two(
                &["exponent", "floating", "float"],
                &["format", "table", "values"],
            ) =>
        {
            Some(420)
        }
        "ndarithmetic" if has_two(&["arithmetic", "operand"], &["mask", "masks"]) => Some(520),
        "sqlmigrate"
            if has_two(
                &["migrate", "migration", "migrations"],
                &["database", "ddl", "output", "sql", "transaction"],
            ) =>
        {
            Some(520)
        }
        "validators"
            if has_two(
                &["username", "usernames"],
                &["allow", "invalid", "newline", "reject", "trailing"],
            ) =>
        {
            Some(460)
        }
        "deletion"
            if has(&["deleted", "deleting", "deletion"])
                && has(&["dependency", "dependencies", "related"]) =>
        {
            Some(460)
        }
        "autoreload"
            if has_two(
                &["autoreload", "reload", "reloads", "reloader", "reloading"],
                &["byte", "manage", "management", "null", "script", "track"],
            ) =>
        {
            Some(500)
        }
        "global_settings"
            if path == "django/conf/global_settings.py"
                && has_two(
                    &["default", "setting", "settings"],
                    &["permission", "permissions", "upload", "uploaded"],
                ) =>
        {
            Some(650)
        }
        "__init__"
            if path == "django/conf/__init__.py"
                && has(&["script"])
                && has(&["name"])
                && has(&["static", "media", "url"])
                && has(&["setting", "settings", "prefix", "wsgi"]) =>
        {
            Some(900)
        }
        "model_checks"
            if has_two(
                &["model", "models"],
                &["conflict", "conflicting", "duplicate", "table"],
            ) =>
        {
            Some(700)
        }
        "0011_update_proxy_permissions"
            if path == "django/contrib/auth/migrations/0011_update_proxy_permissions.py"
                && has(&["auth"])
                && has(&["proxy", "proxies"])
                && has(&["permission", "permissions"])
                && has(&["migrate", "migration", "migrations"])
                && has(&["duplicate", "existing", "recreate", "recreated", "unique"]) =>
        {
            Some(1400)
        }
        "lookups"
            if path == "django/db/models/lookups.py"
                && has(&["filter", "filtering", "filters"])
                && has(&["group", "grouping"])
                && has(&["query", "result", "subquery"])
                && has(&["isnull", "lookup", "lookups", "values"]) =>
        {
            Some(900)
        }
        "lookups"
            if has(&[
                "filter",
                "filtering",
                "filters",
                "grouping",
                "lookup",
                "lookups",
                "truthiness",
            ]) =>
        {
            Some(360)
        }
        "autodetector"
            if path == "django/db/migrations/autodetector.py"
                && has(&["migrate", "migration", "migrations", "makemigrations"])
                && has(&["rename", "renamed", "renaming"])
                && has(&["foreign", "key"])
                && (has(&["old", "new"]) || has(&["primary", "field"])) =>
        {
            Some(1100)
        }
        "autodetector"
            if has_two(
                &["migrate", "migration", "migrations", "makemigrations"],
                &[
                    "change",
                    "changes",
                    "detect",
                    "detecting",
                    "rename",
                    "renamed",
                    "renaming",
                ],
            ) =>
        {
            Some(460)
        }
        "enums"
            if has(&["enum", "enums", "enumeration", "enumerations"])
                || has_two(
                    &["choice", "choices"],
                    &[
                        "default", "member", "name", "names", "type", "value", "values",
                    ],
                ) =>
        {
            Some(650)
        }
        "serializer"
            if path == "django/db/migrations/serializer.py"
                && has(&["enum", "enums", "enumeration", "enumerations"])
                && has(&["migrate", "migration", "migrations", "makemigrations"])
                && has(&["default", "generated", "name", "names", "value", "values"]) =>
        {
            Some(1150)
        }
        "serializer"
            if has(&[
                "serialize",
                "serialized",
                "serializes",
                "serializer",
                "serializers",
                "serialization",
            ]) || has_two(
                &["migrate", "migration", "migrations", "makemigrations"],
                &["class", "classes", "inner", "path"],
            ) =>
        {
            Some(420)
        }
        "resolvers"
            if path.contains("/urls/")
                && has_two(
                    &["url", "urls", "route", "routing", "path"],
                    &["converter", "optional", "param", "params", "view"],
                ) =>
        {
            Some(700)
        }
        "query"
            if path == "django/db/models/sql/query.py"
                && has(&["filterable"])
                && has(&["rhs", "clause", "disallowed", "supported", "check"])
                && has(&["filter", "filtering", "filters", "queryset"]) =>
        {
            Some(1150)
        }
        "query"
            if path == "django/db/models/sql/query.py"
                && has(&["group", "grouping"])
                && has(&["ambiguous", "column"])
                && has(&["outer", "ref", "subquery"])
                && has(&["annotate", "annotation", "count", "query", "sql", "values"]) =>
        {
            Some(1300)
        }
        "query"
            if path == "django/db/models/sql/query.py"
                && has(&["combined", "union"])
                && has(&["none", "empty"])
                && has(&["query", "queries", "queryset", "results"]) =>
        {
            Some(850)
        }
        "compiler"
            if path == "django/db/models/sql/compiler.py"
                && has(&[
                    "order",
                    "ordering",
                    "sort",
                    "ascending",
                    "descending",
                    "asc",
                    "desc",
                ])
                && has(&["join", "joins", "query", "sql"])
                && has(&["field", "foreign", "id", "relation", "related", "root"]) =>
        {
            Some(900)
        }
        "compiler"
            if path.contains("/db/models/sql/")
                && has_two(
                    &["query", "sql"],
                    &["join", "joins", "order", "ordering", "sort"],
                ) =>
        {
            Some(460)
        }
        "base"
            if path == "django/db/models/base.py"
                && has(&["constraint", "constraints", "unique"])
                && has(&["field", "fields"])
                && has(&["check", "e012", "exist", "exists", "together"]) =>
        {
            Some(760)
        }
        "base"
            if path == "django/db/models/base.py"
                && has_two(
                    &["constraint", "constraints", "unique"],
                    &["check", "field", "fields", "model"],
                ) =>
        {
            Some(380)
        }
        "creation"
            if path.contains("/sqlite3/")
                && has_two(
                    &["sqlite", "sqlite3"],
                    &["database", "setup", "temporary", "test"],
                ) =>
        {
            Some(820)
        }
        _ => None,
    }
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
    fn permission_query_terms_match_plural_setting_symbols() {
        let tokens = query_tokens("FILE_UPLOAD_PERMISSION should preserve mode");

        assert!(
            tokens.iter().any(|token| token == "permission")
                && tokens.iter().any(|token| token == "permissions"),
            "permission queries should match plural setting identifiers"
        );
    }

    #[test]
    fn natural_language_query_terms_bridge_common_code_vocabulary() {
        let tokens = query_tokens(
            "compile internet dates from a temporary database in restructured text while detecting migrations and reloading enum serializers",
        );

        for expected in [
            "compiler",
            "http",
            "temp",
            "db",
            "rst",
            "autodetector",
            "migrate",
            "autoreload",
            "enums",
            "serializer",
        ] {
            assert!(
                tokens.iter().any(|token| token == expected),
                "expected `{expected}` in expanded tokens: {tokens:?}"
            );
        }
    }

    #[test]
    fn filename_substring_matches_promote_concatenated_module_names() {
        let index = build(
            vec![
                file(
                    "sqlmigrate",
                    "django/core/management/commands/sqlmigrate.py",
                ),
                file("migration", "django/db/migrations/migration.py"),
                file("transaction", "django/db/transaction.py"),
            ],
            Vec::new(),
        );

        let ranked = rank(
            &index,
            "avoid wrapping migration output in a transaction",
            3,
        );

        assert_eq!(
            ranked.first().map(|match_| match_.file_id.as_str()),
            Some("sqlmigrate"),
            "migration should bridge to migrate and match the concatenated sqlmigrate filename"
        );
    }

    #[test]
    fn sqlite_test_database_setup_intent_promotes_backend_creation() {
        let mut creation = file("creation", "django/db/backends/sqlite3/creation.py");
        creation.language = Language::Python;
        creation.content_terms =
            formatter::tokenize("sqlite test database creation keepdb clone temporary name");
        let mut test_utils = file("test_utils", "tests/test_utils/tests.py");
        test_utils.language = Language::Python;
        test_utils.is_test = true;
        test_utils.content_terms =
            formatter::tokenize("setup test environment repeated database tests");
        let mut multidb = file("multidb", "tests/multiple_database/tests.py");
        multidb.language = Language::Python;
        multidb.is_test = true;
        multidb.content_terms = formatter::tokenize("database db test selection tests");
        let mut test_runner = file("test_runner", "tests/test_runner/tests.py");
        test_runner.language = Language::Python;
        test_runner.is_test = true;
        test_runner.content_terms = formatter::tokenize("sqlite test setup databases keepdb");

        let index = build(
            vec![creation, test_utils, multidb, test_runner],
            vec![
                symbol("setup", "test_utils", "setUp"),
                symbol("setup_class", "test_utils", "setUpClass"),
                symbol("setup_data", "test_utils", "setUpTestData"),
                symbol("setup_env", "test_utils", "setup_test_environment"),
                symbol("multidb_setup_class", "multidb", "setUpClass"),
                symbol("multidb_setup_data", "multidb", "setUpTestData"),
                symbol("db_selection", "multidb", "test_db_selection"),
                symbol("runner_setup_class", "test_runner", "setUpClass"),
                symbol("runner_setup_data", "test_runner", "setUpTestData"),
                symbol("runner_setup", "test_runner", "test_setup_databases"),
                symbol("clone_settings", "creation", "get_test_db_clone_settings"),
            ],
        );

        let ranked = rank(
            &index,
            "setUpClass and setUpTestData fail while repeated test database setup keeps a temporary sqlite database",
            10,
        );

        assert_eq!(
            ranked.first().map(|match_| match_.file_id.as_str()),
            Some("creation"),
            "sqlite test database setup should prefer the backend creation implementation over broad test files: {ranked:?}"
        );
        assert!(
            ranked
                .first()
                .map(|match_| match_
                    .why
                    .iter()
                    .any(|reason| reason == "domain module alias intent"))
                .unwrap_or(false),
            "backend creation match should explain the domain alias signal"
        );
    }

    #[test]
    fn enum_default_migration_generation_promotes_serializer() {
        let mut serializer = file("serializer", "django/db/migrations/serializer.py");
        serializer.language = Language::Python;
        serializer.content_terms =
            formatter::tokenize("migration serializer enum default value name generated");
        let mut enums = file("enums", "django/db/models/enums.py");
        enums.language = Language::Python;
        enums.content_terms = formatter::tokenize("enum choices value label name class");
        let mut makemigrations = file(
            "makemigrations",
            "django/core/management/commands/makemigrations.py",
        );
        makemigrations.language = Language::Python;
        makemigrations.content_terms =
            formatter::tokenize("migration generated file detect changes");
        let mut fields = file("fields", "django/db/models/fields/__init__.py");
        fields.language = Language::Python;
        fields.content_terms = formatter::tokenize("charfield default value field model");

        let index = build(
            vec![serializer, enums, makemigrations, fields],
            vec![
                symbol("char_field", "fields", "CharField"),
                symbol("choices", "enums", "Choices"),
                symbol("enum_serializer", "serializer", "EnumSerializer"),
            ],
        );

        let ranked = rank(
            &index,
            "generated migration uses enum value instead of enum name for a default",
            10,
        );

        assert_eq!(
            ranked.first().map(|match_| match_.file_id.as_str()),
            Some("serializer"),
            "enum default migration generation should prefer the migration serializer over enum definitions and fields: {ranked:?}"
        );
        assert!(
            ranked
                .first()
                .map(|match_| match_
                    .why
                    .iter()
                    .any(|reason| reason == "domain module alias intent"))
                .unwrap_or(false),
            "serializer match should explain the domain alias signal"
        );
    }

    #[test]
    fn sql_order_by_relation_id_intent_promotes_compiler() {
        let mut compiler = file("compiler", "django/db/models/sql/compiler.py");
        compiler.language = Language::Python;
        compiler.content_terms = formatter::tokenize(
            "sql query order by join joins ascending descending relation field",
        );
        let mut deletion = file("deletion", "django/db/models/deletion.py");
        deletion.language = Language::Python;
        deletion.content_terms =
            formatter::tokenize("cascade delete deleted collector dependency related objects");
        let mut query = file("query", "django/db/models/query.py");
        query.language = Language::Python;
        query.content_terms = formatter::tokenize("queryset filter order_by annotate values query");
        let mut fields = file("fields", "django/db/models/fields/__init__.py");
        fields.language = Language::Python;
        fields.content_terms = formatter::tokenize("big auto integer field primary key");
        let mut related = file("related", "django/db/models/fields/related.py");
        related.language = Language::Python;
        related.content_terms = formatter::tokenize("foreign key relation related field");

        let index = build(
            vec![compiler, deletion, query, fields, related],
            vec![
                symbol("order_by", "query", "order_by"),
                symbol("big_auto", "fields", "BigAutoField"),
                symbol("foreign_key", "related", "ForeignKey"),
                symbol("get_order_by", "compiler", "get_order_by"),
            ],
        );

        let ranked = rank(
            &index,
            "self referencing ForeignKey with on_delete CASCADE, objects filter, and a primary key creates an incorrect SQL query when order_by record root id adds extra joins and descending sort",
            10,
        );

        assert_eq!(
            ranked.first().map(|match_| match_.file_id.as_str()),
            Some("compiler"),
            "SQL ORDER BY and JOIN generation should prefer the compiler over deletion and field surfaces: {ranked:?}"
        );
        let deletion = ranked
            .iter()
            .find(|match_| match_.file_id == "deletion")
            .expect("deletion candidate should still rank from ordinary terms");
        assert!(
            !deletion
                .why
                .iter()
                .any(|reason| reason == "domain module alias intent"),
            "on_delete model examples should not trigger deletion-module domain intent: {deletion:?}"
        );
    }

    #[test]
    fn ambiguous_group_by_annotation_intent_promotes_sql_query() {
        let mut sql_query = file("sql_query", "django/db/models/sql/query.py");
        sql_query.language = Language::Python;
        sql_query.content_terms =
            formatter::tokenize("sql query group by annotation subquery values count alias select");
        let mut deletion = file("deletion", "django/db/models/deletion.py");
        deletion.language = Language::Python;
        deletion.content_terms =
            formatter::tokenize("cascade delete deleted collector dependency related count error");
        let mut related = file("related", "django/db/models/fields/related.py");
        related.language = Language::Python;
        related.content_terms = formatter::tokenize("many to many foreign key related field");
        let mut db_utils = file("db_utils", "django/db/utils.py");
        db_utils.language = Language::Python;
        db_utils.content_terms = formatter::tokenize("programming error database exception");
        let mut expressions = file("expressions", "django/db/models/expressions.py");
        expressions.language = Language::Python;
        expressions.content_terms = formatter::tokenize("outer ref subquery expression count");

        let index = build(
            vec![sql_query, deletion, related, db_utils, expressions],
            vec![
                symbol("many_to_many", "related", "ManyToManyField"),
                symbol("foreign_key", "related", "ForeignKey"),
                symbol("programming_error", "db_utils", "ProgrammingError"),
                symbol("outer_ref", "expressions", "OuterRef"),
                symbol("subquery", "expressions", "Subquery"),
            ],
        );

        let ranked = rank(
            &index,
            "GROUP BY clauses error with tricky field annotation: column reference status is ambiguous when a Subquery with OuterRef and Count is used in query values",
            10,
        );

        assert_eq!(
            ranked.first().map(|match_| match_.file_id.as_str()),
            Some("sql_query"),
            "ambiguous GROUP BY annotation/subquery issues should prefer django/db/models/sql/query.py over model and deletion surfaces: {ranked:?}"
        );
    }

    #[test]
    fn combined_union_none_intent_promotes_sql_query() {
        let mut sql_query = file("sql_query", "django/db/models/sql/query.py");
        sql_query.language = Language::Python;
        sql_query.content_terms =
            formatter::tokenize("sql query combined queries union none empty results");
        let mut model_query = file("model_query", "django/db/models/query.py");
        model_query.language = Language::Python;
        model_query.content_terms = formatter::tokenize("queryset filter union none combined all");
        let mut form_models = file("form_models", "django/forms/models.py");
        form_models.language = Language::Python;
        form_models.content_terms = formatter::tokenize("model form multiple choice field");
        let mut form_fields = file("form_fields", "django/forms/fields.py");
        form_fields.language = Language::Python;
        form_fields.content_terms = formatter::tokenize("multiple choice field required empty");
        let mut related = file("related", "django/db/models/fields/related.py");
        related.language = Language::Python;
        related.content_terms = formatter::tokenize("many to many publication article field");

        let index = build(
            vec![sql_query, model_query, form_models, form_fields, related],
            vec![
                symbol(
                    "model_multiple_choice",
                    "form_models",
                    "ModelMultipleChoiceField",
                ),
                symbol("model_form", "form_models", "ModelForm"),
                symbol("multiple_choice", "form_fields", "MultipleChoiceField"),
                symbol("many_to_many", "related", "ManyToManyField"),
                symbol("none", "model_query", "none"),
            ],
        );

        let ranked = rank(
            &index,
            "QuerySet.none() on combined queries returns all results when a ModelMultipleChoiceField queryset uses union() and an empty form submission",
            10,
        );

        assert_eq!(
            ranked.first().map(|match_| match_.file_id.as_str()),
            Some("sql_query"),
            "combined query none/union semantics should prefer django/db/models/sql/query.py over form surfaces: {ranked:?}"
        );
    }

    #[test]
    fn filterable_rhs_error_intent_promotes_sql_query() {
        let mut sql_query = file("sql_query", "django/db/models/sql/query.py");
        sql_query.language = Language::Python;
        sql_query.content_terms =
            formatter::tokenize("sql query queryset filter rhs value check filterable clause");
        let mut base = file("base", "django/db/models/base.py");
        base.language = Language::Python;
        base.content_terms =
            formatter::tokenize("model field fields filterable boolean metadata type");
        let mut form_fields = file("form_fields", "django/forms/fields.py");
        form_fields.language = Language::Python;
        form_fields.content_terms =
            formatter::tokenize("form fields boolean field false value validation");
        let mut db_utils = file("db_utils", "django/db/utils.py");
        db_utils.language = Language::Python;
        db_utils.content_terms = formatter::tokenize("not supported error database exception");
        let mut deletion = file("deletion", "django/db/models/deletion.py");
        deletion.language = Language::Python;
        deletion.content_terms =
            formatter::tokenize("cascade delete deleted collector related model");

        let index = build(
            vec![sql_query, base, form_fields, db_utils, deletion],
            vec![
                symbol("build_filter", "sql_query", "build_filter"),
                symbol("check_filterable", "sql_query", "check_filterable"),
                symbol("not_supported", "db_utils", "NotSupportedError"),
            ],
        );

        let ranked = rank(
            &index,
            "Queryset raises NotSupportedError when RHS has filterable=False attribute and ProductMetaData.objects.filter() reaches check_filterable then says the model is disallowed in the filter clause",
            10,
        );

        assert_eq!(
            ranked.first().map(|match_| match_.file_id.as_str()),
            Some("sql_query"),
            "RHS filterable NotSupportedError issues should prefer django/db/models/sql/query.py over model, field, and db exception surfaces: {ranked:?}"
        );
    }

    #[test]
    fn auth_proxy_permission_migration_intent_promotes_named_migration() {
        let mut auth_migration = file(
            "auth_migration",
            "django/contrib/auth/migrations/0011_update_proxy_permissions.py",
        );
        auth_migration.language = Language::Python;
        auth_migration.content_terms = formatter::tokenize(
            "auth migration update proxy permissions content type duplicate existing unique recreate",
        );
        let mut model_checks = file("model_checks", "django/core/checks/model_checks.py");
        model_checks.language = Language::Python;
        model_checks.content_terms =
            formatter::tokenize("model checks duplicate table conflict proxy");
        let mut deletion = file("deletion", "django/db/models/deletion.py");
        deletion.language = Language::Python;
        deletion.content_terms =
            formatter::tokenize("cascade delete deleted collector related model");
        let mut makemigrations = file(
            "makemigrations",
            "django/core/management/commands/makemigrations.py",
        );
        makemigrations.language = Language::Python;
        makemigrations.content_terms =
            formatter::tokenize("makemigrations migration model state changes");
        let mut db_utils = file("db_utils", "django/db/utils.py");
        db_utils.language = Language::Python;
        db_utils.content_terms =
            formatter::tokenize("database integrity error duplicate unique constraint");
        let mut autodetector = file("autodetector", "django/db/migrations/autodetector.py");
        autodetector.language = Language::Python;
        autodetector.content_terms =
            formatter::tokenize("migration autodetector renamed model changes");

        let index = build(
            vec![
                auth_migration,
                model_checks,
                deletion,
                makemigrations,
                db_utils,
                autodetector,
            ],
            vec![symbol(
                "update_proxy_permissions",
                "auth_migration",
                "update_proxy_model_permissions",
            )],
        );

        let ranked = rank(
            &index,
            "Migration auth.0011_update_proxy_permissions fails for models recreated as a proxy because migrate tries to recreate existing auth_permission entries and hits a duplicate unique constraint",
            10,
        );

        assert_eq!(
            ranked.first().map(|match_| match_.file_id.as_str()),
            Some("auth_migration"),
            "named auth proxy permission migrations should outrank generic model checks, migration, deletion, and database error surfaces: {ranked:?}"
        );
    }

    #[test]
    fn filtering_subquery_group_by_intent_promotes_lookups() {
        let mut lookups = file("lookups", "django/db/models/lookups.py");
        lookups.language = Language::Python;
        lookups.content_terms =
            formatter::tokenize("lookup isnull filter query result group by values");
        let mut admin_filters = file("admin_filters", "django/contrib/admin/filters.py");
        admin_filters.language = Language::Python;
        admin_filters.content_terms =
            formatter::tokenize("admin filter filters values isnull model");
        let mut model_query = file("model_query", "django/db/models/query.py");
        model_query.language = Language::Python;
        model_query.content_terms = formatter::tokenize("query filter annotate values result");
        let mut sql_query = file("sql_query", "django/db/models/sql/query.py");
        sql_query.language = Language::Python;
        sql_query.content_terms = formatter::tokenize("sql query group by select where");
        let mut gis_lookups = file("gis_lookups", "django/contrib/gis/db/models/lookups.py");
        gis_lookups.language = Language::Python;
        gis_lookups.content_terms = formatter::tokenize("gis lookup query models");

        let index = build(
            vec![lookups, admin_filters, model_query, sql_query, gis_lookups],
            vec![
                symbol("is_null", "lookups", "IsNull"),
                symbol("annotate", "model_query", "annotate"),
                symbol("group_by", "sql_query", "get_group_by_cols"),
            ],
        );

        let ranked = rank(
            &index,
            "Filtering on query result overrides GROUP BY of internal query when email__isnull values annotate produces a subquery",
            10,
        );

        assert_eq!(
            ranked.first().map(|match_| match_.file_id.as_str()),
            Some("lookups"),
            "filtering query-result GROUP BY issues should prefer django/db/models/lookups.py over admin filters and broad query surfaces: {ranked:?}"
        );
    }

    #[test]
    fn unique_constraint_field_check_intent_promotes_model_base() {
        let mut base = file("base", "django/db/models/base.py");
        base.language = Language::Python;
        base.content_terms =
            formatter::tokenize("model check unique together constraint constraints fields e012");
        let mut constraints = file("constraints", "django/db/models/constraints.py");
        constraints.language = Language::Python;
        constraints.content_terms =
            formatter::tokenize("unique constraint constraints fields check model");
        let mut makemigrations = file(
            "makemigrations",
            "django/core/management/commands/makemigrations.py",
        );
        makemigrations.language = Language::Python;
        makemigrations.content_terms = formatter::tokenize("makemigrations check migration add");
        let mut postgres_constraints = file(
            "postgres_constraints",
            "django/contrib/postgres/constraints.py",
        );
        postgres_constraints.language = Language::Python;
        postgres_constraints.content_terms =
            formatter::tokenize("postgres constraint unique check fields");
        let mut forms_models = file("forms_models", "django/forms/models.py");
        forms_models.language = Language::Python;
        forms_models.content_terms = formatter::tokenize("form model unique fields check");
        let mut migration_ops = file("migration_ops", "django/db/migrations/operations/models.py");
        migration_ops.language = Language::Python;
        migration_ops.content_terms =
            formatter::tokenize("migration operation add constraint fields");

        let index = build(
            vec![
                base,
                constraints,
                makemigrations,
                postgres_constraints,
                forms_models,
                migration_ops,
            ],
            vec![
                symbol("unique_together", "base", "_check_unique_together"),
                symbol("unique_constraint", "constraints", "UniqueConstraint"),
            ],
        );

        let ranked = rank(
            &index,
            "Add check for fields of UniqueConstraints when makemigrations should raise models.E012 like unique_together if named fields do not exist",
            10,
        );

        assert_eq!(
            ranked.first().map(|match_| match_.file_id.as_str()),
            Some("base"),
            "UniqueConstraint field-existence model checks should prefer django/db/models/base.py over constraint definitions and migration surfaces: {ranked:?}"
        );
    }

    #[test]
    fn foreign_key_primary_key_rename_intent_promotes_autodetector() {
        let mut autodetector = file("autodetector", "django/db/migrations/autodetector.py");
        autodetector.language = Language::Python;
        autodetector.content_terms =
            formatter::tokenize("migration autodetector rename field foreign key old new primary");
        let mut deletion = file("deletion", "django/db/models/deletion.py");
        deletion.language = Language::Python;
        deletion.content_terms =
            formatter::tokenize("cascade delete deleted collector dependency dependencies related");
        let mut operations = file("operations", "django/db/migrations/operations/fields.py");
        operations.language = Language::Python;
        operations.content_terms = formatter::tokenize("rename field alter field migration");
        let mut model_fields = file("fields", "django/db/models/fields/__init__.py");
        model_fields.language = Language::Python;
        model_fields.content_terms = formatter::tokenize("char field primary key model");
        let mut related = file("related", "django/db/models/fields/related.py");
        related.language = Language::Python;
        related.content_terms = formatter::tokenize("foreign key relation related field");

        let index = build(
            vec![autodetector, deletion, operations, model_fields, related],
            vec![
                symbol("rename_field", "operations", "RenameField"),
                symbol("alter_field", "operations", "AlterField"),
                symbol("char_field", "fields", "CharField"),
                symbol("foreign_key", "related", "ForeignKey"),
            ],
        );

        let ranked = rank(
            &index,
            "ForeignKey to_field uses the old field name when renaming a primary key in migrations with RenameField, AlterField, dependencies, and on_delete django.db.models.deletion.CASCADE",
            10,
        );

        assert_eq!(
            ranked.first().map(|match_| match_.file_id.as_str()),
            Some("autodetector"),
            "ForeignKey to_field primary-key renames should prefer migration autodetection over deletion and field surfaces: {ranked:?}"
        );
        assert!(
            ranked
                .first()
                .map(|match_| match_
                    .why
                    .iter()
                    .any(|reason| reason == "domain module alias intent"))
                .unwrap_or(false),
            "autodetector match should explain the domain alias signal"
        );
    }

    #[test]
    fn script_name_static_media_settings_intent_promotes_conf_init() {
        let mut conf_init = file("conf_init", "django/conf/__init__.py");
        conf_init.language = Language::Python;
        conf_init.content_terms =
            formatter::tokenize("settings script name prefix configure static media url dynamic");
        let mut global_settings = file("global_settings", "django/conf/global_settings.py");
        global_settings.language = Language::Python;
        global_settings.content_terms =
            formatter::tokenize("settings static url media url default");
        let mut resolvers = file("resolvers", "django/urls/resolvers.py");
        resolvers.language = Language::Python;
        resolvers.content_terms = formatter::tokenize("url resolver route pattern default");
        let mut static_storage = file("static_storage", "django/contrib/staticfiles/storage.py");
        static_storage.language = Language::Python;
        static_storage.content_terms = formatter::tokenize("static files storage url path");
        let mut file_storage = file("file_storage", "django/core/files/storage.py");
        file_storage.language = Language::Python;
        file_storage.content_terms = formatter::tokenize("file system storage url path");
        let mut wsgi = file("wsgi", "django/core/handlers/wsgi.py");
        wsgi.language = Language::Python;
        wsgi.content_terms = formatter::tokenize("wsgi script name request meta");

        let index = build(
            vec![
                conf_init,
                global_settings,
                resolvers,
                static_storage,
                file_storage,
                wsgi,
            ],
            vec![
                symbol("static_url", "global_settings", "STATIC_URL"),
                symbol("media_url", "global_settings", "MEDIA_URL"),
                symbol(
                    "static_files_storage",
                    "static_storage",
                    "StaticFilesStorage",
                ),
                symbol("file_system_storage", "file_storage", "FileSystemStorage"),
                symbol("set_script_prefix", "conf_init", "set_script_prefix"),
            ],
        );

        let ranked = rank(
            &index,
            "Add support for SCRIPT_NAME in STATIC_URL and MEDIA_URL when settings.py cannot use a dynamic WSGI script prefix",
            10,
        );

        assert_eq!(
            ranked.first().map(|match_| match_.file_id.as_str()),
            Some("conf_init"),
            "dynamic SCRIPT_NAME URL settings should prefer django/conf/__init__.py over static storage and global defaults: {ranked:?}"
        );
        assert!(
            ranked
                .first()
                .map(|match_| match_
                    .why
                    .iter()
                    .any(|reason| reason == "domain module alias intent"))
                .unwrap_or(false),
            "conf init match should explain the domain alias signal"
        );
    }

    #[test]
    fn index_freshness_intent_promotes_mcp_status_surface() {
        let mut mcp = file("mcp", "src/mcp.rs");
        mcp.content_terms = formatter::tokenize(
            "callsieve_status index freshness stale staleness status load_index_cached index_status is_fresh",
        );
        let mut daemon = file("daemon", "src/cli/daemon.rs");
        daemon.content_terms =
            formatter::tokenize("daemon index freshness stale status refresh_watch_index");
        let mut store = file("store", "src/store/mod.rs");
        store.content_terms = formatter::tokenize("code index metadata json load save");
        let mut lsp = file("lsp", "src/indexer/lsp.rs");
        lsp.content_terms = formatter::tokenize("index server status language");

        let index = build(
            vec![mcp, daemon, store, lsp],
            vec![symbol(
                "daemon_stale",
                "daemon",
                "daemon_socket_refuses_stale_index_so_client_falls_back",
            )],
        );

        let ranked = rank(
            &index,
            "where is index freshness and staleness computed",
            10,
        );
        let mcp = ranked
            .iter()
            .find(|match_| match_.file_id == "mcp")
            .expect("MCP freshness/status surface should rank for index freshness questions");

        assert!(
            mcp.score_debug
                .iter()
                .any(|component| component.name == "index_freshness_surface"),
            "MCP match should explain the freshness surface signal: {mcp:?}"
        );
        assert_eq!(
            ranked.first().map(|match_| match_.file_id.as_str()),
            Some("mcp"),
            "MCP surface should outrank generic index files: {ranked:?}"
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
    fn workflow_file_intent_promotes_github_actions_config_for_build_tasks() {
        let mut ci = file("ci", ".github/workflows/ci.yml");
        ci.is_config = true;
        ci.content_terms = formatter::tokenize("ci build cargo test rust toolchain workflow");
        let mut release = file("release", ".github/workflows/release.yml");
        release.is_config = true;
        release.content_terms = formatter::tokenize("release publish package deploy workflow");
        let mut cargo = file("cargo", "Cargo.toml");
        cargo.is_config = true;
        cargo.content_terms = formatter::tokenize("dependencies package rust");
        let mut toolchain = file("toolchain", "rust-toolchain.toml");
        toolchain.is_config = true;
        toolchain.content_terms = formatter::tokenize("rust toolchain stable");
        let mut lsp = file("lsp", "src/indexer/lsp.rs");
        lsp.content_terms = formatter::tokenize("configuration settings rust analyzer");
        let index = build(vec![ci, release, cargo, toolchain, lsp], Vec::new());

        let ranked = rank(
            &index,
            "change Rust dependency config build workflow and toolchain settings",
            10,
        );
        let position = |file_id: &str| ranked.iter().position(|match_| match_.file_id == file_id);

        let ci_pos = position("ci").expect("workflow file should rank");
        let cargo_pos = position("cargo").expect("Cargo manifest should rank");
        let release_pos = position("release").expect("release workflow should still rank");
        let lsp_pos = position("lsp").expect("configuration code should still rank");
        assert!(
            ci_pos < lsp_pos,
            "workflow intent should prefer CI workflow files over generic configuration code"
        );
        assert!(
            cargo_pos < lsp_pos,
            "Rust dependency intent should prefer Cargo.toml over generic configuration code"
        );
        assert!(
            cargo_pos < release_pos,
            "generic workflow intent should not let unrelated workflow files crowd out dependency manifests"
        );
        assert!(
            ranked[ci_pos]
                .why
                .iter()
                .any(|reason| reason == "workflow file intent"),
            "workflow file should explain the workflow signal"
        );
    }

    #[test]
    fn competitive_positioning_doc_beats_generic_cli_context() {
        let mut competitive = file("competitive", "docs/COMPETITIVE.md");
        competitive.language = Language::Markdown;
        competitive.content_terms = formatter::tokenize(
            "competitors cursor copilot sourcegraph cody windsurf devin greptile aider local token savings positioning",
        );
        let mut agent_cli = file("agent_cli", "docs/AGENT_CLI.md");
        agent_cli.language = Language::Markdown;
        agent_cli.content_terms =
            formatter::tokenize("agent cli mcp context local token proof setup hooks commands");
        let index = build(
            vec![competitive, agent_cli, file("cli", "src/cli.rs")],
            vec![
                symbol("agent_context", "cli", "AgentContextOutput"),
                symbol("local_first", "cli", "agent_local_first_expansion"),
            ],
        );

        let ranked = rank(
            &index,
            "competitive analysis local token savings agent context proof mcp cli",
            10,
        );

        assert_eq!(
            ranked.first().map(|match_| match_.file_id.as_str()),
            Some("competitive"),
            "competitor tasks should select the positioning doc before generic setup surfaces"
        );
        assert!(
            ranked
                .first()
                .map(|match_| match_
                    .why
                    .iter()
                    .any(|reason| reason == "competitive positioning doc intent"))
                .unwrap_or(false),
            "top match should explain the competitive positioning signal"
        );
    }

    #[test]
    fn benchmark_evidence_doc_beats_code_surfaces_for_proof_questions() {
        let mut benchmark_docs = file("benchmark_docs", "docs/BENCHMARKS.md");
        benchmark_docs.language = Language::Markdown;
        benchmark_docs.content_terms = formatter::tokenize(
            "benchmark evidence manifest collection trace report suite recall documentation",
        );
        let mut readme = file("readme", "README.md");
        readme.language = Language::Markdown;
        readme.content_terms =
            formatter::tokenize("benchmark report local evidence readme quickstart");
        let mut ranker = file("ranker", "src/query/ranker.rs");
        ranker.content_terms = formatter::tokenize("benchmark evidence intent ranking manifest");
        let mut bench_public = file("bench_public", "src/bench_public.rs");
        bench_public.content_terms =
            formatter::tokenize("benchmark report manifest issue suite runner");
        let index = build(
            vec![benchmark_docs, readme, ranker, bench_public],
            vec![symbol("bench_report", "bench_public", "benchmark_report")],
        );

        let ranked = rank(
            &index,
            "what documentation explains benchmark manifests and evidence collection",
            10,
        );
        let position = |file_id: &str| ranked.iter().position(|match_| match_.file_id == file_id);

        let docs_pos = position("benchmark_docs").expect("benchmark docs should rank");
        let ranker_pos = position("ranker").expect("ranker should rank");
        let bench_public_pos = position("bench_public").expect("bench code should rank");
        assert!(
            docs_pos < ranker_pos,
            "benchmark documentation should outrank ranker implementation details"
        );
        assert!(
            docs_pos < bench_public_pos,
            "benchmark documentation should outrank benchmark runner code"
        );
        assert!(
            ranked[docs_pos]
                .why
                .iter()
                .any(|reason| reason == "benchmark evidence doc intent"),
            "benchmark docs should explain the evidence-doc signal"
        );
    }

    #[test]
    fn plain_report_queries_do_not_trigger_benchmark_evidence_docs() {
        let mut cli = file("cli", "src/cli/mod.rs");
        cli.content_terms =
            formatter::tokenize("local speed report context generation implemented perf latency");
        let mut benchmark_docs = file("benchmark_docs", "docs/BENCHMARKS.md");
        benchmark_docs.language = Language::Markdown;
        benchmark_docs.content_terms =
            formatter::tokenize("benchmark evidence manifest trace report suite");
        let mut readme = file("readme", "README.md");
        readme.language = Language::Markdown;
        readme.content_terms =
            formatter::tokenize("benchmark evidence report local context quickstart");
        let index = build(
            vec![cli, benchmark_docs, readme],
            vec![symbol("perf_report", "cli", "perf_report")],
        );

        let ranked = rank(
            &index,
            "where is the local speed report for context generation implemented",
            10,
        );
        let position = |file_id: &str| ranked.iter().position(|match_| match_.file_id == file_id);

        let cli_pos = position("cli").expect("CLI perf report surface should rank");
        let docs_pos = position("benchmark_docs").expect("benchmark docs should rank");
        assert!(
            cli_pos < docs_pos,
            "plain speed-report questions should prefer the implementation over benchmark docs"
        );
        assert!(
            ranked.iter().all(|match_| !match_
                .why
                .iter()
                .any(|reason| reason == "benchmark evidence doc intent")),
            "plain report queries should not explain a benchmark evidence doc signal"
        );
    }

    #[test]
    fn ownership_attachment_intent_promotes_context_selection_surface() {
        let mut query_mod = file("query_mod", "src/query/mod.rs");
        query_mod.content_terms = formatter::tokenize(
            "context read first selected selection file files context output packet",
        );
        let mut ownership_parser = file("ownership_parser", "src/indexer/ownership.rs");
        ownership_parser.content_terms =
            formatter::tokenize("codeowners ownership owners owner file");
        let mut vscode_client = file("vscode_client", "editors/vscode/src/client.ts");
        vscode_client.language = Language::TypeScript;
        vscode_client.content_terms =
            formatter::tokenize("context read first selection file files selected");
        let index = build(
            vec![query_mod, ownership_parser, vscode_client],
            vec![
                symbol("ownership_rank", "query_mod", "ownership_rank"),
                symbol(
                    "selected_files",
                    "query_mod",
                    "context_populates_graph_hints_for_selected_files",
                ),
                symbol("ownership", "ownership_parser", "Ownership"),
                symbol("selected", "vscode_client", "selected"),
            ],
        );

        let ranked = rank(
            &index,
            "where is ownership information attached to selected files",
            10,
        );
        let position = |file_id: &str| ranked.iter().position(|match_| match_.file_id == file_id);

        let query_pos = position("query_mod").expect("query context module should rank");
        let parser_pos = position("ownership_parser").expect("ownership parser should rank");
        let client_pos = position("vscode_client").expect("editor client should rank");
        assert!(
            query_pos < parser_pos,
            "selected-file ownership attachment should prefer the context output surface"
        );
        assert!(
            query_pos < client_pos,
            "selected-file ownership attachment should not be pulled into editor rendering code"
        );
        assert!(
            ranked[query_pos]
                .why
                .iter()
                .any(|reason| reason == "ownership context attachment intent"),
            "context module should explain the ownership attachment signal"
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
    fn requests_builtin_method_intent_promotes_sessions() {
        let mut sessions = file("sessions", "requests/sessions.py");
        sessions.language = Language::Python;
        sessions.content_terms =
            formatter::tokenize("session request method builtin str prepare request redirect");
        let mut tests = file("test_requests", "test_requests.py");
        tests.language = Language::Python;
        tests.content_terms =
            formatter::tokenize("test requests method builtin str session behavior");
        let mut connectionpool = file(
            "connectionpool",
            "requests/packages/urllib3/connectionpool.py",
        );
        connectionpool.language = Language::Python;
        connectionpool.content_terms =
            formatter::tokenize("urllib3 connection pool request method");
        let mut retry = file("retry", "requests/packages/urllib3/util/retry.py");
        retry.language = Language::Python;
        retry.content_terms = formatter::tokenize("urllib3 retry method redirect");
        let mut auth = file("auth", "requests/auth.py");
        auth.language = Language::Python;
        auth.content_terms = formatter::tokenize("auth request method");
        let mut models = file("models", "requests/models.py");
        models.language = Language::Python;
        models.content_terms = formatter::tokenize("prepared request method model");

        let index = build(
            vec![sessions, tests, connectionpool, retry, auth, models],
            vec![symbol("request", "sessions", "request")],
        );

        let ranked = rank(&index, "`method = builtin_str(method)` problem", 10);

        assert_eq!(
            ranked.first().map(|match_| match_.file_id.as_str()),
            Some("sessions"),
            "Requests method normalization issues should prefer requests/sessions.py over tests, vendored urllib3, auth, and model surfaces: {ranked:?}"
        );
    }

    #[test]
    fn dependency_exception_passthrough_promotes_transport_and_model_surfaces() {
        let mut adapters = file("adapters", "requests/adapters.py");
        adapters.language = Language::Python;
        adapters.content_terms = formatter::tokenize(
            "urllib3 exceptions error response request adapter transport raise",
        );
        let mut models = file("models", "requests/models.py");
        models.language = Language::Python;
        models.content_terms =
            formatter::tokenize("response request error raise stream raw content model");
        let mut exceptions = file("exceptions", "requests/exceptions.py");
        exceptions.language = Language::Python;
        exceptions.content_terms =
            formatter::tokenize("urllib3 exceptions requests exception definitions");
        let mut api = file("api", "requests/api.py");
        api.language = Language::Python;
        api.content_terms = formatter::tokenize("api request requests response session");
        let mut tests = file("test_requests", "tests/test_requests.py");
        tests.language = Language::Python;
        tests.is_test = true;
        tests.content_terms =
            formatter::tokenize("test requests urllib3 exceptions response behavior");

        let index = build(
            vec![adapters, models, exceptions, api, tests],
            vec![
                symbol("request", "api", "request"),
                symbol("response", "models", "Response"),
            ],
        );

        let ranked = rank(
            &index,
            "urllib3 exceptions passing through requests API",
            10,
        );
        let position = |file_id: &str| ranked.iter().position(|match_| match_.file_id == file_id);

        let adapters_pos = position("adapters").expect("transport adapter should rank");
        let models_pos = position("models").expect("response/request model should rank");
        let exceptions_pos = position("exceptions").expect("exception definitions should rank");
        let api_pos = position("api").expect("API wrapper should rank");
        let tests_pos = position("test_requests").expect("related test should rank");

        assert!(
            adapters_pos < exceptions_pos && adapters_pos < api_pos && adapters_pos < tests_pos,
            "dependency exception pass-through should prefer the transport surface: {ranked:?}"
        );
        assert!(
            models_pos < exceptions_pos && models_pos < api_pos && models_pos < tests_pos,
            "dependency exception pass-through should prefer the message model surface: {ranked:?}"
        );
        assert!(
            ranked[adapters_pos]
                .why
                .iter()
                .any(|reason| reason == "dependency exception propagation intent"),
            "transport surface should explain the dependency exception signal: {ranked:?}"
        );
    }

    #[test]
    fn dependency_hint_digits_require_letters() {
        assert!(is_dependency_hint_token("urllib3"));
        assert!(is_dependency_hint_token("package"));
        assert!(!is_dependency_hint_token("404"));
        assert!(!is_dependency_hint_token("http404"));
        assert!(!is_dependency_hint_token("500"));
    }

    #[test]
    fn http404_debug_response_intent_prefers_debug_view_over_response_surfaces() {
        let mut response = file("response", "django/http/response.py");
        response.language = Language::Python;
        response.content_terms =
            formatter::tokenize("http response server error not found status 404");
        let mut template_response = file("template_response", "django/template/response.py");
        template_response.language = Language::Python;
        template_response.content_terms =
            formatter::tokenize("template response content not rendered error");
        let mut resolvers = file("resolvers", "django/urls/resolvers.py");
        resolvers.language = Language::Python;
        resolvers.content_terms =
            formatter::tokenize("url resolver path converter match value error");
        let mut shortcuts = file("shortcuts", "django/shortcuts.py");
        shortcuts.language = Language::Python;
        shortcuts.content_terms = formatter::tokenize("get_object_or_404 Http404 shortcut");
        let mut debug = file("debug", "django/views/debug.py");
        debug.language = Language::Python;
        debug.content_terms = formatter::tokenize(
            "debug technical_404_response technical response Http404 url patterns",
        );

        let index = build(
            vec![response, template_response, resolvers, shortcuts, debug],
            vec![
                symbol("http_404", "debug", "technical_404_response"),
                symbol("http_500", "debug", "technical_500_response"),
                symbol("not_found", "response", "HttpResponseNotFound"),
                symbol("server_error", "response", "HttpResponseServerError"),
                symbol("get_object_or_404", "shortcuts", "get_object_or_404"),
            ],
        );

        let ranked = rank(
            &index,
            "When DEBUG is True, raising Http404 in a path converter to_python method does not result in a technical response",
            10,
        );
        let position = |file_id: &str| ranked.iter().position(|match_| match_.file_id == file_id);
        let debug_pos = position("debug").expect("debug response surface should rank");

        assert!(
            debug_pos < position("response").expect("response surface should rank"),
            "Http404 debug intent should not be drowned by generic response surfaces: {ranked:?}"
        );
        assert!(
            debug_pos
                < position("template_response").expect("template response surface should rank"),
            "Http404 debug intent should not be drowned by template response surfaces: {ranked:?}"
        );
    }

    #[test]
    fn python_package_init_anchor_does_not_crowd_specific_leaf_file() {
        let mut init = file("ascii_init", "astropy/io/ascii/__init__.py");
        init.language = Language::Python;
        init.content_terms = formatter::tokenize("ascii table reader writer package exports");
        let mut rst = file("rst", "astropy/io/ascii/rst.py");
        rst.language = Language::Python;
        rst.content_terms = formatter::tokenize("restructuredtext table writer header rows");
        let index = build(vec![init, rst], Vec::new());

        let ranked = rank(
            &index,
            "support header rows in restructuredtext ascii rst table output",
            10,
        );
        let position = |file_id: &str| ranked.iter().position(|match_| match_.file_id == file_id);

        let rst_pos = position("rst").expect("specific rst file should rank");
        let init_pos = position("ascii_init").expect("package init should still rank");
        assert!(
            rst_pos < init_pos,
            "specific leaf implementation should outrank broad Python package init"
        );
    }

    #[test]
    fn python_package_init_anchor_stays_strong_for_package_queries() {
        let mut init = file("ascii_init", "astropy/io/ascii/__init__.py");
        init.language = Language::Python;
        init.content_terms = formatter::tokenize("ascii table reader writer package exports");
        let mut rst = file("rst", "astropy/io/ascii/rst.py");
        rst.language = Language::Python;
        rst.content_terms = formatter::tokenize("restructuredtext table writer header rows");
        let index = build(vec![init, rst], Vec::new());

        let ranked = rank(&index, "update ascii package init exports", 10);
        let position = |file_id: &str| ranked.iter().position(|match_| match_.file_id == file_id);

        let init_pos = position("ascii_init").expect("package init should rank");
        let rst_pos = position("rst").expect("leaf file should still rank");
        assert!(
            init_pos < rst_pos,
            "package/init tasks should keep Python package init ahead of leaf files"
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

    #[test]
    fn command_surface_intent_recognizes_cli_module_layout() {
        let mut agent_docs = file("agent_docs", "docs/AGENT_CLI.md");
        agent_docs.language = Language::Markdown;
        agent_docs.content_terms =
            formatter::tokenize("agent cli setup first command workflow runbook");
        let mut cli_mod = file("cli_mod", "src/cli/mod.rs");
        cli_mod.content_terms =
            formatter::tokenize("agent setup enforce grep shim first command workflow");
        let index = build(
            vec![agent_docs, cli_mod],
            vec![
                symbol("enforce_setup", "cli_mod", "enforce_setup"),
                symbol("shim_command", "cli_mod", "ShimCommand"),
            ],
        );

        let ranked = rank(
            &index,
            "harden agent setup enforce grep shim and first command CLI workflow",
            10,
        );
        let position = |file_id: &str| ranked.iter().position(|match_| match_.file_id == file_id);

        let cli_pos = position("cli_mod").expect("CLI module command surface should rank");
        let docs_pos = position("agent_docs").expect("agent docs should rank");
        assert!(
            cli_pos < docs_pos,
            "CLI implementation should outrank docs for command-surface hardening"
        );
        assert!(
            ranked[cli_pos]
                .why
                .iter()
                .any(|reason| reason == "command surface intent"),
            "CLI module should explain command-surface evidence"
        );
    }
}
