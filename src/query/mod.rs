pub mod formatter;
pub mod ranker;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::{
    indexer::{SCHEMA_VERSION, language::Language},
    store::{self, CodeIndex, FileRecord, ReferenceRecord, SymbolRecord},
};

const MAX_CONTEXT_SYMBOLS_PER_FILE: usize = 8;
const MAX_CONTEXT_GRAPH_SCORE: i32 = 240;

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
    edge_source: String,
    confidence: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    lsp_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_range: Option<[usize; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_range: Option<[usize; 2]>,
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
    #[serde(default, alias = "trace")]
    session: Option<ObservedSessionComparison>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    commands: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    files_read: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    notes: Vec<String>,
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    miss_reasons: Vec<String>,
    expected_file_recall: f64,
    benchmark: BenchmarkOutput,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_session: Option<ObservedSessionOutput>,
}

#[derive(Debug, Serialize)]
struct BenchmarkSuiteSummary {
    task_count: usize,
    tasks_with_all_expected_files: usize,
    tasks_with_misses: usize,
    expected_files: usize,
    expected_files_found: usize,
    missed_expected_files: usize,
    expected_file_recall: f64,
    total_estimated_token_savings: isize,
    average_estimated_token_reduction_percent: f64,
    total_estimated_avoided_grep_commands: usize,
    total_estimated_avoided_file_reads: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    misses: Vec<BenchmarkSuiteMiss>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_session: Option<ObservedSessionSummary>,
}

#[derive(Debug, Clone, Serialize)]
struct BenchmarkSuiteMiss {
    id: Option<String>,
    task: String,
    missing_files: Vec<String>,
    selected_files: Vec<String>,
    reasons: Vec<String>,
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

#[derive(Debug, Clone, Deserialize)]
pub struct BenchmarkReportManifest {
    repos: Vec<BenchmarkReportRepoInput>,
    #[serde(default)]
    thresholds: PilotThresholds,
}

#[derive(Debug, Clone, Deserialize)]
struct BenchmarkReportRepoInput {
    path: PathBuf,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    languages: Vec<String>,
    #[serde(default)]
    external: bool,
    #[serde(default, alias = "suite", alias = "suite_path", alias = "tasks")]
    suite_path: Option<PathBuf>,
    #[serde(default, alias = "suites", alias = "suite_paths")]
    suite_paths: Vec<PathBuf>,
    #[serde(
        default,
        alias = "trace",
        alias = "trace_path",
        alias = "session_trace"
    )]
    trace_path: Option<PathBuf>,
    #[serde(default, alias = "traces", alias = "trace_paths")]
    trace_paths: Vec<PathBuf>,
    #[serde(default)]
    thresholds: Option<PilotThresholds>,
}

impl BenchmarkReportRepoInput {
    fn suite_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if let Some(path) = &self.suite_path {
            paths.push(path.clone());
        }
        paths.extend(self.suite_paths.clone());
        paths
    }

    fn trace_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if let Some(path) = &self.trace_path {
            paths.push(path.clone());
        }
        paths.extend(self.trace_paths.clone());
        paths
    }

    fn thresholds<'a>(&'a self, manifest: &'a PilotThresholds) -> &'a PilotThresholds {
        self.thresholds.as_ref().unwrap_or(manifest)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PilotThresholds {
    #[serde(default = "default_min_recall")]
    minimum_recall: f64,
    #[serde(default = "default_min_token_reduction_percent")]
    minimum_token_reduction_percent: f64,
    #[serde(default)]
    minimum_observed_sessions: usize,
    #[serde(default)]
    minimum_external_repos: usize,
    #[serde(default = "default_min_observed_token_reduction_percent")]
    minimum_observed_token_reduction_percent: f64,
    #[serde(default = "default_maximum_controlled_replay_ratio")]
    maximum_controlled_replay_ratio: f64,
    #[serde(default)]
    maximum_trace_violations: usize,
    #[serde(default)]
    require_fresh_index: bool,
    #[serde(default)]
    require_lsp_where_available: bool,
    #[serde(default)]
    require_codex_bootstrap: bool,
}

impl Default for PilotThresholds {
    fn default() -> Self {
        Self {
            minimum_recall: default_min_recall(),
            minimum_token_reduction_percent: default_min_token_reduction_percent(),
            minimum_observed_sessions: 0,
            minimum_external_repos: 0,
            minimum_observed_token_reduction_percent: default_min_observed_token_reduction_percent(
            ),
            maximum_controlled_replay_ratio: default_maximum_controlled_replay_ratio(),
            maximum_trace_violations: 0,
            require_fresh_index: false,
            require_lsp_where_available: false,
            require_codex_bootstrap: false,
        }
    }
}

fn default_min_recall() -> f64 {
    1.0
}

fn default_min_token_reduction_percent() -> f64 {
    0.0
}

fn default_min_observed_token_reduction_percent() -> f64 {
    0.0
}

fn default_maximum_controlled_replay_ratio() -> f64 {
    1.0
}

#[derive(Debug, Serialize)]
pub struct BenchmarkReportOutput {
    repo_count: usize,
    repos: Vec<BenchmarkReportRepoOutput>,
    summary: BenchmarkReportSummary,
}

#[derive(Debug, Serialize)]
pub struct BenchmarkDoctorOutput {
    status: String,
    repos: usize,
    checks: usize,
    failures: usize,
    check_results: Vec<BenchmarkDoctorCheck>,
}

#[derive(Debug, Serialize)]
struct BenchmarkDoctorCheck {
    path: String,
    check: String,
    status: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct BenchmarkReportRepoOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    path: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    languages: Vec<String>,
    external: bool,
    suite_path: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    suite_paths: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    trace_path: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    trace_paths: Vec<String>,
    task_count: usize,
    expected_file_recall: f64,
    estimated_token_savings: isize,
    estimated_token_reduction_percent: f64,
    avoided_grep_commands: usize,
    avoided_file_reads: usize,
    expected_files: usize,
    expected_files_found: usize,
    missed_expected_files: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    misses: Vec<BenchmarkSuiteMiss>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_trace: Option<TraceSummaryOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_session: Option<TraceSummaryOutput>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct BenchmarkReportSummary {
    repos: usize,
    tasks: usize,
    expected_files: usize,
    expected_files_found: usize,
    missed_expected_files: usize,
    expected_file_recall: f64,
    total_estimated_token_savings: isize,
    average_estimated_token_reduction_percent: f64,
    total_avoided_grep_commands: usize,
    total_avoided_file_reads: usize,
    repos_with_misses: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    misses: Vec<BenchmarkReportMiss>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_trace: Option<TraceSummaryOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_session: Option<TraceSummaryOutput>,
}

#[derive(Debug, Serialize)]
pub struct PilotReportOutput {
    command: &'static str,
    status: String,
    repo_count: usize,
    session_count: usize,
    proof: PilotProofSummary,
    thresholds: PilotThresholds,
    benchmark: BenchmarkReportOutput,
    repos: Vec<PilotRepoOutput>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    failures: Vec<PilotFailure>,
}

#[derive(Debug, Serialize)]
pub struct ProofReportOutput {
    command: &'static str,
    status: String,
    claim: &'static str,
    proof: PilotProofSummary,
    thresholds: PilotThresholds,
    benchmark: BenchmarkReportOutput,
    repos: Vec<PilotRepoOutput>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    failures: Vec<PilotFailure>,
}

#[derive(Debug, Serialize)]
struct PilotProofSummary {
    repos: usize,
    sessions: usize,
    observed_sessions: usize,
    controlled_replay_sessions: usize,
    unclassified_sessions: usize,
    external_repos: usize,
    languages: Vec<String>,
    expected_file_recall: f64,
    token_reduction_percent: f64,
    observed_token_reduction_percent: f64,
    controlled_replay_ratio: f64,
    token_savings: isize,
    avoided_grep_commands: usize,
    avoided_file_reads: usize,
    trace_policy_violations: usize,
    fresh_indexes: usize,
    daemon_fresh_repos: usize,
    lsp_enriched_repos: usize,
    lsp_available_repos: usize,
    codex_bootstrap_repos: usize,
}

#[derive(Debug, Serialize)]
struct PilotRepoOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    path: String,
    languages: Vec<String>,
    status: IndexStatusOutput,
    trace_check: TraceCheckOutput,
    threshold_status: String,
}

#[derive(Debug, Serialize)]
struct PilotFailure {
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    path: String,
    check: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct BenchmarkReportMiss {
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    path: String,
    misses: Vec<BenchmarkSuiteMiss>,
}

#[derive(Debug, Deserialize)]
struct TraceSuiteInput {
    tasks: Vec<TraceTaskInput>,
}

#[derive(Debug, Deserialize)]
struct TraceTaskInput {
    id: Option<String>,
    task: Option<String>,
    #[serde(default)]
    expected_files: Vec<String>,
    observed: Option<ObservedSessionComparison>,
    #[serde(default, alias = "trace")]
    session: Option<ObservedSessionComparison>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum TraceCollection {
    ObservedSession,
    ControlledReplay,
    Unclassified,
}

#[derive(Debug, Serialize, Clone)]
pub struct TraceSummaryOutput {
    sessions: usize,
    observed_sessions: usize,
    controlled_replay_sessions: usize,
    unclassified_sessions: usize,
    baseline_tokens: usize,
    callsieve_tokens: usize,
    token_savings: isize,
    token_reduction_percent: f64,
    avoided_grep_commands: usize,
    avoided_file_reads: usize,
    files_still_missed: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    missed_files: Vec<TraceMiss>,
}

#[derive(Debug, Serialize, Clone)]
struct TraceMiss {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    task: Option<String>,
    files: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct TraceReplayOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<TraceReplayMetadata>,
    tasks: Vec<TraceReplayTaskOutput>,
}

#[derive(Debug, Serialize)]
struct TraceReplayMetadata {
    client: String,
    model: String,
    collection: String,
}

#[derive(Debug, Serialize)]
struct TraceReplayTaskOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    task: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    expected_files: Vec<String>,
    session: ObservedSessionComparison,
}

#[derive(Debug)]
pub struct CodexSessionTraceInput {
    pub task: String,
    pub model: String,
    pub expected_files: Vec<String>,
    pub limit: usize,
    pub snippets_per_file: usize,
    pub include_snippets: bool,
}

#[derive(Debug)]
struct ContextFirstTraceInput {
    id: Option<String>,
    task: String,
    expected_files: Vec<String>,
    limit: usize,
    snippets_per_file: usize,
    include_snippets: bool,
    callsieve_command: String,
    callsieve_notes: Vec<String>,
}

#[derive(Debug)]
struct ContextCandidate {
    file_id: String,
    best_score: i32,
    graph_score: i32,
    graph_confidence: f64,
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
            graph_confidence: 0.0,
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
            && self.symbol_ids.len() < MAX_CONTEXT_SYMBOLS_PER_FILE
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

    fn add_graph_boost(&mut self, score: i32, confidence: f64, why: String) {
        self.graph_confidence = self.graph_confidence.max(confidence);
        if self.seen_why.insert(why.clone()) {
            self.graph_score = (self.graph_score + score).min(MAX_CONTEXT_GRAPH_SCORE);
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

#[derive(Debug, Serialize)]
pub struct IndexStatusOutput {
    root: String,
    index_exists: bool,
    fresh: bool,
    schema_version: Option<u32>,
    expected_schema_version: u32,
    watch_status: String,
    watcher_mode: String,
    index_generation: u64,
    indexed_at: Option<u64>,
    index_age_seconds: Option<u64>,
    files: usize,
    symbols: usize,
    imports: usize,
    references: usize,
    lsp_enriched: bool,
    lsp_enriched_at: Option<u64>,
    lsp_enrichment_age_seconds: Option<u64>,
    stale_files: usize,
    changed_files: usize,
    removed_files: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    stale_file_sample: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    lsp_servers: Vec<crate::store::LspServerStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    daemon: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct TraceCheckOutput {
    status: String,
    strict: bool,
    sessions: usize,
    violations: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    violation_details: Vec<TraceCheckViolation>,
}

#[derive(Debug, Serialize)]
struct TraceCheckViolation {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    task: Option<String>,
    event_kind: String,
    first_violation_command: String,
    first_grep_command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_file_read_command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_callsieve_context_command: Option<String>,
    reason: String,
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
    let candidate_limit = limit.saturating_mul(16);
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
            .then_with(|| right.graph_confidence.total_cmp(&left.graph_confidence))
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
    let mut total_expected_files = 0;
    let mut total_expected_files_found = 0;
    let mut tasks_with_all_expected_files = 0;
    let mut total_estimated_token_savings = 0;
    let mut total_estimated_reduction_percent = 0.0;
    let mut total_estimated_avoided_grep_commands = 0;
    let mut total_estimated_avoided_file_reads = 0;
    let mut misses = Vec::new();
    let mut observed_summary = ObservedSessionAccumulator::default();

    for task in suite.tasks {
        let BenchmarkSuiteTaskInput {
            id,
            task,
            expected_files: task_expected_files,
            observed,
            session,
        } = task;
        let benchmark = benchmark_context(
            root,
            index,
            &task,
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
        let expected_files_for_task = task_expected_files.len();
        let expected_files_found_for_task: Vec<String> = task_expected_files
            .iter()
            .filter(|file| selected_set.contains(file.as_str()))
            .cloned()
            .collect();
        let expected_files_missing: Vec<String> = task_expected_files
            .iter()
            .filter(|file| !selected_set.contains(file.as_str()))
            .cloned()
            .collect();
        let expected_file_recall =
            recall(expected_files_found_for_task.len(), expected_files_for_task);
        let miss_reasons =
            miss_reasons_for(index, &expected_files_missing, &selected_files, &benchmark);

        total_expected_files += expected_files_for_task;
        total_expected_files_found += expected_files_found_for_task.len();
        if expected_files_for_task > 0 && expected_files_missing.is_empty() {
            tasks_with_all_expected_files += 1;
        }
        total_estimated_token_savings += benchmark.savings.estimated_token_savings;
        total_estimated_reduction_percent += benchmark.savings.estimated_token_reduction_percent;
        total_estimated_avoided_grep_commands += benchmark.savings.avoided_grep_commands;
        total_estimated_avoided_file_reads += benchmark.savings.avoided_file_reads;

        if !expected_files_missing.is_empty() {
            misses.push(BenchmarkSuiteMiss {
                id: id.clone(),
                task: task.clone(),
                missing_files: expected_files_missing.clone(),
                selected_files: selected_files.clone(),
                reasons: miss_reasons.clone(),
            });
        }

        let observed_session = session.or(observed).map(|observed| {
            let output = observed_session_output(observed);
            observed_summary.add(&output);
            output
        });

        task_outputs.push(BenchmarkSuiteTaskOutput {
            id,
            task,
            expected_files: task_expected_files,
            selected_files,
            expected_files_found: expected_files_found_for_task,
            expected_files_missing,
            miss_reasons,
            expected_file_recall,
            benchmark,
            observed_session,
        });
    }

    let task_count = task_outputs.len();
    let summary = BenchmarkSuiteSummary {
        task_count,
        tasks_with_all_expected_files,
        tasks_with_misses: misses.len(),
        expected_files: total_expected_files,
        expected_files_found: total_expected_files_found,
        missed_expected_files: total_expected_files.saturating_sub(total_expected_files_found),
        expected_file_recall: recall(total_expected_files_found, total_expected_files),
        total_estimated_token_savings,
        average_estimated_token_reduction_percent: if task_count == 0 {
            0.0
        } else {
            total_estimated_reduction_percent / task_count as f64
        },
        total_estimated_avoided_grep_commands,
        total_estimated_avoided_file_reads,
        misses,
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

pub fn trace_replay(
    root: &Path,
    index: &CodeIndex,
    suite: BenchmarkSuiteInput,
    limit: usize,
    snippets_per_file: usize,
    include_snippets: bool,
) -> Result<TraceReplayOutput> {
    let mut tasks = Vec::new();

    for task in suite.tasks {
        let BenchmarkSuiteTaskInput {
            id,
            task,
            expected_files,
            observed: _,
            session: _,
        } = task;

        let mut callsieve_command = format!(
            "callsieve context {} {:?} --limit {limit} --snippets-per-file {snippets_per_file}",
            root.display(),
            task
        );
        if !include_snippets {
            callsieve_command.push_str(" --no-snippets");
        }

        tasks.push(trace_task_for_context_first_session(
            root,
            index,
            ContextFirstTraceInput {
                id,
                task,
                expected_files,
                limit,
                snippets_per_file,
                include_snippets,
                callsieve_command,
                callsieve_notes: vec![
                    "Controlled local replay, not human-session telemetry.".to_string(),
                    "CallSieve cost is the serialized context packet plus full reads of selected read-first files."
                        .to_string(),
                ],
            },
        )?);
    }

    Ok(TraceReplayOutput {
        metadata: Some(TraceReplayMetadata {
            client: "callsieve".to_string(),
            model: "controlled-replay".to_string(),
            collection: "controlled_replay".to_string(),
        }),
        tasks,
    })
}

pub fn codex_session_trace(
    root: &Path,
    index: &CodeIndex,
    input: CodexSessionTraceInput,
) -> Result<TraceReplayOutput> {
    let CodexSessionTraceInput {
        task,
        model,
        expected_files,
        limit,
        snippets_per_file,
        include_snippets,
    } = input;
    let mut callsieve_command = format!(
        "callsieve codex-session {} {:?} --model {:?} --limit {limit} --snippets-per-file {snippets_per_file}",
        root.display(),
        task,
        model
    );
    if !include_snippets {
        callsieve_command.push_str(" --no-snippets");
    }

    let task = trace_task_for_context_first_session(
        root,
        index,
        ContextFirstTraceInput {
            id: Some("codex-chatgpt-session".to_string()),
            task,
            expected_files,
            limit,
            snippets_per_file,
            include_snippets,
            callsieve_command,
            callsieve_notes: vec![
                "Codex/ChatGPT session trace scaffold.".to_string(),
                "Baseline is deterministic local grep/read replay for the same task.".to_string(),
                format!("Assisted session model: {model}."),
                "CallSieve cost is the serialized context packet plus full reads of selected read-first files."
                    .to_string(),
            ],
        },
    )?;

    Ok(TraceReplayOutput {
        metadata: Some(TraceReplayMetadata {
            client: "codex-chatgpt".to_string(),
            model: model.to_string(),
            collection: "controlled_replay".to_string(),
        }),
        tasks: vec![task],
    })
}

pub fn benchmark_report(
    manifest: BenchmarkReportManifest,
    limit: usize,
    snippets_per_file: usize,
    include_snippets: bool,
) -> Result<BenchmarkReportOutput> {
    let mut repos = Vec::new();
    let mut total_tasks = 0;
    let mut total_expected_files = 0;
    let mut total_expected_files_found = 0;
    let mut total_estimated_token_savings = 0;
    let mut total_estimated_reduction_percent = 0.0;
    let mut total_avoided_grep_commands = 0;
    let mut total_avoided_file_reads = 0;
    let mut misses = Vec::new();
    let mut trace_accumulator = TraceAccumulator::default();

    for repo in manifest.repos {
        let index = store::json_store::load_index(&repo.path)?;
        let suite_paths = repo.suite_paths();
        let trace_paths = repo.trace_paths();
        let is_external = repo_is_external(&repo);
        let mut repo_task_count = 0;
        let mut repo_expected_files = 0;
        let mut repo_expected_files_found = 0;
        let mut repo_estimated_token_savings = 0;
        let mut repo_reduction_percent_total = 0.0;
        let mut repo_avoided_grep_commands = 0;
        let mut repo_avoided_file_reads = 0;
        let mut repo_misses = Vec::new();
        let mut repo_warnings = Vec::new();

        for suite_path in &suite_paths {
            let suite_json = fs::read_to_string(suite_path)?;
            let suite: BenchmarkSuiteInput = serde_json::from_str(&suite_json)?;
            let output = benchmark_suite(
                &repo.path,
                &index,
                suite,
                limit,
                snippets_per_file,
                include_snippets,
            )?;

            repo_task_count += output.summary.task_count;
            repo_expected_files += output.summary.expected_files;
            repo_expected_files_found += output.summary.expected_files_found;
            repo_estimated_token_savings += output.summary.total_estimated_token_savings;
            repo_reduction_percent_total +=
                output.summary.average_estimated_token_reduction_percent
                    * output.summary.task_count as f64;
            repo_avoided_grep_commands += output.summary.total_estimated_avoided_grep_commands;
            repo_avoided_file_reads += output.summary.total_estimated_avoided_file_reads;
            repo_misses.extend(output.summary.misses);
            repo_warnings.extend(output.warnings);
        }

        let mut repo_trace_accumulator = TraceAccumulator::default();
        for trace_path in &trace_paths {
            let trace_json = fs::read_to_string(trace_path)?;
            let trace_summary = trace_summary_from_str(&trace_json)?;
            repo_trace_accumulator.add_summary(&trace_summary);
            trace_accumulator.add_summary(&trace_summary);
        }
        let session_trace = repo_trace_accumulator.finish();
        let observed_session = observed_only_summary(session_trace.clone());

        total_tasks += repo_task_count;
        total_expected_files += repo_expected_files;
        total_expected_files_found += repo_expected_files_found;
        total_estimated_token_savings += repo_estimated_token_savings;
        total_estimated_reduction_percent += if repo_task_count == 0 {
            0.0
        } else {
            repo_reduction_percent_total / repo_task_count as f64
        };
        total_avoided_grep_commands += repo_avoided_grep_commands;
        total_avoided_file_reads += repo_avoided_file_reads;

        if !repo_misses.is_empty() {
            misses.push(BenchmarkReportMiss {
                label: repo.label.clone(),
                path: repo.path.display().to_string(),
                misses: repo_misses.clone(),
            });
        }

        repos.push(BenchmarkReportRepoOutput {
            label: repo.label,
            path: repo.path.display().to_string(),
            languages: repo.languages,
            external: is_external,
            suite_path: suite_paths
                .first()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            suite_paths: suite_paths
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            trace_path: trace_paths.first().map(|path| path.display().to_string()),
            trace_paths: trace_paths
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            task_count: repo_task_count,
            expected_file_recall: recall(repo_expected_files_found, repo_expected_files),
            estimated_token_savings: repo_estimated_token_savings,
            estimated_token_reduction_percent: if repo_task_count == 0 {
                0.0
            } else {
                repo_reduction_percent_total / repo_task_count as f64
            },
            avoided_grep_commands: repo_avoided_grep_commands,
            avoided_file_reads: repo_avoided_file_reads,
            expected_files: repo_expected_files,
            expected_files_found: repo_expected_files_found,
            missed_expected_files: repo_expected_files.saturating_sub(repo_expected_files_found),
            misses: repo_misses,
            session_trace,
            observed_session,
            warnings: repo_warnings,
        });
    }

    let repo_count = repos.len();
    let session_trace = trace_accumulator.finish();
    let observed_session = observed_only_summary(session_trace.clone());
    let summary = BenchmarkReportSummary {
        repos: repo_count,
        tasks: total_tasks,
        expected_files: total_expected_files,
        expected_files_found: total_expected_files_found,
        missed_expected_files: total_expected_files.saturating_sub(total_expected_files_found),
        expected_file_recall: recall(total_expected_files_found, total_expected_files),
        total_estimated_token_savings,
        average_estimated_token_reduction_percent: if repo_count == 0 {
            0.0
        } else {
            total_estimated_reduction_percent / repo_count as f64
        },
        total_avoided_grep_commands,
        total_avoided_file_reads,
        repos_with_misses: misses.len(),
        misses,
        session_trace,
        observed_session,
    };

    Ok(BenchmarkReportOutput {
        repo_count,
        repos,
        summary,
    })
}

pub fn pilot_report(
    manifest: BenchmarkReportManifest,
    limit: usize,
    snippets_per_file: usize,
    include_snippets: bool,
) -> Result<PilotReportOutput> {
    let benchmark = benchmark_report(manifest.clone(), limit, snippets_per_file, include_snippets)?;
    let mut repos = Vec::new();
    let mut failures = Vec::new();
    let mut language_set = BTreeSet::new();
    let mut trace_policy_violations = 0usize;
    let mut fresh_indexes = 0usize;
    let mut daemon_fresh_repos = 0usize;
    let mut lsp_enriched_repos = 0usize;
    let mut lsp_available_repos = 0usize;
    let mut codex_bootstrap_repos = 0usize;
    let mut external_repos = 0usize;
    let mut observed_trace_accumulator = TraceAccumulator::default();
    let mut controlled_trace_accumulator = TraceAccumulator::default();
    let mut unclassified_trace_accumulator = TraceAccumulator::default();

    for repo in &manifest.repos {
        let index = store::json_store::load_index(&repo.path).ok();
        let status = index_status(&repo.path, index.as_ref());
        let is_external = repo_is_external(repo);
        let codex_bootstrap = codex_bootstrap_installed(&repo.path);
        let daemon_fresh = daemon_is_fresh(&repo.path);
        if is_external {
            external_repos += 1;
        }
        if status.fresh {
            fresh_indexes += 1;
        }
        if daemon_fresh {
            daemon_fresh_repos += 1;
        }
        if status.lsp_enriched {
            lsp_enriched_repos += 1;
        }
        if status.lsp_servers.iter().any(|server| server.available) {
            lsp_available_repos += 1;
        }
        if codex_bootstrap {
            codex_bootstrap_repos += 1;
        }

        let mut languages = repo.languages.clone();
        if languages.is_empty()
            && let Some(index) = index.as_ref()
        {
            languages = index
                .files
                .iter()
                .map(|file| language_name(file.language).to_string())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
        }
        for language in &languages {
            language_set.insert(language.clone());
        }

        let mut repo_trace_violations = 0usize;
        let mut repo_trace_sessions = 0usize;
        let mut repo_violation_details = Vec::new();
        let mut repo_mislabeled_controlled_replay = false;
        for trace_path in repo.trace_paths() {
            let trace_json = fs::read_to_string(&trace_path)?;
            let trace_value: serde_json::Value = serde_json::from_str(&trace_json)?;
            let trace_collection = trace_collection_from_value(&trace_value);
            let trace_summary = trace_summary_from_str(&trace_json)?;
            match trace_collection {
                TraceCollection::ObservedSession => {
                    observed_trace_accumulator.add_summary(&trace_summary)
                }
                TraceCollection::ControlledReplay => {
                    controlled_trace_accumulator.add_summary(&trace_summary)
                }
                TraceCollection::Unclassified => {
                    unclassified_trace_accumulator.add_summary(&trace_summary)
                }
            }
            if trace_collection == TraceCollection::ObservedSession
                && trace_has_controlled_replay_markers(&trace_value)
            {
                repo_mislabeled_controlled_replay = true;
            }
            let check = trace_check_from_str_with_options(&trace_json, true)?;
            repo_trace_sessions += check.sessions;
            repo_trace_violations += check.violations;
            repo_violation_details.extend(check.violation_details);
        }
        trace_policy_violations += repo_trace_violations;
        let trace_check = TraceCheckOutput {
            status: if repo_trace_violations == 0 {
                "pass"
            } else {
                "fail"
            }
            .to_string(),
            strict: true,
            sessions: repo_trace_sessions,
            violations: repo_trace_violations,
            violation_details: repo_violation_details,
        };

        let thresholds = repo.thresholds(&manifest.thresholds);
        let repo_benchmark = benchmark
            .repos
            .iter()
            .find(|candidate| candidate.path == repo.path.display().to_string());
        let mut repo_failed = false;

        if let Some(repo_benchmark) = repo_benchmark {
            if repo_benchmark.expected_file_recall < thresholds.minimum_recall {
                repo_failed = true;
                failures.push(PilotFailure {
                    label: repo.label.clone(),
                    path: repo.path.display().to_string(),
                    check: "minimum_recall".to_string(),
                    message: format!(
                        "expected-file recall {:.3} is below threshold {:.3}",
                        repo_benchmark.expected_file_recall, thresholds.minimum_recall
                    ),
                });
            }
            if repo_benchmark.estimated_token_reduction_percent
                < thresholds.minimum_token_reduction_percent
            {
                repo_failed = true;
                failures.push(PilotFailure {
                    label: repo.label.clone(),
                    path: repo.path.display().to_string(),
                    check: "minimum_token_reduction_percent".to_string(),
                    message: format!(
                        "estimated token reduction {:.1}% is below threshold {:.1}%",
                        repo_benchmark.estimated_token_reduction_percent,
                        thresholds.minimum_token_reduction_percent
                    ),
                });
            }
        }

        if trace_check.violations > thresholds.maximum_trace_violations {
            repo_failed = true;
            failures.push(PilotFailure {
                label: repo.label.clone(),
                path: repo.path.display().to_string(),
                check: "maximum_trace_violations".to_string(),
                message: format!(
                    "trace policy violations {} exceed threshold {}",
                    trace_check.violations, thresholds.maximum_trace_violations
                ),
            });
        }
        if repo_mislabeled_controlled_replay {
            repo_failed = true;
            failures.push(PilotFailure {
                label: repo.label.clone(),
                path: repo.path.display().to_string(),
                check: "observed_trace_mislabeled_controlled_replay".to_string(),
                message:
                    "trace metadata says observed_session but controlled-replay markers are present"
                        .to_string(),
            });
        }
        if thresholds.require_fresh_index && !status.fresh {
            repo_failed = true;
            failures.push(PilotFailure {
                label: repo.label.clone(),
                path: repo.path.display().to_string(),
                check: "require_fresh_index".to_string(),
                message: "index is stale or missing".to_string(),
            });
        }
        if thresholds.require_lsp_where_available
            && status.lsp_servers.iter().any(|server| server.available)
            && !status.lsp_enriched
        {
            repo_failed = true;
            failures.push(PilotFailure {
                label: repo.label.clone(),
                path: repo.path.display().to_string(),
                check: "require_lsp_where_available".to_string(),
                message: "local LSP server is available but index is not LSP-enriched".to_string(),
            });
        }
        if thresholds.require_codex_bootstrap && !codex_bootstrap {
            repo_failed = true;
            failures.push(PilotFailure {
                label: repo.label.clone(),
                path: repo.path.display().to_string(),
                check: "require_codex_bootstrap".to_string(),
                message: "Codex bootstrap files are missing".to_string(),
            });
        }

        repos.push(PilotRepoOutput {
            label: repo.label.clone(),
            path: repo.path.display().to_string(),
            languages,
            status,
            trace_check,
            threshold_status: if repo_failed { "fail" } else { "pass" }.to_string(),
        });
    }

    let observed_summary = observed_trace_accumulator.finish();
    let controlled_summary = controlled_trace_accumulator.finish();
    let unclassified_summary = unclassified_trace_accumulator.finish();
    let observed_sessions = observed_summary
        .as_ref()
        .map(|summary| summary.sessions)
        .unwrap_or_default();
    let controlled_replay_sessions = controlled_summary
        .as_ref()
        .map(|summary| summary.sessions)
        .unwrap_or_default();
    let unclassified_sessions = unclassified_summary
        .as_ref()
        .map(|summary| summary.sessions)
        .unwrap_or_default();
    let total_trace_sessions =
        observed_sessions + controlled_replay_sessions + unclassified_sessions;
    let observed_token_reduction_percent = observed_summary
        .as_ref()
        .map(|summary| summary.token_reduction_percent)
        .unwrap_or_default();
    let controlled_replay_ratio = if total_trace_sessions == 0 {
        0.0
    } else {
        controlled_replay_sessions as f64 / total_trace_sessions as f64
    };

    let proof = PilotProofSummary {
        repos: benchmark.summary.repos,
        sessions: total_trace_sessions,
        observed_sessions,
        controlled_replay_sessions,
        unclassified_sessions,
        external_repos,
        languages: language_set.into_iter().collect(),
        expected_file_recall: benchmark.summary.expected_file_recall,
        token_reduction_percent: benchmark.summary.average_estimated_token_reduction_percent,
        observed_token_reduction_percent,
        controlled_replay_ratio,
        token_savings: benchmark.summary.total_estimated_token_savings,
        avoided_grep_commands: benchmark.summary.total_avoided_grep_commands,
        avoided_file_reads: benchmark.summary.total_avoided_file_reads,
        trace_policy_violations,
        fresh_indexes,
        daemon_fresh_repos,
        lsp_enriched_repos,
        lsp_available_repos,
        codex_bootstrap_repos,
    };
    let session_count = proof.sessions;
    if proof.observed_sessions < manifest.thresholds.minimum_observed_sessions {
        failures.push(PilotFailure {
            label: None,
            path: ".".to_string(),
            check: "minimum_observed_sessions".to_string(),
            message: format!(
                "observed sessions {} are below threshold {}",
                proof.observed_sessions, manifest.thresholds.minimum_observed_sessions
            ),
        });
    }
    if proof.external_repos < manifest.thresholds.minimum_external_repos {
        failures.push(PilotFailure {
            label: None,
            path: ".".to_string(),
            check: "minimum_external_repos".to_string(),
            message: format!(
                "external repos {} are below threshold {}",
                proof.external_repos, manifest.thresholds.minimum_external_repos
            ),
        });
    }
    if proof.observed_token_reduction_percent
        < manifest.thresholds.minimum_observed_token_reduction_percent
    {
        failures.push(PilotFailure {
            label: None,
            path: ".".to_string(),
            check: "minimum_observed_token_reduction_percent".to_string(),
            message: format!(
                "observed token reduction {:.1}% is below threshold {:.1}%",
                proof.observed_token_reduction_percent,
                manifest.thresholds.minimum_observed_token_reduction_percent
            ),
        });
    }
    if proof.controlled_replay_ratio > manifest.thresholds.maximum_controlled_replay_ratio {
        failures.push(PilotFailure {
            label: None,
            path: ".".to_string(),
            check: "maximum_controlled_replay_ratio".to_string(),
            message: format!(
                "controlled replay ratio {:.3} exceeds threshold {:.3}",
                proof.controlled_replay_ratio, manifest.thresholds.maximum_controlled_replay_ratio
            ),
        });
    }
    let status = if failures.is_empty() { "pass" } else { "fail" }.to_string();

    Ok(PilotReportOutput {
        command: "pilot-report",
        status,
        repo_count: benchmark.repo_count,
        session_count,
        proof,
        thresholds: manifest.thresholds,
        benchmark,
        repos,
        failures,
    })
}

pub fn proof_report(
    manifest: BenchmarkReportManifest,
    limit: usize,
    snippets_per_file: usize,
    include_snippets: bool,
) -> Result<ProofReportOutput> {
    let pilot = pilot_report(manifest, limit, snippets_per_file, include_snippets)?;
    Ok(ProofReportOutput {
        command: "proof-report",
        status: pilot.status,
        claim: "CallSieve reduces grep/read token waste in real observed developer sessions only when observed-session gates pass; controlled replay is reported separately.",
        proof: pilot.proof,
        thresholds: pilot.thresholds,
        benchmark: pilot.benchmark,
        repos: pilot.repos,
        failures: pilot.failures,
    })
}

pub fn benchmark_doctor_from_str(manifest_json: &str) -> Result<BenchmarkDoctorOutput> {
    let value: serde_json::Value = serde_json::from_str(manifest_json)?;
    let repos = value
        .get("repos")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut check_results = Vec::new();

    if repos.is_empty() {
        check_results.push(BenchmarkDoctorCheck {
            path: ".".to_string(),
            check: "manifest.repos".to_string(),
            status: "fail".to_string(),
            message: "manifest must contain at least one repo".to_string(),
        });
    }

    for repo in &repos {
        let path = repo
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let suite_paths = manifest_path_values(repo, &["suite_path", "suite", "tasks"])
            .into_iter()
            .chain(manifest_path_values(repo, &["suite_paths", "suites"]))
            .collect::<Vec<_>>();
        let trace_paths = manifest_path_values(repo, &["trace_path", "trace", "session_trace"])
            .into_iter()
            .chain(manifest_path_values(repo, &["trace_paths", "traces"]))
            .collect::<Vec<_>>();

        push_path_check(
            &mut check_results,
            path,
            "repo_path",
            Path::new(path).is_dir(),
        );
        push_path_check(
            &mut check_results,
            path,
            "index",
            store::json_store::index_path(Path::new(path)).is_file(),
        );

        if suite_paths.is_empty() {
            push_path_check(&mut check_results, "", "suite_path", false);
        }
        for suite_path in &suite_paths {
            push_path_check(
                &mut check_results,
                suite_path,
                "suite_path",
                Path::new(suite_path).is_file(),
            );
            if Path::new(suite_path).is_file() {
                let suite_ok = fs::read_to_string(suite_path)
                    .ok()
                    .and_then(|json| serde_json::from_str::<BenchmarkSuiteInput>(&json).ok())
                    .is_some();
                push_parse_check(&mut check_results, suite_path, "suite_parse", suite_ok);
            }
        }

        for trace_path in &trace_paths {
            push_path_check(
                &mut check_results,
                trace_path,
                "trace_path",
                Path::new(trace_path).is_file(),
            );
            if Path::new(trace_path).is_file() {
                let trace_ok = fs::read_to_string(trace_path)
                    .ok()
                    .and_then(|json| trace_summary_from_str(&json).ok())
                    .is_some();
                push_parse_check(&mut check_results, trace_path, "trace_parse", trace_ok);
            }
        }
    }

    let failures = check_results
        .iter()
        .filter(|check| check.status == "fail")
        .count();
    Ok(BenchmarkDoctorOutput {
        status: if failures == 0 { "pass" } else { "fail" }.to_string(),
        repos: repos.len(),
        checks: check_results.len(),
        failures,
        check_results,
    })
}

pub fn pilot_doctor_from_str(manifest_json: &str) -> Result<BenchmarkDoctorOutput> {
    let value: serde_json::Value = serde_json::from_str(manifest_json)?;
    let manifest: BenchmarkReportManifest = serde_json::from_value(value.clone())?;
    let mut output = benchmark_doctor_from_str(manifest_json)?;
    let repos = value
        .get("repos")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();

    for repo_value in &repos {
        let path = repo_value
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let repo_input = manifest
            .repos
            .iter()
            .find(|repo| repo.path.as_path() == Path::new(path));
        let thresholds = repo_input
            .map(|repo| repo.thresholds(&manifest.thresholds))
            .unwrap_or(&manifest.thresholds);
        let index = store::json_store::load_index(Path::new(path)).ok();
        let status = index_status(Path::new(path), index.as_ref());

        if thresholds.require_fresh_index {
            output.check_results.push(BenchmarkDoctorCheck {
                path: path.to_string(),
                check: "fresh_index".to_string(),
                status: if status.fresh { "pass" } else { "fail" }.to_string(),
                message: if status.fresh {
                    "ok".to_string()
                } else {
                    "index is stale or missing".to_string()
                },
            });
        }

        if thresholds.require_lsp_where_available {
            let lsp_available = status.lsp_servers.iter().any(|server| server.available);
            output.check_results.push(BenchmarkDoctorCheck {
                path: path.to_string(),
                check: "lsp_enriched_when_available".to_string(),
                status: if !lsp_available || status.lsp_enriched {
                    "pass"
                } else {
                    "fail"
                }
                .to_string(),
                message: if !lsp_available || status.lsp_enriched {
                    "ok".to_string()
                } else {
                    "local LSP server is available but index is not LSP-enriched".to_string()
                },
            });
        }
    }

    output.failures = output
        .check_results
        .iter()
        .filter(|check| check.status == "fail")
        .count();
    output.checks = output.check_results.len();
    output.status = if output.failures == 0 { "pass" } else { "fail" }.to_string();
    Ok(output)
}

pub fn trace_summary_from_str(trace_json: &str) -> Result<TraceSummaryOutput> {
    let value: serde_json::Value = serde_json::from_str(trace_json)?;
    let collection = trace_collection_from_value(&value);
    if value.get("tasks").is_some() {
        let suite: TraceSuiteInput = serde_json::from_value(value)?;
        return Ok(trace_summary_from_tasks(suite.tasks, collection));
    }

    if value.get("baseline").is_some() && value.get("callsieve").is_some() {
        let expected_files = string_array(value.get("expected_files"));
        let observed: ObservedSessionComparison = serde_json::from_value(value.clone())?;
        return Ok(trace_summary_from_tasks(
            vec![TraceTaskInput {
                id: optional_string(value.get("id")),
                task: optional_string(value.get("task")),
                expected_files,
                observed: Some(observed),
                session: None,
            }],
            collection,
        ));
    }

    let task: TraceTaskInput = serde_json::from_value(value)?;
    Ok(trace_summary_from_tasks(vec![task], collection))
}

pub fn trace_check_from_str(trace_json: &str) -> Result<TraceCheckOutput> {
    trace_check_from_str_with_options(trace_json, false)
}

pub fn trace_check_from_str_with_options(
    trace_json: &str,
    strict: bool,
) -> Result<TraceCheckOutput> {
    let value: serde_json::Value = serde_json::from_str(trace_json)?;
    let tasks = trace_tasks_from_value(value);
    let mut sessions = 0;
    let mut violation_details = Vec::new();

    for task in tasks {
        let command_strings = trace_command_strings(&task);
        if command_strings.is_empty() {
            continue;
        }
        sessions += 1;

        let first_grep = command_strings
            .iter()
            .enumerate()
            .find(|(_, command)| is_grep_command(command));
        let first_context = command_strings
            .iter()
            .enumerate()
            .find(|(_, command)| is_callsieve_context_command(command));

        if let Some((grep_index, grep_command)) = first_grep
            && first_context.is_none_or(|(context_index, _)| grep_index < context_index)
        {
            violation_details.push(TraceCheckViolation {
                id: optional_string(task.get("id")),
                task: optional_string(task.get("task")),
                event_kind: "grep_before_context".to_string(),
                first_violation_command: grep_command.clone(),
                first_grep_command: grep_command.clone(),
                first_file_read_command: None,
                first_callsieve_context_command: first_context.map(|(_, command)| command.clone()),
                reason: "grep or broad search happened before callsieve_context".to_string(),
            });
            continue;
        }

        if strict {
            let first_read = command_strings
                .iter()
                .enumerate()
                .find(|(_, command)| is_file_read_command(command));
            if let Some((read_index, read_command)) = first_read
                && first_context.is_none_or(|(context_index, _)| read_index < context_index)
            {
                violation_details.push(TraceCheckViolation {
                    id: optional_string(task.get("id")),
                    task: optional_string(task.get("task")),
                    event_kind: "read_before_context".to_string(),
                    first_violation_command: read_command.clone(),
                    first_grep_command: read_command.clone(),
                    first_file_read_command: Some(read_command.clone()),
                    first_callsieve_context_command: first_context
                        .map(|(_, command)| command.clone()),
                    reason: "file read happened before callsieve_context in strict mode"
                        .to_string(),
                });
            }
        }
    }

    let violations = violation_details.len();
    Ok(TraceCheckOutput {
        status: if violations == 0 { "pass" } else { "fail" }.to_string(),
        strict,
        sessions,
        violations,
        violation_details,
    })
}

pub fn index_status(root: &Path, index: Option<&CodeIndex>) -> IndexStatusOutput {
    let Some(index) = index else {
        return IndexStatusOutput {
            root: root_label(root),
            index_exists: false,
            fresh: false,
            schema_version: None,
            expected_schema_version: SCHEMA_VERSION,
            watch_status: "missing".to_string(),
            watcher_mode: "none".to_string(),
            index_generation: 0,
            indexed_at: None,
            index_age_seconds: None,
            files: 0,
            symbols: 0,
            imports: 0,
            references: 0,
            lsp_enriched: false,
            lsp_enriched_at: None,
            lsp_enrichment_age_seconds: None,
            stale_files: 0,
            changed_files: 0,
            removed_files: 0,
            stale_file_sample: Vec::new(),
            last_error: None,
            lsp_servers: Vec::new(),
            daemon: daemon_state_value(root),
            warnings: vec![format!(
                "missing CallSieve index; run `callsieve index {}` first",
                root.display()
            )],
        };
    };

    let stale_detail = stale_file_details(root, index);
    let stale_file_sample = stale_detail.paths;
    let mut warnings = stale_file_sample
        .iter()
        .take(20)
        .map(|file| format!("stale index entry: {file}"))
        .collect::<Vec<_>>();
    warnings.extend(index.warnings.clone());
    let schema_matches = index.schema_version == SCHEMA_VERSION;

    IndexStatusOutput {
        root: root_label(root),
        index_exists: true,
        fresh: stale_file_sample.is_empty() && schema_matches,
        schema_version: Some(index.schema_version),
        expected_schema_version: SCHEMA_VERSION,
        watch_status: index.metadata.watch_status.clone(),
        watcher_mode: index.metadata.watcher_mode.clone(),
        index_generation: index.metadata.index_generation,
        indexed_at: (index.metadata.indexed_at > 0).then_some(index.metadata.indexed_at),
        index_age_seconds: index_age(index.metadata.indexed_at),
        files: index.files.len(),
        symbols: index.symbols.len(),
        imports: index.imports.len(),
        references: index.references.len(),
        lsp_enriched: index.metadata.lsp_enriched,
        lsp_enriched_at: (index.metadata.lsp_enriched_at > 0)
            .then_some(index.metadata.lsp_enriched_at),
        lsp_enrichment_age_seconds: index_age(index.metadata.lsp_enriched_at),
        stale_files: stale_file_sample.len(),
        changed_files: stale_detail.changed,
        removed_files: stale_detail.removed,
        stale_file_sample: stale_file_sample.into_iter().take(20).collect(),
        last_error: index.metadata.last_error.clone(),
        lsp_servers: index.metadata.lsp_servers.clone(),
        daemon: daemon_state_value(root),
        warnings,
    }
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

fn miss_reasons_for(
    index: &CodeIndex,
    missing_files: &[String],
    selected_files: &[String],
    benchmark: &BenchmarkOutput,
) -> Vec<String> {
    if missing_files.is_empty() {
        return Vec::new();
    }

    let mut reasons = Vec::new();
    let indexed_paths: BTreeSet<&str> = index.files.iter().map(|file| file.path.as_str()).collect();
    let unindexed_files: Vec<&str> = missing_files
        .iter()
        .map(String::as_str)
        .filter(|file| !indexed_paths.contains(file))
        .collect();

    if !unindexed_files.is_empty() {
        reasons.push(format!(
            "expected files are not in the index: {}",
            unindexed_files.join(", ")
        ));
    }

    if selected_files.is_empty() {
        reasons.push("no CallSieve files were selected".to_string());
    } else if unindexed_files.len() < missing_files.len()
        && benchmark.baseline.matched_files > selected_files.len()
    {
        reasons.push(format!(
            "expected file fell outside the selected context limit; selected {} of {} grep-matched files",
            selected_files.len(),
            benchmark.baseline.matched_files
        ));
    } else if unindexed_files.len() < missing_files.len() {
        reasons.push(
            "expected file did not match deterministic symbol, path, keyword, or graph signals"
                .to_string(),
        );
    }

    if benchmark.baseline.matched_files == 0 {
        reasons.push("baseline grep terms did not match any indexed files".to_string());
    }

    if benchmark.callsieve.selected_symbols == 0 {
        reasons.push("selected files had no matching indexed symbols".to_string());
    }

    reasons
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
                0.5,
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
                0.5,
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
                reference.confidence,
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
                reference.confidence,
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
                reference.confidence,
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
                reference.confidence,
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
        edge_source: reference.edge_source.clone(),
        confidence: reference.confidence,
        lsp_method: reference.lsp_method.clone(),
        source_range: reference.source_range,
        target_range: reference.target_range,
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

struct BaselineReplay {
    benchmark: BaselineBenchmark,
    matched_files: Vec<String>,
}

fn baseline_benchmark(root: &Path, index: &CodeIndex, task: &str) -> BaselineBenchmark {
    baseline_replay(root, index, task).benchmark
}

fn trace_task_for_context_first_session(
    root: &Path,
    index: &CodeIndex,
    input: ContextFirstTraceInput,
) -> Result<TraceReplayTaskOutput> {
    let ContextFirstTraceInput {
        id,
        task,
        expected_files,
        limit,
        snippets_per_file,
        include_snippets,
        callsieve_command,
        callsieve_notes,
    } = input;
    let baseline = baseline_replay(root, index, &task);
    let context = build_context(
        root,
        index,
        &task,
        limit,
        snippets_per_file,
        include_snippets,
    )?;
    let packet = serde_json::to_string(&context)?;
    let packet_tokens = estimate_tokens(&packet);
    let callsieve_files: Vec<String> = context
        .read_first
        .iter()
        .map(|file| file.file.clone())
        .collect();
    let callsieve_read_tokens = read_tokens_for_files(root, &callsieve_files);

    Ok(TraceReplayTaskOutput {
        id,
        task,
        expected_files,
        session: ObservedSessionComparison {
            baseline: ObservedSessionMetrics {
                grep_commands: baseline.benchmark.grep_commands,
                file_reads: baseline.matched_files.len(),
                tokens: baseline.benchmark.estimated_total_tokens,
                commands: baseline
                    .benchmark
                    .grep_terms
                    .iter()
                    .map(|term| format!("rg -n {term} {}", root.display()))
                    .collect(),
                files_read: baseline.matched_files,
                notes: vec![
                    "Controlled local replay, not human-session telemetry.".to_string(),
                    "Baseline simulates grepping task terms, then reading every matched indexed file."
                        .to_string(),
                ],
            },
            callsieve: ObservedSessionMetrics {
                grep_commands: 0,
                file_reads: callsieve_files.len(),
                tokens: packet_tokens + callsieve_read_tokens,
                commands: vec![callsieve_command],
                files_read: callsieve_files,
                notes: callsieve_notes,
            },
        },
    })
}

fn baseline_replay(root: &Path, index: &CodeIndex, task: &str) -> BaselineReplay {
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

    BaselineReplay {
        benchmark: BaselineBenchmark {
            strategy: "naive grep term scan plus full matched-file reads".to_string(),
            grep_terms: terms.clone(),
            grep_commands: terms.len(),
            matched_files: matched_files.len(),
            matched_lines,
            estimated_search_result_tokens: search_result_tokens,
            estimated_read_tokens: read_tokens,
            estimated_total_tokens: search_result_tokens + read_tokens,
            matched_files_sample,
        },
        matched_files,
    }
}

fn read_tokens_for_files(root: &Path, files: &[String]) -> usize {
    files
        .iter()
        .filter_map(|file| fs::read_to_string(root.join(file)).ok())
        .map(|content| estimate_tokens(&content))
        .sum()
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

#[derive(Default)]
struct TraceAccumulator {
    sessions: usize,
    observed_sessions: usize,
    controlled_replay_sessions: usize,
    unclassified_sessions: usize,
    baseline_tokens: usize,
    callsieve_tokens: usize,
    avoided_grep_commands: usize,
    avoided_file_reads: usize,
    files_still_missed: usize,
    missed_files: Vec<TraceMiss>,
}

impl TraceAccumulator {
    fn add_observed(
        &mut self,
        id: Option<String>,
        task: Option<String>,
        expected_files: Vec<String>,
        observed: ObservedSessionOutput,
        collection: TraceCollection,
    ) {
        self.sessions += 1;
        match collection {
            TraceCollection::ObservedSession => self.observed_sessions += 1,
            TraceCollection::ControlledReplay => self.controlled_replay_sessions += 1,
            TraceCollection::Unclassified => self.unclassified_sessions += 1,
        }
        self.baseline_tokens += observed.baseline.tokens;
        self.callsieve_tokens += observed.callsieve.tokens;
        self.avoided_grep_commands += observed.savings.avoided_grep_commands;
        self.avoided_file_reads += observed.savings.avoided_file_reads;

        let read_files: BTreeSet<&str> = observed
            .callsieve
            .files_read
            .iter()
            .map(String::as_str)
            .collect();
        let missing: Vec<String> = expected_files
            .into_iter()
            .filter(|file| !read_files.contains(file.as_str()))
            .collect();
        if !missing.is_empty() {
            self.files_still_missed += missing.len();
            self.missed_files.push(TraceMiss {
                id,
                task,
                files: missing,
            });
        }
    }

    fn add_summary(&mut self, summary: &TraceSummaryOutput) {
        self.sessions += summary.sessions;
        self.observed_sessions += summary.observed_sessions;
        self.controlled_replay_sessions += summary.controlled_replay_sessions;
        self.unclassified_sessions += summary.unclassified_sessions;
        self.baseline_tokens += summary.baseline_tokens;
        self.callsieve_tokens += summary.callsieve_tokens;
        self.avoided_grep_commands += summary.avoided_grep_commands;
        self.avoided_file_reads += summary.avoided_file_reads;
        self.files_still_missed += summary.files_still_missed;
        self.missed_files.extend(summary.missed_files.clone());
    }

    fn finish(self) -> Option<TraceSummaryOutput> {
        if self.sessions == 0 {
            return None;
        }
        Some(self.finish_output())
    }

    fn finish_output(self) -> TraceSummaryOutput {
        let token_savings = self.baseline_tokens as isize - self.callsieve_tokens as isize;
        let token_reduction_percent = if self.baseline_tokens == 0 {
            0.0
        } else {
            (token_savings as f64 / self.baseline_tokens as f64) * 100.0
        };

        TraceSummaryOutput {
            sessions: self.sessions,
            observed_sessions: self.observed_sessions,
            controlled_replay_sessions: self.controlled_replay_sessions,
            unclassified_sessions: self.unclassified_sessions,
            baseline_tokens: self.baseline_tokens,
            callsieve_tokens: self.callsieve_tokens,
            token_savings,
            token_reduction_percent,
            avoided_grep_commands: self.avoided_grep_commands,
            avoided_file_reads: self.avoided_file_reads,
            files_still_missed: self.files_still_missed,
            missed_files: self.missed_files,
        }
    }
}

fn trace_summary_from_tasks(
    tasks: Vec<TraceTaskInput>,
    collection: TraceCollection,
) -> TraceSummaryOutput {
    let mut accumulator = TraceAccumulator::default();

    for task in tasks {
        let TraceTaskInput {
            id,
            task,
            expected_files,
            observed,
            session,
        } = task;
        if let Some(observed) = session.or(observed) {
            accumulator.add_observed(
                id,
                task,
                expected_files,
                observed_session_output(observed),
                collection,
            );
        }
    }

    accumulator.finish_output()
}

fn observed_only_summary(summary: Option<TraceSummaryOutput>) -> Option<TraceSummaryOutput> {
    summary.filter(|summary| summary.observed_sessions > 0)
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
    stale_files(root, index)
        .into_iter()
        .take(20)
        .map(|file| format!("stale index entry: {file}"))
        .collect()
}

fn stale_files(root: &Path, index: &CodeIndex) -> Vec<String> {
    stale_file_details(root, index).paths
}

#[derive(Default)]
struct StaleFileDetails {
    paths: Vec<String>,
    changed: usize,
    removed: usize,
}

fn stale_file_details(root: &Path, index: &CodeIndex) -> StaleFileDetails {
    let mut details = StaleFileDetails::default();
    index.files.iter().for_each(|file| {
        let path = root.join(&file.path);
        let Ok(metadata) = fs::metadata(&path) else {
            details.paths.push(file.path.clone());
            details.removed += 1;
            return;
        };
        let mtime = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
            .unwrap_or_default();

        if metadata.len() != file.size_bytes || mtime != file.mtime {
            details.paths.push(file.path.clone());
            details.changed += 1;
        }
    });
    details
}

fn index_age(timestamp: u64) -> Option<u64> {
    if timestamp == 0 {
        return None;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    Some(now.saturating_sub(timestamp))
}

fn trace_tasks_from_value(value: serde_json::Value) -> Vec<serde_json::Value> {
    if let Some(tasks) = value.get("tasks").and_then(serde_json::Value::as_array) {
        return tasks.clone();
    }

    vec![value]
}

fn trace_command_strings(task: &serde_json::Value) -> Vec<String> {
    if let Some(commands) = task
        .get("session")
        .or_else(|| task.get("trace"))
        .or_else(|| task.get("observed"))
        .and_then(|session| session.get("callsieve"))
        .and_then(|callsieve| callsieve.get("commands"))
        .and_then(serde_json::Value::as_array)
    {
        return commands
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_string)
            .collect();
    }

    if let Some(commands) = task
        .get("callsieve")
        .and_then(|callsieve| callsieve.get("commands"))
        .and_then(serde_json::Value::as_array)
    {
        return commands
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_string)
            .collect();
    }

    let events = task
        .get("events")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let has_phases = events.iter().any(|event| event.get("phase").is_some());

    events
        .iter()
        .filter(|event| {
            !has_phases
                || event
                    .get("phase")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|phase| phase == "callsieve")
        })
        .filter_map(|event| event.get("command").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect()
}

fn trace_collection_from_value(value: &serde_json::Value) -> TraceCollection {
    let collection = value
        .get("metadata")
        .and_then(|metadata| metadata.get("collection"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    match collection {
        "observed_session" => TraceCollection::ObservedSession,
        "controlled_replay" | "context_first_session_start" => TraceCollection::ControlledReplay,
        _ => TraceCollection::Unclassified,
    }
}

fn string_array(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_string)
        .collect()
}

fn optional_string(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn manifest_path_values(value: &serde_json::Value, keys: &[&str]) -> Vec<String> {
    let mut paths = Vec::new();
    for key in keys {
        match value.get(*key) {
            Some(serde_json::Value::String(path)) => paths.push(path.clone()),
            Some(serde_json::Value::Array(values)) => {
                paths.extend(
                    values
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_string),
                );
            }
            _ => {}
        }
    }
    paths
}

fn repo_is_external(repo: &BenchmarkReportRepoInput) -> bool {
    if repo.external {
        return true;
    }

    repo.label
        .as_deref()
        .is_some_and(|label| label.starts_with("github-") || label.contains("external"))
        || repo
            .path
            .to_string_lossy()
            .replace('\\', "/")
            .contains("/github-")
}

fn codex_bootstrap_installed(root: &Path) -> bool {
    root.join(".codex/config.toml").is_file()
        && root.join(".codex/CALLSIEVE.md").is_file()
        && root.join(".callsieve/bin").is_dir()
        && root.join(".callsieve/codex-launch.ps1").is_file()
        && root.join(".callsieve/codex-launch.sh").is_file()
}

fn daemon_is_fresh(root: &Path) -> bool {
    let path = root.join(store::json_store::INDEX_DIR).join("daemon.json");
    let Ok(data) = fs::read(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&data) else {
        return false;
    };
    let stopped = value
        .get("status")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|status| matches!(status, "stopped" | "stop_requested" | "missing"));
    let indexed = value
        .get("last_indexed_at")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default()
        > 0;
    let errored = value
        .get("last_error")
        .is_some_and(|error| !error.is_null());
    indexed && !stopped && !errored
}

fn daemon_state_value(root: &Path) -> Option<serde_json::Value> {
    let path = root.join(store::json_store::INDEX_DIR).join("daemon.json");
    fs::read(path)
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
}

fn trace_has_controlled_replay_markers(value: &serde_json::Value) -> bool {
    let haystack = serde_json::to_string(value)
        .unwrap_or_default()
        .to_ascii_lowercase();
    haystack.contains("controlled local replay")
        || haystack.contains("deterministic local grep/read replay")
        || haystack.contains("baseline simulates grepping")
        || haystack.contains("callsieve codex-session")
}

fn is_grep_command(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let first = lower.split_whitespace().next().unwrap_or_default();
    matches!(first, "rg" | "grep" | "ripgrep")
        || lower.contains(" rg ")
        || lower.contains(" grep ")
        || lower.contains("ripgrep")
}

fn is_file_read_command(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let first = lower.split_whitespace().next().unwrap_or_default();
    matches!(
        first,
        "cat" | "less" | "more" | "head" | "tail" | "sed" | "nl" | "bat" | "type" | "get-content"
    ) || lower.contains(" get-content ")
        || lower.starts_with("read_file")
        || lower.contains(" read_file")
}

fn is_callsieve_context_command(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    lower.contains("callsieve context")
        || lower.contains("callsieve agent-context")
        || lower.contains("callsieve codex-session")
        || lower.contains("callsieve_context")
        || lower.contains("callsieve guard")
        || lower.contains("callsieve grep")
}

fn language_name(language: Language) -> &'static str {
    match language {
        Language::TypeScript => "typescript",
        Language::JavaScript => "javascript",
        Language::Python => "python",
        Language::Rust => "rust",
        Language::Markdown => "markdown",
        Language::Json => "json",
        Language::Toml => "toml",
        Language::Yaml => "yaml",
        Language::Text => "text",
    }
}

fn push_path_check(checks: &mut Vec<BenchmarkDoctorCheck>, path: &str, check: &str, passed: bool) {
    checks.push(BenchmarkDoctorCheck {
        path: if path.is_empty() {
            "<missing>".to_string()
        } else {
            path.to_string()
        },
        check: check.to_string(),
        status: if passed { "pass" } else { "fail" }.to_string(),
        message: if passed {
            "ok".to_string()
        } else {
            format!("{check} is missing or invalid")
        },
    });
}

fn push_parse_check(checks: &mut Vec<BenchmarkDoctorCheck>, path: &str, check: &str, passed: bool) {
    checks.push(BenchmarkDoctorCheck {
        path: path.to_string(),
        check: check.to_string(),
        status: if passed { "pass" } else { "fail" }.to_string(),
        message: if passed {
            "ok".to_string()
        } else {
            format!("{check} failed")
        },
    });
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
                        commands: vec!["rg createSession".to_string()],
                        files_read: vec!["src/auth/session.ts".to_string()],
                        notes: Vec::new(),
                    },
                    callsieve: ObservedSessionMetrics {
                        grep_commands: 1,
                        file_reads: 3,
                        tokens: 4_000,
                        commands: vec!["callsieve context".to_string()],
                        files_read: vec!["src/auth/session.ts".to_string()],
                        notes: Vec::new(),
                    },
                }),
                session: None,
            }],
        };

        let output = benchmark_suite(temp.path(), &index, suite, 8, 2, true).unwrap();

        assert_eq!(output.task_count, 1);
        assert_eq!(output.summary.expected_files, 2);
        assert_eq!(output.summary.expected_files_found, 2);
        assert_eq!(output.summary.expected_file_recall, 1.0);
        assert_eq!(output.summary.tasks_with_all_expected_files, 1);
        assert_eq!(output.summary.tasks_with_misses, 0);
        assert!(output.summary.total_estimated_avoided_grep_commands > 0);

        let observed = output.summary.observed_session.unwrap();
        assert_eq!(observed.sessions, 1);
        assert_eq!(observed.token_savings, 8_000);
        assert_eq!(observed.avoided_grep_commands, 5);
        assert_eq!(observed.avoided_file_reads, 6);
    }

    #[test]
    fn trace_summary_aggregates_sessions_and_missed_files() {
        let output = trace_summary_from_str(
            r#"{
  "tasks": [
    {
      "id": "auth-session",
      "task": "change auth session",
      "expected_files": ["src/auth/session.ts", "src/auth/token.ts"],
      "session": {
        "baseline": {
          "grep_commands": 6,
          "file_reads": 9,
          "tokens": 12000,
          "files_read": ["src/auth/session.ts", "src/auth/token.ts"]
        },
        "callsieve": {
          "grep_commands": 1,
          "file_reads": 3,
          "tokens": 4000,
          "files_read": ["src/auth/session.ts"]
        }
      }
    }
  ]
}"#,
        )
        .unwrap();

        assert_eq!(output.sessions, 1);
        assert_eq!(output.baseline_tokens, 12_000);
        assert_eq!(output.callsieve_tokens, 4_000);
        assert_eq!(output.token_savings, 8_000);
        assert_eq!(output.avoided_grep_commands, 5);
        assert_eq!(output.avoided_file_reads, 6);
        assert_eq!(output.files_still_missed, 1);
        assert_eq!(output.missed_files[0].files[0], "src/auth/token.ts");
    }

    #[test]
    fn benchmark_report_aggregates_multiple_local_repos() {
        let (repo_a, index_a) = fixture_index();
        crate::store::json_store::save_index(repo_a.path(), &index_a).unwrap();
        let suite_a = repo_a.path().join("tasks.json");
        write(
            &suite_a,
            r#"{"tasks":[{"id":"auth-a","task":"change createSession token behavior","expected_files":["src/auth/session.ts","src/auth/token.ts"]}]}"#,
        );

        let (repo_b, index_b) = fixture_index();
        crate::store::json_store::save_index(repo_b.path(), &index_b).unwrap();
        let suite_b = repo_b.path().join("tasks.json");
        write(
            &suite_b,
            r#"{"tasks":[{"id":"auth-b","task":"change tokenFor behavior","expected_files":["src/auth/token.ts"]}]}"#,
        );

        let manifest = BenchmarkReportManifest {
            thresholds: PilotThresholds::default(),
            repos: vec![
                BenchmarkReportRepoInput {
                    path: repo_a.path().to_path_buf(),
                    label: Some("repo-a".to_string()),
                    languages: Vec::new(),
                    external: false,
                    suite_path: Some(suite_a),
                    suite_paths: Vec::new(),
                    trace_path: None,
                    trace_paths: Vec::new(),
                    thresholds: None,
                },
                BenchmarkReportRepoInput {
                    path: repo_b.path().to_path_buf(),
                    label: Some("repo-b".to_string()),
                    languages: Vec::new(),
                    external: false,
                    suite_path: Some(suite_b),
                    suite_paths: Vec::new(),
                    trace_path: None,
                    trace_paths: Vec::new(),
                    thresholds: None,
                },
            ],
        };

        let output = benchmark_report(manifest, 8, 2, true).unwrap();

        assert_eq!(output.repo_count, 2);
        assert_eq!(output.summary.repos, 2);
        assert_eq!(output.summary.tasks, 2);
        assert_eq!(output.summary.expected_files, 3);
        assert_eq!(output.summary.expected_files_found, 3);
        assert_eq!(output.summary.expected_file_recall, 1.0);
        assert!(output.summary.total_avoided_grep_commands > 0);
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
