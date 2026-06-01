pub mod formatter;
pub mod ranker;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::{
    indexer::language::Language,
    store::{CodeIndex, FileRecord, ReferenceRecord, SymbolRecord},
};

#[derive(Debug, Serialize)]
pub struct SymbolsOutput {
    root: String,
    symbols: Vec<SymbolListItem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SymbolListItem {
    file: String,
    name: String,
    kind: String,
    lines: [usize; 2],
    visibility: String,
}

#[derive(Debug, Serialize)]
pub struct SymbolOutput {
    query: String,
    matches: Vec<SymbolDetail>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SymbolDetail {
    file: String,
    name: String,
    kind: String,
    language: Language,
    lines: [usize; 2],
    visibility: String,
    signature: String,
    imports: Vec<String>,
    referenced_by: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    calls: Vec<ReferenceEdge>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    references: Vec<ReferenceEdge>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    called_by: Vec<ReferenceEdge>,
}

#[derive(Debug, Serialize)]
pub struct QueryOutput {
    query: String,
    root: String,
    matches: Vec<QueryMatch>,
    stats: QueryStats,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct QueryMatch {
    rank: usize,
    score: i32,
    file: String,
    language: Language,
    #[serde(skip_serializing_if = "Option::is_none")]
    symbol: Option<QuerySymbol>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snippet: Option<Snippet>,
    why: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    related_tests: Vec<RelatedTest>,
}

#[derive(Debug, Serialize)]
struct QuerySymbol {
    name: String,
    kind: String,
    lines: [usize; 2],
    visibility: String,
    signature: String,
}

#[derive(Debug, Serialize)]
struct Snippet {
    lines: [usize; 2],
    text: String,
}

#[derive(Debug, Serialize)]
struct RelatedTest {
    file: String,
    symbols: Vec<String>,
}

#[derive(Debug, Serialize)]
struct QueryStats {
    searched_files: usize,
    matched_files: usize,
    matched_symbols: usize,
}

#[derive(Debug, Serialize)]
pub struct ContextOutput {
    task: String,
    root: String,
    read_first: Vec<ContextFile>,
    stats: ContextStats,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ContextFile {
    rank: usize,
    score: i32,
    file: String,
    language: Language,
    symbols: Vec<QuerySymbol>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    snippets: Vec<Snippet>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    imports: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    referenced_by: Vec<String>,
    blast_radius: BlastRadius,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    calls: Vec<ReferenceEdge>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    called_by: Vec<ReferenceEdge>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    related_tests: Vec<RelatedTest>,
    why: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ReferenceEdge {
    file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    symbol: Option<String>,
    target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_file: Option<String>,
    kind: String,
    line: usize,
}

#[derive(Debug, Serialize)]
struct BlastRadius {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    imports: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    referenced_by: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tests: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    calls: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    called_by: Vec<String>,
    risk: String,
}

#[derive(Debug, Serialize)]
struct ContextStats {
    candidate_matches: usize,
    selected_files: usize,
    selected_symbols: usize,
    related_tests: usize,
}

#[derive(Debug, Serialize)]
pub struct BenchmarkOutput {
    task: String,
    root: String,
    estimator: String,
    baseline: BaselineBenchmark,
    callsieve: CallsieveBenchmark,
    savings: SavingsBenchmark,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct BaselineBenchmark {
    strategy: String,
    grep_terms: Vec<String>,
    grep_commands: usize,
    matched_files: usize,
    matched_lines: usize,
    estimated_search_result_tokens: usize,
    estimated_read_tokens: usize,
    estimated_total_tokens: usize,
    matched_files_sample: Vec<String>,
}

#[derive(Debug, Serialize)]
struct CallsieveBenchmark {
    strategy: String,
    selected_files: usize,
    selected_symbols: usize,
    related_tests: usize,
    packet_bytes: usize,
    estimated_packet_tokens: usize,
    top_files: Vec<BenchmarkContextFile>,
}

#[derive(Debug, Serialize)]
struct BenchmarkContextFile {
    file: String,
    score: i32,
    risk: String,
    why: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SavingsBenchmark {
    avoided_grep_commands: usize,
    avoided_file_reads: usize,
    estimated_token_savings: isize,
    estimated_token_reduction_percent: f64,
    notes: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct BenchmarkSuiteInput {
    tasks: Vec<BenchmarkSuiteTaskInput>,
}

#[derive(Debug, Deserialize)]
struct BenchmarkSuiteTaskInput {
    id: Option<String>,
    task: String,
    #[serde(default)]
    expected_files: Vec<String>,
    observed: Option<ObservedSessionComparison>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ObservedSessionComparison {
    baseline: ObservedSessionMetrics,
    callsieve: ObservedSessionMetrics,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ObservedSessionMetrics {
    grep_commands: usize,
    file_reads: usize,
    tokens: usize,
}

#[derive(Debug, Serialize)]
pub struct BenchmarkSuiteOutput {
    root: String,
    task_count: usize,
    tasks: Vec<BenchmarkSuiteTaskOutput>,
    summary: BenchmarkSuiteSummary,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct BenchmarkSuiteTaskOutput {
    id: Option<String>,
    task: String,
    expected_files: Vec<String>,
    selected_files: Vec<String>,
    expected_files_found: Vec<String>,
    expected_files_missing: Vec<String>,
    expected_file_recall: f64,
    benchmark: BenchmarkOutput,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_session: Option<ObservedSessionOutput>,
}

#[derive(Debug, Serialize)]
struct BenchmarkSuiteSummary {
    task_count: usize,
    tasks_with_all_expected_files: usize,
    expected_files: usize,
    expected_files_found: usize,
    expected_file_recall: f64,
    total_estimated_token_savings: isize,
    average_estimated_token_reduction_percent: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_session: Option<ObservedSessionSummary>,
}

#[derive(Debug, Serialize)]
struct ObservedSessionOutput {
    baseline: ObservedSessionMetrics,
    callsieve: ObservedSessionMetrics,
    savings: ObservedSessionSavings,
}

#[derive(Debug, Serialize)]
struct ObservedSessionSavings {
    avoided_grep_commands: usize,
    avoided_file_reads: usize,
    token_savings: isize,
    token_reduction_percent: f64,
}

#[derive(Debug, Serialize)]
struct ObservedSessionSummary {
    sessions: usize,
    baseline_tokens: usize,
    callsieve_tokens: usize,
    token_savings: isize,
    token_reduction_percent: f64,
    avoided_grep_commands: usize,
    avoided_file_reads: usize,
}

#[derive(Debug)]
struct ContextCandidate {
    file_id: String,
    best_score: i32,
    graph_score: i32,
    first_rank: usize,
    symbol_ids: Vec<String>,
    why: Vec<String>,
    seen_why: BTreeSet<String>,
}

impl ContextCandidate {
    fn new(file_id: String, score: i32, first_rank: usize) -> Self {
        Self {
            file_id,
            best_score: score,
            graph_score: 0,
            first_rank,
            symbol_ids: Vec::new(),
            why: Vec::new(),
            seen_why: BTreeSet::new(),
        }
    }

    fn score(&self) -> i32 {
        let bonus_count = self
            .symbol_ids
            .len()
            .saturating_sub(1)
            .min((i32::MAX / 5) as usize) as i32;
        self.best_score + self.graph_score + (bonus_count * 5)
    }

    fn add_match(&mut self, score: i32, symbol_id: Option<&str>, why: &[String]) {
        self.best_score = self.best_score.max(score);

        if let Some(symbol_id) = symbol_id
            && !self.symbol_ids.iter().any(|existing| existing == symbol_id)
        {
            self.symbol_ids.push(symbol_id.to_string());
        }

        for reason in why {
            if self.seen_why.insert(reason.clone()) {
                self.why.push(reason.clone());
            }
        }
    }

    fn add_graph_boost(&mut self, score: i32, why: String) {
        if self.seen_why.insert(why.clone()) {
            self.graph_score += score;
            self.why.push(why);
        }
    }
}

#[derive(Debug, Serialize)]
pub struct StatsOutput {
    root: String,
    files: usize,
    symbols: usize,
    imports: usize,
    references: usize,
    tests: usize,
    configs: usize,
    languages: BTreeMap<Language, usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

pub fn list_symbols(root: &Path, index: &CodeIndex, limit: usize) -> Result<SymbolsOutput> {
    let symbols = index
        .symbols
        .iter()
        .take(limit)
        .filter_map(|symbol| {
            file_by_id(index, &symbol.file_id).map(|file| SymbolListItem {
                file: file.path.clone(),
                name: symbol.name.clone(),
                kind: symbol.kind.clone(),
                lines: [symbol.start_line, symbol.end_line],
                visibility: symbol.visibility.clone(),
            })
        })
        .collect();

    Ok(SymbolsOutput {
        root: root_label(root),
        symbols,
        warnings: stale_warnings(root, index),
    })
}

pub fn find_symbol(
    root: &Path,
    index: &CodeIndex,
    symbol_name: &str,
    limit: usize,
) -> Result<SymbolOutput> {
    let mut matches: Vec<&SymbolRecord> = index
        .symbols
        .iter()
        .filter(|symbol| {
            symbol.name.eq_ignore_ascii_case(symbol_name)
                || symbol
                    .name
                    .to_ascii_lowercase()
                    .contains(&symbol_name.to_ascii_lowercase())
        })
        .collect();

    matches.sort_by(|left, right| {
        let left_exact = left.name.eq_ignore_ascii_case(symbol_name);
        let right_exact = right.name.eq_ignore_ascii_case(symbol_name);
        right_exact
            .cmp(&left_exact)
            .then(left.file_id.cmp(&right.file_id))
            .then(left.start_line.cmp(&right.start_line))
    });

    let details = matches
        .into_iter()
        .take(limit)
        .filter_map(|symbol| {
            let file = file_by_id(index, &symbol.file_id)?;
            Some(SymbolDetail {
                file: file.path.clone(),
                name: symbol.name.clone(),
                kind: symbol.kind.clone(),
                language: symbol.language,
                lines: [symbol.start_line, symbol.end_line],
                visibility: symbol.visibility.clone(),
                signature: symbol.signature.clone(),
                imports: imports_for_file(index, &file.path),
                referenced_by: references_to_file(index, &file.path),
                calls: calls_from_symbol(index, symbol),
                references: references_from_symbol(index, symbol),
                called_by: called_by_symbol(index, symbol),
            })
        })
        .collect();

    Ok(SymbolOutput {
        query: symbol_name.to_string(),
        matches: details,
        warnings: stale_warnings(root, index),
    })
}

pub fn run_query(
    root: &Path,
    index: &CodeIndex,
    question: &str,
    limit: usize,
    include_snippets: bool,
) -> Result<QueryOutput> {
    let ranked = ranker::rank(index, question, limit);
    let matched_files: BTreeSet<String> =
        ranked.iter().map(|match_| match_.file_id.clone()).collect();
    let matched_symbols = ranked
        .iter()
        .filter(|match_| match_.symbol_id.is_some())
        .count();

    let matches = ranked
        .into_iter()
        .enumerate()
        .filter_map(|(rank_index, ranked)| {
            let file = file_by_id(index, &ranked.file_id)?;
            let symbol = ranked
                .symbol_id
                .as_deref()
                .and_then(|symbol_id| index.symbols.iter().find(|symbol| symbol.id == symbol_id));

            let snippet = include_snippets
                .then(|| snippet_for(root, file, symbol))
                .flatten();

            Some(QueryMatch {
                rank: rank_index + 1,
                score: ranked.score,
                file: file.path.clone(),
                language: file.language,
                symbol: symbol.map(|symbol| QuerySymbol {
                    name: symbol.name.clone(),
                    kind: symbol.kind.clone(),
                    lines: [symbol.start_line, symbol.end_line],
                    visibility: symbol.visibility.clone(),
                    signature: symbol.signature.clone(),
                }),
                snippet,
                why: ranked.why,
                related_tests: related_tests(index, file),
            })
        })
        .collect();

    Ok(QueryOutput {
        query: question.to_string(),
        root: root_label(root),
        matches,
        stats: QueryStats {
            searched_files: index.files.len(),
            matched_files: matched_files.len(),
            matched_symbols,
        },
        warnings: stale_warnings(root, index),
    })
}

pub fn build_context(
    root: &Path,
    index: &CodeIndex,
    task: &str,
    limit: usize,
    snippets_per_file: usize,
    include_snippets: bool,
) -> Result<ContextOutput> {
    let candidate_limit = limit.saturating_mul(4);
    let ranked = ranker::rank(index, task, candidate_limit);
    let mut grouped: BTreeMap<String, ContextCandidate> = BTreeMap::new();

    for (rank_index, ranked_match) in ranked.iter().enumerate() {
        let entry = grouped
            .entry(ranked_match.file_id.clone())
            .or_insert_with(|| {
                ContextCandidate::new(ranked_match.file_id.clone(), ranked_match.score, rank_index)
            });
        entry.add_match(
            ranked_match.score,
            ranked_match.symbol_id.as_deref(),
            &ranked_match.why,
        );
    }
    add_graph_context(index, &ranked, &mut grouped);
    add_reference_context(index, &ranked, &mut grouped);

    let mut candidates: Vec<ContextCandidate> = grouped.into_values().collect();
    candidates.sort_by(|left, right| {
        right
            .score()
            .cmp(&left.score())
            .then(left.first_rank.cmp(&right.first_rank))
            .then(left.file_id.cmp(&right.file_id))
    });

    let mut selected_symbols = 0;
    let mut selected_related_tests = 0;
    let read_first: Vec<ContextFile> = candidates
        .into_iter()
        .take(limit)
        .enumerate()
        .filter_map(|(rank_index, candidate)| {
            let file = file_by_id(index, &candidate.file_id)?;
            let symbol_records: Vec<&SymbolRecord> = candidate
                .symbol_ids
                .iter()
                .filter_map(|symbol_id| symbol_by_id(index, symbol_id))
                .collect();

            let symbols: Vec<QuerySymbol> = symbol_records
                .iter()
                .map(|symbol| QuerySymbol {
                    name: symbol.name.clone(),
                    kind: symbol.kind.clone(),
                    lines: [symbol.start_line, symbol.end_line],
                    visibility: symbol.visibility.clone(),
                    signature: symbol.signature.clone(),
                })
                .collect();

            let snippets = context_snippets(
                root,
                file,
                &symbol_records,
                snippets_per_file,
                include_snippets,
            );
            let related_tests = related_tests(index, file);
            let imports = resolved_imports_for_file(index, &file.path);
            let referenced_by = references_to_file(index, &file.path);
            let calls = calls_from_file(index, file);
            let called_by = called_by_file(index, file);
            let blast_radius =
                blast_radius_for(&imports, &referenced_by, &related_tests, &calls, &called_by);

            selected_symbols += symbols.len();
            selected_related_tests += related_tests.len();

            Some(ContextFile {
                rank: rank_index + 1,
                score: candidate.score(),
                file: file.path.clone(),
                language: file.language,
                symbols,
                snippets,
                imports,
                referenced_by,
                blast_radius,
                calls,
                called_by,
                related_tests,
                why: candidate.why,
            })
        })
        .collect();

    Ok(ContextOutput {
        task: task.to_string(),
        root: root_label(root),
        stats: ContextStats {
            candidate_matches: ranked.len(),
            selected_files: read_first.len(),
            selected_symbols,
            related_tests: selected_related_tests,
        },
        read_first,
        warnings: stale_warnings(root, index),
    })
}

pub fn benchmark_context(
    root: &Path,
    index: &CodeIndex,
    task: &str,
    limit: usize,
    snippets_per_file: usize,
    include_snippets: bool,
) -> Result<BenchmarkOutput> {
    let baseline = baseline_benchmark(root, index, task);
    let context = build_context(
        root,
        index,
        task,
        limit,
        snippets_per_file,
        include_snippets,
    )?;
    let packet = serde_json::to_string(&context)?;
    let packet_tokens = estimate_tokens(&packet);
    let top_files = context
        .read_first
        .iter()
        .map(|file| BenchmarkContextFile {
            file: file.file.clone(),
            score: file.score,
            risk: file.blast_radius.risk.clone(),
            why: file.why.iter().take(3).cloned().collect(),
        })
        .collect();
    let callsieve = CallsieveBenchmark {
        strategy: "callsieve context packet".to_string(),
        selected_files: context.stats.selected_files,
        selected_symbols: context.stats.selected_symbols,
        related_tests: context.stats.related_tests,
        packet_bytes: packet.len(),
        estimated_packet_tokens: packet_tokens,
        top_files,
    };

    let token_savings =
        baseline.estimated_total_tokens as isize - callsieve.estimated_packet_tokens as isize;
    let reduction_percent = if baseline.estimated_total_tokens == 0 {
        0.0
    } else {
        (token_savings as f64 / baseline.estimated_total_tokens as f64) * 100.0
    };
    let savings = SavingsBenchmark {
        avoided_grep_commands: baseline.grep_commands.saturating_sub(1),
        avoided_file_reads: baseline
            .matched_files
            .saturating_sub(callsieve.selected_files),
        estimated_token_savings: token_savings,
        estimated_token_reduction_percent: reduction_percent,
        notes: vec![
            "Estimates use one token per four UTF-8 bytes.".to_string(),
            "Baseline simulates grepping task terms, then reading every matched file.".to_string(),
            "Callsieve cost is the serialized context packet for the same task.".to_string(),
        ],
    };

    Ok(BenchmarkOutput {
        task: task.to_string(),
        root: root_label(root),
        estimator: "local deterministic token estimate".to_string(),
        baseline,
        callsieve,
        savings,
        warnings: context.warnings,
    })
}

pub fn benchmark_suite(
    root: &Path,
    index: &CodeIndex,
    suite: BenchmarkSuiteInput,
    limit: usize,
    snippets_per_file: usize,
    include_snippets: bool,
) -> Result<BenchmarkSuiteOutput> {
    let mut task_outputs = Vec::new();
    let mut expected_files = 0;
    let mut expected_files_found = 0;
    let mut tasks_with_all_expected_files = 0;
    let mut total_estimated_token_savings = 0;
    let mut total_estimated_reduction_percent = 0.0;
    let mut observed_summary = ObservedSessionAccumulator::default();

    for task in suite.tasks {
        let benchmark = benchmark_context(
            root,
            index,
            &task.task,
            limit,
            snippets_per_file,
            include_snippets,
        )?;
        let selected_files: Vec<String> = benchmark
            .callsieve
            .top_files
            .iter()
            .map(|file| file.file.clone())
            .collect();
        let selected_set: BTreeSet<&str> = selected_files.iter().map(String::as_str).collect();
        let expected_files_for_task = task.expected_files.len();
        let expected_files_found_for_task: Vec<String> = task
            .expected_files
            .iter()
            .filter(|file| selected_set.contains(file.as_str()))
            .cloned()
            .collect();
        let expected_files_missing: Vec<String> = task
            .expected_files
            .iter()
            .filter(|file| !selected_set.contains(file.as_str()))
            .cloned()
            .collect();
        let expected_file_recall =
            recall(expected_files_found_for_task.len(), expected_files_for_task);

        expected_files += expected_files_for_task;
        expected_files_found += expected_files_found_for_task.len();
        if expected_files_for_task > 0 && expected_files_missing.is_empty() {
            tasks_with_all_expected_files += 1;
        }
        total_estimated_token_savings += benchmark.savings.estimated_token_savings;
        total_estimated_reduction_percent += benchmark.savings.estimated_token_reduction_percent;

        let observed_session = task.observed.map(|observed| {
            let output = observed_session_output(observed);
            observed_summary.add(&output);
            output
        });

        task_outputs.push(BenchmarkSuiteTaskOutput {
            id: task.id,
            task: task.task,
            expected_files: task.expected_files,
            selected_files,
            expected_files_found: expected_files_found_for_task,
            expected_files_missing,
            expected_file_recall,
            benchmark,
            observed_session,
        });
    }

    let task_count = task_outputs.len();
    let summary = BenchmarkSuiteSummary {
        task_count,
        tasks_with_all_expected_files,
        expected_files,
        expected_files_found,
        expected_file_recall: recall(expected_files_found, expected_files),
        total_estimated_token_savings,
        average_estimated_token_reduction_percent: if task_count == 0 {
            0.0
        } else {
            total_estimated_reduction_percent / task_count as f64
        },
        observed_session: observed_summary.finish(),
    };

    Ok(BenchmarkSuiteOutput {
        root: root_label(root),
        task_count,
        tasks: task_outputs,
        summary,
        warnings: stale_warnings(root, index),
    })
}

pub fn stats(root: &Path, index: &CodeIndex) -> Result<StatsOutput> {
    let mut languages = BTreeMap::new();
    for file in &index.files {
        *languages.entry(file.language).or_insert(0) += 1;
    }

    Ok(StatsOutput {
        root: root_label(root),
        files: index.files.len(),
        symbols: index.symbols.len(),
        imports: index.imports.len(),
        references: index.references.len(),
        tests: index.files.iter().filter(|file| file.is_test).count(),
        configs: index.files.iter().filter(|file| file.is_config).count(),
        languages,
        warnings: stale_warnings(root, index),
    })
}

fn file_by_id<'a>(index: &'a CodeIndex, file_id: &str) -> Option<&'a FileRecord> {
    index.files.iter().find(|file| file.id == file_id)
}

fn symbol_by_id<'a>(index: &'a CodeIndex, symbol_id: &str) -> Option<&'a SymbolRecord> {
    index.symbols.iter().find(|symbol| symbol.id == symbol_id)
}

fn file_by_path<'a>(index: &'a CodeIndex, path: &str) -> Option<&'a FileRecord> {
    index.files.iter().find(|file| file.path == path)
}

fn add_graph_context(
    index: &CodeIndex,
    ranked: &[ranker::RankedMatch],
    grouped: &mut BTreeMap<String, ContextCandidate>,
) {
    const IMPORTED_FILE_BOOST: i32 = 12;
    const REFERENCING_FILE_BOOST: i32 = 8;

    let matched_file_ids: BTreeSet<&str> = ranked
        .iter()
        .map(|match_| match_.file_id.as_str())
        .collect();

    for file_id in matched_file_ids {
        let Some(file) = file_by_id(index, file_id) else {
            continue;
        };

        for imported_path in resolved_imports_for_file(index, &file.path) {
            let Some(imported_file) = file_by_path(index, &imported_path) else {
                continue;
            };
            let entry = grouped
                .entry(imported_file.id.clone())
                .or_insert_with(|| ContextCandidate::new(imported_file.id.clone(), 0, usize::MAX));
            entry.add_graph_boost(
                IMPORTED_FILE_BOOST,
                format!("referenced by matched file: {}", file.path),
            );
        }

        for referencing_path in references_to_file(index, &file.path) {
            let Some(referencing_file) = file_by_path(index, &referencing_path) else {
                continue;
            };
            let entry = grouped
                .entry(referencing_file.id.clone())
                .or_insert_with(|| {
                    ContextCandidate::new(referencing_file.id.clone(), 0, usize::MAX)
                });
            entry.add_graph_boost(
                REFERENCING_FILE_BOOST,
                format!("references matched file: {}", file.path),
            );
        }
    }
}

fn add_reference_context(
    index: &CodeIndex,
    ranked: &[ranker::RankedMatch],
    grouped: &mut BTreeMap<String, ContextCandidate>,
) {
    const CALLEE_BOOST: i32 = 10;
    const CALLER_BOOST: i32 = 14;

    let matched_file_ids: BTreeSet<&str> = ranked
        .iter()
        .map(|match_| match_.file_id.as_str())
        .collect();
    let matched_symbol_ids: BTreeSet<&str> = ranked
        .iter()
        .filter_map(|match_| match_.symbol_id.as_deref())
        .collect();

    for reference in &index.references {
        if matched_file_ids.contains(reference.file_id.as_str())
            && let Some(target_path) = reference.target_path.as_deref()
            && let Some(target_file) = file_by_path(index, target_path)
        {
            let entry = grouped
                .entry(target_file.id.clone())
                .or_insert_with(|| ContextCandidate::new(target_file.id.clone(), 0, usize::MAX));
            entry.add_graph_boost(
                CALLEE_BOOST,
                format!(
                    "{} from matched file: {}",
                    reference.kind, reference.target_name
                ),
            );
        }

        if let Some(target_path) = reference.target_path.as_deref()
            && file_by_path(index, target_path)
                .is_some_and(|target_file| matched_file_ids.contains(target_file.id.as_str()))
            && let Some(source_file) = file_by_path(index, &reference.source_path)
        {
            let entry = grouped
                .entry(source_file.id.clone())
                .or_insert_with(|| ContextCandidate::new(source_file.id.clone(), 0, usize::MAX));
            entry.add_graph_boost(
                CALLER_BOOST,
                format!("{} matched file: {}", reference.kind, target_path),
            );
        }

        if let Some(target_symbol_id) = reference.target_symbol_id.as_deref()
            && matched_symbol_ids.contains(target_symbol_id)
            && let Some(source_file) = file_by_path(index, &reference.source_path)
        {
            let entry = grouped
                .entry(source_file.id.clone())
                .or_insert_with(|| ContextCandidate::new(source_file.id.clone(), 0, usize::MAX));
            entry.add_graph_boost(
                CALLER_BOOST,
                format!(
                    "{} matched symbol: {}",
                    reference.kind, reference.target_name
                ),
            );
        }

        if let Some(source_symbol_id) = reference.source_symbol_id.as_deref()
            && matched_symbol_ids.contains(source_symbol_id)
            && let Some(target_path) = reference.target_path.as_deref()
            && let Some(target_file) = file_by_path(index, target_path)
        {
            let entry = grouped
                .entry(target_file.id.clone())
                .or_insert_with(|| ContextCandidate::new(target_file.id.clone(), 0, usize::MAX));
            entry.add_graph_boost(
                CALLEE_BOOST,
                format!(
                    "{} from matched symbol: {}",
                    reference.kind, reference.target_name
                ),
            );
        }
    }
}

fn imports_for_file(index: &CodeIndex, path: &str) -> Vec<String> {
    index
        .imports
        .iter()
        .filter(|import| import.source_path == path)
        .map(|import| {
            import
                .resolved_path
                .clone()
                .unwrap_or_else(|| import.imported.clone())
        })
        .collect()
}

fn resolved_imports_for_file(index: &CodeIndex, path: &str) -> Vec<String> {
    let mut imports: Vec<String> = index
        .imports
        .iter()
        .filter(|import| import.source_path == path)
        .filter_map(|import| import.resolved_path.clone())
        .collect();
    imports.sort();
    imports.dedup();
    imports
}

fn references_to_file(index: &CodeIndex, path: &str) -> Vec<String> {
    let mut references: Vec<String> = index
        .imports
        .iter()
        .filter(|import| import.resolved_path.as_deref() == Some(path))
        .map(|import| import.source_path.clone())
        .collect();
    references.sort();
    references.dedup();
    references
}

fn calls_from_symbol(index: &CodeIndex, symbol: &SymbolRecord) -> Vec<ReferenceEdge> {
    index
        .references
        .iter()
        .filter(|reference| {
            reference.source_symbol_id.as_deref() == Some(symbol.id.as_str())
                && reference.kind == "call"
        })
        .map(|reference| reference_edge(index, reference))
        .take(10)
        .collect()
}

fn references_from_symbol(index: &CodeIndex, symbol: &SymbolRecord) -> Vec<ReferenceEdge> {
    index
        .references
        .iter()
        .filter(|reference| {
            reference.source_symbol_id.as_deref() == Some(symbol.id.as_str())
                && reference.kind != "call"
        })
        .map(|reference| reference_edge(index, reference))
        .take(10)
        .collect()
}

fn called_by_symbol(index: &CodeIndex, symbol: &SymbolRecord) -> Vec<ReferenceEdge> {
    index
        .references
        .iter()
        .filter(|reference| {
            reference.target_symbol_id.as_deref() == Some(symbol.id.as_str())
                && reference.kind == "call"
        })
        .map(|reference| reference_edge(index, reference))
        .take(10)
        .collect()
}

fn calls_from_file(index: &CodeIndex, file: &FileRecord) -> Vec<ReferenceEdge> {
    index
        .references
        .iter()
        .filter(|reference| reference.source_path == file.path && reference.kind == "call")
        .map(|reference| reference_edge(index, reference))
        .take(10)
        .collect()
}

fn called_by_file(index: &CodeIndex, file: &FileRecord) -> Vec<ReferenceEdge> {
    index
        .references
        .iter()
        .filter(|reference| {
            reference.target_path.as_deref() == Some(file.path.as_str())
                && reference.source_path != file.path
                && reference.kind == "call"
        })
        .map(|reference| reference_edge(index, reference))
        .take(10)
        .collect()
}

fn reference_edge(index: &CodeIndex, reference: &ReferenceRecord) -> ReferenceEdge {
    ReferenceEdge {
        file: reference.source_path.clone(),
        symbol: reference
            .source_symbol_id
            .as_deref()
            .and_then(|symbol_id| symbol_by_id(index, symbol_id))
            .map(|symbol| symbol.name.clone()),
        target: reference.target_name.clone(),
        target_file: reference.target_path.clone(),
        kind: reference.kind.clone(),
        line: reference.line,
    }
}

fn blast_radius_for(
    imports: &[String],
    referenced_by: &[String],
    related_tests: &[RelatedTest],
    calls: &[ReferenceEdge],
    called_by: &[ReferenceEdge],
) -> BlastRadius {
    let tests: Vec<String> = related_tests.iter().map(|test| test.file.clone()).collect();
    let call_targets = edge_files(calls, true);
    let callers = edge_files(called_by, false);
    let total_edges = imports.len() + referenced_by.len() + call_targets.len() + callers.len();
    let risk = if referenced_by.len() >= 5 || callers.len() >= 5 || total_edges >= 8 {
        "high"
    } else if total_edges > 0 || !tests.is_empty() {
        "medium"
    } else {
        "low"
    };

    BlastRadius {
        imports: imports.to_vec(),
        referenced_by: referenced_by.to_vec(),
        tests,
        calls: call_targets,
        called_by: callers,
        risk: risk.to_string(),
    }
}

fn edge_files(edges: &[ReferenceEdge], use_target: bool) -> Vec<String> {
    let mut files: Vec<String> = edges
        .iter()
        .filter_map(|edge| {
            if use_target {
                edge.target_file.clone()
            } else {
                Some(edge.file.clone())
            }
        })
        .collect();
    files.sort();
    files.dedup();
    files
}

fn baseline_benchmark(root: &Path, index: &CodeIndex, task: &str) -> BaselineBenchmark {
    let terms = benchmark_terms(task);
    let mut matched_files = Vec::new();
    let mut matched_lines = 0;
    let mut search_result_tokens = 0;
    let mut read_tokens = 0;

    for file in &index.files {
        let Ok(content) = fs::read_to_string(root.join(&file.path)) else {
            continue;
        };
        let mut file_matched_lines = 0;
        let mut file_search_tokens = 0;

        for (line_index, line) in content.lines().enumerate() {
            let line_lower = line.to_ascii_lowercase();
            if terms.iter().any(|term| line_lower.contains(term)) {
                file_matched_lines += 1;
                let result_line = format!("{}:{}:{}", file.path, line_index + 1, line.trim());
                file_search_tokens += estimate_tokens(&result_line);
            }
        }

        if file_matched_lines > 0 {
            matched_files.push(file.path.clone());
            matched_lines += file_matched_lines;
            search_result_tokens += file_search_tokens;
            read_tokens += estimate_tokens(&content);
        }
    }

    matched_files.sort();
    let matched_files_sample = matched_files.iter().take(10).cloned().collect();

    BaselineBenchmark {
        strategy: "naive grep term scan plus full matched-file reads".to_string(),
        grep_terms: terms.clone(),
        grep_commands: terms.len(),
        matched_files: matched_files.len(),
        matched_lines,
        estimated_search_result_tokens: search_result_tokens,
        estimated_read_tokens: read_tokens,
        estimated_total_tokens: search_result_tokens + read_tokens,
        matched_files_sample,
    }
}

fn benchmark_terms(task: &str) -> Vec<String> {
    let stopwords = BTreeSet::from([
        "about",
        "behavior",
        "change",
        "code",
        "file",
        "find",
        "fix",
        "for",
        "from",
        "handled",
        "how",
        "implement",
        "make",
        "the",
        "this",
        "update",
        "what",
        "where",
        "with",
    ]);
    let mut terms: Vec<String> = formatter::tokenize(task)
        .into_iter()
        .filter(|term| term.len() >= 3 && !stopwords.contains(term.as_str()))
        .collect();

    terms.sort();
    terms.dedup();

    if terms.is_empty() {
        terms = formatter::tokenize(task)
            .into_iter()
            .filter(|term| term.len() >= 2)
            .collect();
        terms.sort();
        terms.dedup();
    }

    terms
}

fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.len().div_ceil(4)
    }
}

fn recall(found: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        found as f64 / total as f64
    }
}

fn observed_session_output(observed: ObservedSessionComparison) -> ObservedSessionOutput {
    let token_savings = observed.baseline.tokens as isize - observed.callsieve.tokens as isize;
    let token_reduction_percent = if observed.baseline.tokens == 0 {
        0.0
    } else {
        (token_savings as f64 / observed.baseline.tokens as f64) * 100.0
    };
    let savings = ObservedSessionSavings {
        avoided_grep_commands: observed
            .baseline
            .grep_commands
            .saturating_sub(observed.callsieve.grep_commands),
        avoided_file_reads: observed
            .baseline
            .file_reads
            .saturating_sub(observed.callsieve.file_reads),
        token_savings,
        token_reduction_percent,
    };

    ObservedSessionOutput {
        baseline: observed.baseline,
        callsieve: observed.callsieve,
        savings,
    }
}

#[derive(Default)]
struct ObservedSessionAccumulator {
    sessions: usize,
    baseline_tokens: usize,
    callsieve_tokens: usize,
    avoided_grep_commands: usize,
    avoided_file_reads: usize,
}

impl ObservedSessionAccumulator {
    fn add(&mut self, observed: &ObservedSessionOutput) {
        self.sessions += 1;
        self.baseline_tokens += observed.baseline.tokens;
        self.callsieve_tokens += observed.callsieve.tokens;
        self.avoided_grep_commands += observed.savings.avoided_grep_commands;
        self.avoided_file_reads += observed.savings.avoided_file_reads;
    }

    fn finish(self) -> Option<ObservedSessionSummary> {
        if self.sessions == 0 {
            return None;
        }

        let token_savings = self.baseline_tokens as isize - self.callsieve_tokens as isize;
        let token_reduction_percent = if self.baseline_tokens == 0 {
            0.0
        } else {
            (token_savings as f64 / self.baseline_tokens as f64) * 100.0
        };

        Some(ObservedSessionSummary {
            sessions: self.sessions,
            baseline_tokens: self.baseline_tokens,
            callsieve_tokens: self.callsieve_tokens,
            token_savings,
            token_reduction_percent,
            avoided_grep_commands: self.avoided_grep_commands,
            avoided_file_reads: self.avoided_file_reads,
        })
    }
}

fn snippet_for(root: &Path, file: &FileRecord, symbol: Option<&SymbolRecord>) -> Option<Snippet> {
    let content = fs::read_to_string(root.join(&file.path)).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return None;
    }

    let (start, end) = match symbol {
        Some(symbol) => {
            let start = symbol.start_line.max(1);
            let end = symbol.end_line.min(start + 18).min(lines.len()).max(start);
            (start, end)
        }
        None => (1, lines.len().min(12)),
    };

    let text = lines[start - 1..end].join("\n");
    Some(Snippet {
        lines: [start, end],
        text,
    })
}

fn context_snippets(
    root: &Path,
    file: &FileRecord,
    symbols: &[&SymbolRecord],
    snippets_per_file: usize,
    include_snippets: bool,
) -> Vec<Snippet> {
    if !include_snippets || snippets_per_file == 0 {
        return Vec::new();
    }

    let mut snippets: Vec<Snippet> = symbols
        .iter()
        .take(snippets_per_file)
        .filter_map(|symbol| snippet_for(root, file, Some(*symbol)))
        .collect();

    if snippets.is_empty()
        && let Some(snippet) = snippet_for(root, file, None)
    {
        snippets.push(snippet);
    }

    snippets
}

fn related_tests(index: &CodeIndex, file: &FileRecord) -> Vec<RelatedTest> {
    if file.is_test {
        return Vec::new();
    }

    let stem = file_stem(&file.path).to_ascii_lowercase();
    index
        .files
        .iter()
        .filter(|candidate| {
            candidate.is_test && candidate.path.to_ascii_lowercase().contains(stem.as_str())
        })
        .take(5)
        .map(|test_file| RelatedTest {
            file: test_file.path.clone(),
            symbols: index
                .symbols
                .iter()
                .filter(|symbol| symbol.file_id == test_file.id)
                .map(|symbol| symbol.name.clone())
                .collect(),
        })
        .collect()
}

fn file_stem(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(path)
        .split('.')
        .next()
        .unwrap_or(path)
        .to_string()
}

fn stale_warnings(root: &Path, index: &CodeIndex) -> Vec<String> {
    index
        .files
        .iter()
        .filter_map(|file| {
            let path = root.join(&file.path);
            let metadata = fs::metadata(&path).ok()?;
            let mtime = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs())
                .unwrap_or_default();

            (metadata.len() != file.size_bytes || mtime != file.mtime)
                .then(|| format!("stale index entry: {}", file.path))
        })
        .take(20)
        .collect()
}

fn root_label(path: &Path) -> String {
    if path == Path::new(".") {
        ".".to_string()
    } else {
        path.display().to_string()
    }
}

pub fn path_tokens(path: &str) -> Vec<String> {
    formatter::tokenize(&path.replace(['/', '.', '-', '_'], " "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer;
    use std::fs;

    fn write(path: impl AsRef<Path>, content: &str) {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn fixture_index() -> (tempfile::TempDir, CodeIndex) {
        let temp = tempfile::tempdir().unwrap();
        write(
            temp.path().join("src/auth/session.ts"),
            "import { tokenFor } from './token';\n\nexport function createSession(userId: string) {\n  return tokenFor(userId);\n}\n\nexport function refreshSession(userId: string) {\n  return createSession(userId);\n}\n",
        );
        write(
            temp.path().join("src/auth/token.ts"),
            "export function tokenFor(userId: string) {\n  return `token:${userId}`;\n}\n",
        );
        write(
            temp.path().join("src/auth/session.test.ts"),
            "import { createSession } from './session';\n\ntest('createSession returns token-backed session', () => {\n  createSession('demo');\n});\n",
        );

        let index = indexer::build_index(temp.path()).unwrap();
        (temp, index)
    }

    #[test]
    fn query_ranks_exact_symbol_above_keyword_match() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("auth.ts"),
            "export function createSession() {\n  return true;\n}\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("notes.ts"),
            "export const authNotes = () => true;\n",
        )
        .unwrap();

        let index = indexer::build_index(temp.path()).unwrap();
        let output = run_query(temp.path(), &index, "createSession auth", 10, true).unwrap();

        assert_eq!(
            output.matches[0].symbol.as_ref().unwrap().name,
            "createSession"
        );
    }

    #[test]
    fn context_groups_symbol_matches_by_file_and_deduplicates_why() {
        let (temp, index) = fixture_index();
        let output = build_context(
            temp.path(),
            &index,
            "change createSession refreshSession session token behavior",
            8,
            2,
            true,
        )
        .unwrap();

        let first = &output.read_first[0];
        assert_eq!(first.file, "src/auth/session.ts");
        assert!(first.symbols.len() >= 2);

        let unique_why: BTreeSet<&String> = first.why.iter().collect();
        assert_eq!(first.why.len(), unique_why.len());
    }

    #[test]
    fn context_respects_limit_and_snippets_per_file() {
        let (temp, index) = fixture_index();
        let output = build_context(
            temp.path(),
            &index,
            "change createSession refreshSession token behavior",
            1,
            1,
            true,
        )
        .unwrap();

        assert_eq!(output.read_first.len(), 1);
        assert_eq!(output.read_first[0].snippets.len(), 1);
    }

    #[test]
    fn context_includes_related_tests_for_matching_fixture_names() {
        let (temp, index) = fixture_index();
        let output = build_context(
            temp.path(),
            &index,
            "change createSession token behavior",
            8,
            2,
            true,
        )
        .unwrap();

        assert!(
            output.read_first[0]
                .related_tests
                .iter()
                .any(|test| test.file == "src/auth/session.test.ts")
        );
    }

    #[test]
    fn context_adds_imported_files_as_graph_neighbors() {
        let (temp, index) = fixture_index();
        let output = build_context(
            temp.path(),
            &index,
            "change createSession behavior",
            8,
            2,
            true,
        )
        .unwrap();

        let token_file = output
            .read_first
            .iter()
            .find(|file| file.file == "src/auth/token.ts")
            .expect("imported token file should be selected");

        assert!(
            token_file
                .why
                .contains(&"referenced by matched file: src/auth/session.ts".to_string())
        );
    }

    #[test]
    fn context_adds_referencing_files_as_graph_neighbors() {
        let (temp, index) = fixture_index();
        let output =
            build_context(temp.path(), &index, "change tokenFor behavior", 8, 2, true).unwrap();

        let session_file = output
            .read_first
            .iter()
            .find(|file| file.file == "src/auth/session.ts")
            .expect("referencing session file should be selected");

        assert!(
            session_file
                .why
                .contains(&"references matched file: src/auth/token.ts".to_string())
        );
    }

    #[test]
    fn context_keeps_exact_symbol_matches_above_graph_neighbors() {
        let (temp, index) = fixture_index();
        let output =
            build_context(temp.path(), &index, "change tokenFor behavior", 8, 2, true).unwrap();

        assert_eq!(output.read_first[0].file, "src/auth/token.ts");
    }

    #[test]
    fn context_populates_graph_hints_for_selected_files() {
        let (temp, index) = fixture_index();
        let output = build_context(
            temp.path(),
            &index,
            "change createSession behavior",
            8,
            2,
            true,
        )
        .unwrap();

        let session_file = output
            .read_first
            .iter()
            .find(|file| file.file == "src/auth/session.ts")
            .unwrap();

        assert!(
            session_file
                .imports
                .contains(&"src/auth/token.ts".to_string())
        );
        assert!(
            session_file
                .referenced_by
                .contains(&"src/auth/session.test.ts".to_string())
        );
    }

    #[test]
    fn context_populates_blast_radius_for_selected_files() {
        let (temp, index) = fixture_index();
        let output = build_context(
            temp.path(),
            &index,
            "change createSession behavior",
            8,
            2,
            true,
        )
        .unwrap();

        let session_file = output
            .read_first
            .iter()
            .find(|file| file.file == "src/auth/session.ts")
            .unwrap();

        assert!(
            session_file
                .blast_radius
                .imports
                .contains(&"src/auth/token.ts".to_string())
        );
        assert!(
            session_file
                .blast_radius
                .referenced_by
                .contains(&"src/auth/session.test.ts".to_string())
        );
        assert!(
            session_file
                .blast_radius
                .tests
                .contains(&"src/auth/session.test.ts".to_string())
        );
        assert_eq!(session_file.blast_radius.risk, "medium");
    }

    #[test]
    fn context_marks_widely_referenced_files_as_high_risk() {
        let temp = tempfile::tempdir().unwrap();
        write(
            temp.path().join("src/shared.ts"),
            "export function sharedToken() {\n  return 'token';\n}\n",
        );
        for index in 0..5 {
            write(
                temp.path().join(format!("src/consumer{index}.ts")),
                &format!(
                    "import {{ sharedToken }} from './shared';\n\nexport function consumer{index}() {{\n  return sharedToken();\n}}\n"
                ),
            );
        }

        let index = indexer::build_index(temp.path()).unwrap();
        let output = build_context(
            temp.path(),
            &index,
            "change sharedToken behavior",
            8,
            2,
            true,
        )
        .unwrap();

        let shared_file = output
            .read_first
            .iter()
            .find(|file| file.file == "src/shared.ts")
            .unwrap();

        assert_eq!(shared_file.blast_radius.risk, "high");
        assert_eq!(shared_file.blast_radius.referenced_by.len(), 5);
    }

    #[test]
    fn benchmark_estimates_token_savings_against_grep_read_loop() {
        let temp = tempfile::tempdir().unwrap();
        write(
            temp.path().join("src/auth/session.ts"),
            "import { tokenFor } from './token';\n\nexport function createSession(userId: string) {\n  return tokenFor(userId);\n}\n",
        );
        write(
            temp.path().join("src/auth/token.ts"),
            "export function tokenFor(userId: string) {\n  return `token:${userId}`;\n}\n",
        );
        for index in 0..6 {
            write(
                temp.path().join(format!("src/noise{index}.ts")),
                &format!(
                    "export const unrelated{index} = true;\n// {}\n",
                    "token ".repeat(1_000)
                ),
            );
        }

        let index = indexer::build_index(temp.path()).unwrap();
        let output = benchmark_context(
            temp.path(),
            &index,
            "change createSession token behavior",
            8,
            2,
            true,
        )
        .unwrap();

        assert!(output.baseline.matched_files > output.callsieve.selected_files);
        assert!(output.baseline.estimated_total_tokens > output.callsieve.estimated_packet_tokens);
        assert!(output.savings.estimated_token_savings > 0);
        assert!(
            output
                .callsieve
                .top_files
                .iter()
                .any(|file| file.file == "src/auth/session.ts")
        );
    }

    #[test]
    fn benchmark_suite_reports_expected_file_recall_and_observed_sessions() {
        let (temp, index) = fixture_index();
        let suite = BenchmarkSuiteInput {
            tasks: vec![BenchmarkSuiteTaskInput {
                id: Some("auth-session".to_string()),
                task: "change createSession token behavior".to_string(),
                expected_files: vec![
                    "src/auth/session.ts".to_string(),
                    "src/auth/token.ts".to_string(),
                ],
                observed: Some(ObservedSessionComparison {
                    baseline: ObservedSessionMetrics {
                        grep_commands: 6,
                        file_reads: 9,
                        tokens: 12_000,
                    },
                    callsieve: ObservedSessionMetrics {
                        grep_commands: 1,
                        file_reads: 3,
                        tokens: 4_000,
                    },
                }),
            }],
        };

        let output = benchmark_suite(temp.path(), &index, suite, 8, 2, true).unwrap();

        assert_eq!(output.task_count, 1);
        assert_eq!(output.summary.expected_files, 2);
        assert_eq!(output.summary.expected_files_found, 2);
        assert_eq!(output.summary.expected_file_recall, 1.0);
        assert_eq!(output.summary.tasks_with_all_expected_files, 1);

        let observed = output.summary.observed_session.unwrap();
        assert_eq!(observed.sessions, 1);
        assert_eq!(observed.token_savings, 8_000);
        assert_eq!(observed.avoided_grep_commands, 5);
        assert_eq!(observed.avoided_file_reads, 6);
    }

    #[test]
    fn context_deduplicates_graph_reasons() {
        let (temp, index) = fixture_index();
        let output = build_context(
            temp.path(),
            &index,
            "change createSession refreshSession behavior",
            8,
            2,
            true,
        )
        .unwrap();

        for file in output.read_first {
            let unique_why: BTreeSet<&String> = file.why.iter().collect();
            assert_eq!(file.why.len(), unique_why.len());
        }
    }
}
