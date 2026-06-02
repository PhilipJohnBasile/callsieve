pub mod formatter;
pub mod ranker;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::{
    indexer::{SCHEMA_VERSION, language::Language},
    store::{self, CodeIndex, FileRecord, ImportRecord, ReferenceRecord, SymbolRecord},
};

const MAX_CONTEXT_SYMBOLS_PER_FILE: usize = 4;
const MAX_CONTEXT_WHY: usize = 6;
const MAX_CONTEXT_RELATION_FILES: usize = 5;
const MAX_CONTEXT_GRAPH_EDGES: usize = 1;
const MAX_CONTEXT_RELATED_TESTS: usize = 3;
const MAX_CONTEXT_RELATED_TEST_SYMBOLS: usize = 5;
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
    timing: TimingStats,
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
    why_debug: Vec<ranker::ScoreComponent>,
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

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct TimingStats {
    pub index_load_ms: u64,
    pub ranking_ms: u64,
    pub graph_expansion_ms: u64,
    pub snippet_ms: u64,
    pub total_ms: u64,
}

#[derive(Debug, Serialize)]
pub struct ContextOutput {
    task: String,
    root: String,
    read_first: Vec<ContextFile>,
    stats: ContextStats,
    timing: TimingStats,
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    why_debug: Vec<ranker::ScoreComponent>,
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

impl QueryOutput {
    pub fn add_index_load_time(&mut self, index_load_ms: u64) {
        self.timing.index_load_ms = index_load_ms;
        self.timing.total_ms = self.timing.total_ms.saturating_add(index_load_ms);
    }
}

impl ContextOutput {
    pub fn add_index_load_time(&mut self, index_load_ms: u64) {
        self.timing.index_load_ms = index_load_ms;
        self.timing.total_ms = self.timing.total_ms.saturating_add(index_load_ms);
    }
}

#[derive(Debug, Serialize)]
pub struct BenchmarkOutput {
    task: String,
    root: String,
    estimator: String,
    baseline: BaselineBenchmark,
    callsieve: CallsieveBenchmark,
    context_payload_reduction: ContextPayloadReduction,
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

#[derive(Debug, Serialize, Clone)]
struct ContextPayloadReduction {
    label: &'static str,
    evidence_tier: &'static str,
    platform_scope: &'static str,
    baseline_context_payload_tokens_estimate: usize,
    callsieve_context_payload_tokens_estimate: usize,
    context_payload_tokens_saved_estimate: isize,
    context_payload_reduction_ratio: f64,
    context_payload_reduction_percent: f64,
    estimator: &'static str,
    warning: &'static str,
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
    #[serde(default)]
    critical_files: Vec<String>,
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
    baseline_context_payload_tokens_estimate: usize,
    callsieve_context_payload_tokens_estimate: usize,
    context_payload_reduction: ContextPayloadReduction,
    total_estimated_token_savings: isize,
    average_estimated_token_reduction_percent: f64,
    total_estimated_avoided_grep_commands: usize,
    total_estimated_avoided_file_reads: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    misses: Vec<BenchmarkSuiteMiss>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_session: Option<ObservedSessionSummary>,
}

#[derive(Debug, Serialize)]
pub struct EvalRetrievalOutput {
    command: &'static str,
    status: String,
    root: String,
    limit: usize,
    snippets_per_file: usize,
    task_count: usize,
    tasks: Vec<EvalRetrievalTaskOutput>,
    summary: EvalRetrievalSummary,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

impl EvalRetrievalOutput {
    pub fn failed(&self) -> bool {
        self.status == "fail"
    }
}

#[derive(Debug, Serialize)]
struct EvalRetrievalTaskOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    task: String,
    status: String,
    expected_files: Vec<String>,
    critical_files: Vec<String>,
    selected_files: Vec<String>,
    expected_files_found: Vec<String>,
    expected_files_missing: Vec<String>,
    critical_files_found: Vec<String>,
    critical_files_missing: Vec<String>,
    recall_at_k: f64,
    critical_recall: f64,
    selected_tokens: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    failure_reasons: Vec<String>,
}

#[derive(Debug, Serialize)]
struct EvalRetrievalSummary {
    task_count: usize,
    passed_tasks: usize,
    failed_tasks: usize,
    expected_files: usize,
    expected_files_found: usize,
    missed_expected_files: usize,
    recall_at_k: f64,
    critical_files: usize,
    critical_files_found: usize,
    missed_critical_files: usize,
    critical_recall: f64,
    selected_tokens: usize,
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
    protocol: Option<String>,
    #[serde(default)]
    thresholds: PilotThresholds,
    #[serde(default)]
    audit: ProofAuditInput,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ProofAuditInput {
    #[serde(default)]
    planned_tasks: usize,
    #[serde(default)]
    rejected_sessions: usize,
    #[serde(default)]
    token_accounting_sources: Vec<String>,
    #[serde(default)]
    product_market: ProductMarketInput,
    #[serde(default)]
    scale_validation: ScaleValidationInput,
}

#[derive(Debug, Clone, Deserialize)]
struct BenchmarkReportRepoInput {
    path: PathBuf,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    team: Option<String>,
    #[serde(default)]
    languages: Vec<String>,
    #[serde(default)]
    clients: Vec<String>,
    #[serde(default)]
    task_categories: Vec<String>,
    #[serde(default)]
    scale_class: Option<String>,
    #[serde(default)]
    scale_criteria: Vec<String>,
    #[serde(default)]
    proof_tier: Option<String>,
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
    #[serde(default, alias = "policy_trace", alias = "policy_trace_path")]
    policy_trace_path: Option<PathBuf>,
    #[serde(default, alias = "policy_traces", alias = "policy_trace_paths")]
    policy_trace_paths: Vec<PathBuf>,
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

    fn policy_trace_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if let Some(path) = &self.policy_trace_path {
            paths.push(path.clone());
        }
        paths.extend(self.policy_trace_paths.clone());
        if paths.is_empty() {
            return self.trace_paths();
        }
        paths
    }

    fn thresholds<'a>(&'a self, manifest: &'a PilotThresholds) -> &'a PilotThresholds {
        self.thresholds.as_ref().unwrap_or(manifest)
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct ProductMarketInput {
    #[serde(default)]
    teams_completed_pilots: usize,
    #[serde(default, alias = "paid_or_converted_teams")]
    paid_pilot_or_converted_teams: usize,
    #[serde(default, alias = "teams_with_20_sessions")]
    teams_with_20_plus_sessions: usize,
    #[serde(default, alias = "disappointed_teams")]
    meaningfully_worse_without_teams: usize,
    #[serde(default, alias = "case_study_teams")]
    quote_approved_case_study_teams: usize,
    #[serde(default, alias = "renewal_or_loi_teams")]
    renewal_expansion_or_loi_teams: usize,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct ScaleValidationInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent_context_p95_latency_ms: Option<f64>,
    #[serde(default)]
    index_failures: usize,
    #[serde(default)]
    stale_index_failures: usize,
    #[serde(default)]
    crashes: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PilotThresholds {
    #[serde(default = "default_min_recall")]
    minimum_recall: f64,
    #[serde(default = "default_min_token_reduction_percent")]
    minimum_token_reduction_percent: f64,
    #[serde(default)]
    minimum_repos: usize,
    #[serde(default)]
    minimum_observed_sessions: usize,
    #[serde(default)]
    minimum_external_repos: usize,
    #[serde(default)]
    minimum_scale_proxy_repos: usize,
    #[serde(default)]
    minimum_clients: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    required_clients: Vec<String>,
    #[serde(default)]
    minimum_languages: usize,
    #[serde(default)]
    minimum_task_categories: usize,
    #[serde(default = "default_min_observed_token_reduction_percent")]
    minimum_observed_token_reduction_percent: f64,
    #[serde(default)]
    minimum_positive_savings_session_percent: f64,
    #[serde(default)]
    minimum_sessions_over_30_percent_savings_percent: f64,
    #[serde(default = "default_maximum_controlled_replay_ratio")]
    maximum_controlled_replay_ratio: f64,
    #[serde(default)]
    maximum_trace_violations: usize,
    #[serde(default)]
    maximum_critical_misses: usize,
    #[serde(default)]
    minimum_planned_tasks: usize,
    #[serde(default)]
    require_fresh_index: bool,
    #[serde(default)]
    require_lsp_where_available: bool,
    #[serde(default)]
    require_codex_bootstrap: bool,
    #[serde(default)]
    require_transcript_token_accounting: bool,
    #[serde(default, alias = "minimum_paid_pilot_teams")]
    minimum_pilot_teams: usize,
    #[serde(default)]
    minimum_paid_or_converted_teams: usize,
    #[serde(default)]
    minimum_teams_with_20_sessions: usize,
    #[serde(default)]
    minimum_meaningfully_worse_without_teams: usize,
    #[serde(default)]
    minimum_case_study_teams: usize,
    #[serde(default)]
    minimum_renewal_or_loi_teams: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    maximum_agent_context_p95_latency_ms: Option<f64>,
    #[serde(default)]
    maximum_scale_index_failures: usize,
    #[serde(default)]
    maximum_stale_index_failures: usize,
    #[serde(default)]
    maximum_scale_crashes: usize,
}

impl Default for PilotThresholds {
    fn default() -> Self {
        Self {
            minimum_recall: default_min_recall(),
            minimum_token_reduction_percent: default_min_token_reduction_percent(),
            minimum_repos: 0,
            minimum_observed_sessions: 0,
            minimum_external_repos: 0,
            minimum_scale_proxy_repos: 0,
            minimum_clients: 0,
            required_clients: Vec::new(),
            minimum_languages: 0,
            minimum_task_categories: 0,
            minimum_observed_token_reduction_percent: default_min_observed_token_reduction_percent(
            ),
            minimum_positive_savings_session_percent: 0.0,
            minimum_sessions_over_30_percent_savings_percent: 0.0,
            maximum_controlled_replay_ratio: default_maximum_controlled_replay_ratio(),
            maximum_trace_violations: 0,
            maximum_critical_misses: 0,
            minimum_planned_tasks: 0,
            require_fresh_index: false,
            require_lsp_where_available: false,
            require_codex_bootstrap: false,
            require_transcript_token_accounting: false,
            minimum_pilot_teams: 0,
            minimum_paid_or_converted_teams: 0,
            minimum_teams_with_20_sessions: 0,
            minimum_meaningfully_worse_without_teams: 0,
            minimum_case_study_teams: 0,
            minimum_renewal_or_loi_teams: 0,
            maximum_agent_context_p95_latency_ms: None,
            maximum_scale_index_failures: 0,
            maximum_stale_index_failures: 0,
            maximum_scale_crashes: 0,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    team: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    languages: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    clients: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    task_categories: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scale_class: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    scale_criteria: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    proof_tier: Option<String>,
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
    baseline_context_payload_tokens_estimate: usize,
    callsieve_context_payload_tokens_estimate: usize,
    context_payload_reduction: ContextPayloadReduction,
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
    baseline_context_payload_tokens_estimate: usize,
    callsieve_context_payload_tokens_estimate: usize,
    context_payload_reduction: ContextPayloadReduction,
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
    planned_tasks: usize,
    rejected_sessions: usize,
    token_accounting_sources: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    protocol: Option<String>,
    observed_sessions: usize,
    controlled_replay_sessions: usize,
    unclassified_sessions: usize,
    external_repos: usize,
    scale_proxy_repos: usize,
    scale_classes: Vec<BreakdownCount>,
    clients: Vec<String>,
    languages: Vec<String>,
    task_categories: Vec<String>,
    expected_file_recall: f64,
    token_reduction_percent: f64,
    observed_token_reduction_percent: f64,
    positive_savings_sessions: usize,
    positive_savings_session_percent: f64,
    sessions_over_30_percent_savings: usize,
    sessions_over_30_percent_savings_percent: f64,
    controlled_replay_ratio: f64,
    token_savings: isize,
    avoided_grep_commands: usize,
    avoided_file_reads: usize,
    trace_policy_violations: usize,
    critical_files_still_missed: usize,
    transcript_token_accounting_sessions: usize,
    transcript_token_accounting_percent: f64,
    per_client: Vec<BreakdownCount>,
    per_scale_class: Vec<BreakdownCount>,
    per_task_category: Vec<BreakdownCount>,
    product_market: ProductMarketInput,
    scale_validation: ScaleValidationInput,
    fresh_indexes: usize,
    daemon_fresh_repos: usize,
    lsp_enriched_repos: usize,
    lsp_available_repos: usize,
    codex_bootstrap_repos: usize,
}

#[derive(Debug, Serialize)]
struct BreakdownCount {
    name: String,
    count: usize,
}

#[derive(Debug, Serialize)]
struct PilotRepoOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    team: Option<String>,
    languages: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    clients: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    task_categories: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scale_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    proof_tier: Option<String>,
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
    #[serde(default)]
    critical_files: Vec<String>,
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
    critical_files_still_missed: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    missed_files: Vec<TraceMiss>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    critical_missed_files: Vec<TraceMiss>,
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    critical_files: Vec<String>,
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
    critical_files: Vec<String>,
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
    why_debug: Vec<ranker::ScoreComponent>,
    seen_debug: BTreeSet<String>,
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
            why_debug: Vec::new(),
            seen_debug: BTreeSet::new(),
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

    fn add_match(
        &mut self,
        score: i32,
        symbol_id: Option<&str>,
        why: &[String],
        score_debug: &[ranker::ScoreComponent],
    ) {
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

        for component in score_debug {
            self.push_debug(component.clone());
        }
    }

    fn add_graph_boost(&mut self, name: &'static str, score: i32, confidence: f64, why: String) {
        self.graph_confidence = self.graph_confidence.max(confidence);
        if self.seen_why.insert(why.clone()) {
            self.graph_score = (self.graph_score + score).min(MAX_CONTEXT_GRAPH_SCORE);
            self.why.push(why.clone());
            self.push_debug(ranker::ScoreComponent {
                name: name.to_string(),
                points: score,
                detail: why,
            });
        }
    }

    fn push_debug(&mut self, component: ranker::ScoreComponent) {
        let key = format!(
            "{}:{}:{}",
            component.name, component.points, component.detail
        );
        if self.seen_debug.insert(key) {
            self.why_debug.push(component);
        }
    }
}

struct IndexLookup<'a> {
    files_by_id: BTreeMap<&'a str, &'a FileRecord>,
    files_by_path: BTreeMap<&'a str, &'a FileRecord>,
    symbols_by_id: BTreeMap<&'a str, &'a SymbolRecord>,
    symbols_by_file: BTreeMap<&'a str, Vec<&'a SymbolRecord>>,
    imports_by_source: BTreeMap<&'a str, Vec<&'a ImportRecord>>,
    imports_by_resolved: BTreeMap<&'a str, Vec<&'a ImportRecord>>,
    references_by_source_path: BTreeMap<&'a str, Vec<&'a ReferenceRecord>>,
    references_by_target_path: BTreeMap<&'a str, Vec<&'a ReferenceRecord>>,
    references_by_source_symbol: BTreeMap<&'a str, Vec<&'a ReferenceRecord>>,
    references_by_target_symbol: BTreeMap<&'a str, Vec<&'a ReferenceRecord>>,
    test_files: Vec<&'a FileRecord>,
}

impl<'a> IndexLookup<'a> {
    fn new(index: &'a CodeIndex) -> Self {
        let mut lookup = Self {
            files_by_id: BTreeMap::new(),
            files_by_path: BTreeMap::new(),
            symbols_by_id: BTreeMap::new(),
            symbols_by_file: BTreeMap::new(),
            imports_by_source: BTreeMap::new(),
            imports_by_resolved: BTreeMap::new(),
            references_by_source_path: BTreeMap::new(),
            references_by_target_path: BTreeMap::new(),
            references_by_source_symbol: BTreeMap::new(),
            references_by_target_symbol: BTreeMap::new(),
            test_files: Vec::new(),
        };

        for file in &index.files {
            lookup.files_by_id.insert(file.id.as_str(), file);
            lookup.files_by_path.insert(file.path.as_str(), file);
            if file.is_test {
                lookup.test_files.push(file);
            }
        }

        for symbol in &index.symbols {
            lookup.symbols_by_id.insert(symbol.id.as_str(), symbol);
            lookup
                .symbols_by_file
                .entry(symbol.file_id.as_str())
                .or_default()
                .push(symbol);
        }

        for import in &index.imports {
            lookup
                .imports_by_source
                .entry(import.source_path.as_str())
                .or_default()
                .push(import);
            if let Some(resolved_path) = import.resolved_path.as_deref() {
                lookup
                    .imports_by_resolved
                    .entry(resolved_path)
                    .or_default()
                    .push(import);
            }
        }

        for reference in &index.references {
            lookup
                .references_by_source_path
                .entry(reference.source_path.as_str())
                .or_default()
                .push(reference);
            if let Some(target_path) = reference.target_path.as_deref() {
                lookup
                    .references_by_target_path
                    .entry(target_path)
                    .or_default()
                    .push(reference);
            }
            if let Some(source_symbol_id) = reference.source_symbol_id.as_deref() {
                lookup
                    .references_by_source_symbol
                    .entry(source_symbol_id)
                    .or_default()
                    .push(reference);
            }
            if let Some(target_symbol_id) = reference.target_symbol_id.as_deref() {
                lookup
                    .references_by_target_symbol
                    .entry(target_symbol_id)
                    .or_default()
                    .push(reference);
            }
        }

        lookup
    }

    fn file_by_id(&self, file_id: &str) -> Option<&'a FileRecord> {
        self.files_by_id.get(file_id).copied()
    }

    fn file_by_path(&self, path: &str) -> Option<&'a FileRecord> {
        self.files_by_path.get(path).copied()
    }

    fn symbol_by_id(&self, symbol_id: &str) -> Option<&'a SymbolRecord> {
        self.symbols_by_id.get(symbol_id).copied()
    }

    fn symbols_for_file(&self, file_id: &str) -> &[&'a SymbolRecord] {
        self.symbols_by_file
            .get(file_id)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn imports_from_path(&self, path: &str) -> &[&'a ImportRecord] {
        self.imports_by_source
            .get(path)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn imports_to_path(&self, path: &str) -> &[&'a ImportRecord] {
        self.imports_by_resolved
            .get(path)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn references_from_path(&self, path: &str) -> &[&'a ReferenceRecord] {
        self.references_by_source_path
            .get(path)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn references_to_path(&self, path: &str) -> &[&'a ReferenceRecord] {
        self.references_by_target_path
            .get(path)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn references_from_symbol(&self, symbol_id: &str) -> &[&'a ReferenceRecord] {
        self.references_by_source_symbol
            .get(symbol_id)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn references_to_symbol(&self, symbol_id: &str) -> &[&'a ReferenceRecord] {
        self.references_by_target_symbol
            .get(symbol_id)
            .map(Vec::as_slice)
            .unwrap_or_default()
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
    let lookup = IndexLookup::new(index);
    let symbols = index
        .symbols
        .iter()
        .take(limit)
        .filter_map(|symbol| {
            lookup
                .file_by_id(&symbol.file_id)
                .map(|file| SymbolListItem {
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
    let lookup = IndexLookup::new(index);
    let symbol_name_lower = symbol_name.to_ascii_lowercase();
    let mut matches: Vec<&SymbolRecord> = index
        .symbols
        .iter()
        .filter(|symbol| {
            symbol.name.eq_ignore_ascii_case(symbol_name)
                || symbol
                    .name
                    .to_ascii_lowercase()
                    .contains(&symbol_name_lower)
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
            let file = lookup.file_by_id(&symbol.file_id)?;
            Some(SymbolDetail {
                file: file.path.clone(),
                name: symbol.name.clone(),
                kind: symbol.kind.clone(),
                language: symbol.language,
                lines: [symbol.start_line, symbol.end_line],
                visibility: symbol.visibility.clone(),
                signature: symbol.signature.clone(),
                imports: imports_for_file(&lookup, &file.path),
                referenced_by: references_to_file(&lookup, &file.path),
                calls: calls_from_symbol(&lookup, symbol),
                references: references_from_symbol(&lookup, symbol),
                called_by: called_by_symbol(&lookup, symbol),
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
    run_query_with_options(root, index, question, limit, include_snippets, false)
}

pub fn run_query_with_options(
    root: &Path,
    index: &CodeIndex,
    question: &str,
    limit: usize,
    include_snippets: bool,
    why_debug: bool,
) -> Result<QueryOutput> {
    let total_start = Instant::now();
    let lookup = IndexLookup::new(index);
    let ranking_start = Instant::now();
    let ranked = ranker::rank(index, question, limit);
    let ranking_ms = elapsed_ms(ranking_start.elapsed());
    let matched_files = ranked
        .iter()
        .map(|match_| match_.file_id.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    let matched_symbols = ranked
        .iter()
        .filter(|match_| match_.symbol_id.is_some())
        .count();
    let mut snippet_elapsed = Duration::ZERO;

    let matches = ranked
        .into_iter()
        .enumerate()
        .filter_map(|(rank_index, ranked)| {
            let file = lookup.file_by_id(&ranked.file_id)?;
            let symbol = ranked
                .symbol_id
                .as_deref()
                .and_then(|symbol_id| lookup.symbol_by_id(symbol_id));

            let snippet = if include_snippets {
                let snippet_start = Instant::now();
                let snippet = snippet_for(root, file, symbol);
                snippet_elapsed += snippet_start.elapsed();
                snippet
            } else {
                None
            };
            let score_debug = if why_debug {
                ranked.score_debug
            } else {
                Vec::new()
            };

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
                why_debug: score_debug,
                related_tests: related_tests(&lookup, file),
            })
        })
        .collect();

    Ok(QueryOutput {
        query: question.to_string(),
        root: root_label(root),
        matches,
        stats: QueryStats {
            searched_files: index.files.len(),
            matched_files,
            matched_symbols,
        },
        timing: TimingStats {
            index_load_ms: 0,
            ranking_ms,
            graph_expansion_ms: 0,
            snippet_ms: elapsed_ms(snippet_elapsed),
            total_ms: elapsed_ms(total_start.elapsed()),
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
    build_context_with_options(
        root,
        index,
        task,
        limit,
        snippets_per_file,
        include_snippets,
        false,
    )
}

pub fn build_context_with_options(
    root: &Path,
    index: &CodeIndex,
    task: &str,
    limit: usize,
    snippets_per_file: usize,
    include_snippets: bool,
    why_debug: bool,
) -> Result<ContextOutput> {
    let total_start = Instant::now();
    let candidate_limit = limit.saturating_mul(16);
    let lookup = IndexLookup::new(index);
    let ranking_start = Instant::now();
    let ranked = ranker::rank(index, task, candidate_limit);
    let ranking_ms = elapsed_ms(ranking_start.elapsed());
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
            &ranked_match.score_debug,
        );
    }
    let graph_start = Instant::now();
    add_graph_context(&lookup, &ranked, &mut grouped);
    add_reference_context(&lookup, &ranked, &mut grouped);
    let graph_expansion_ms = elapsed_ms(graph_start.elapsed());

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
    let mut snippet_elapsed = Duration::ZERO;
    let read_first: Vec<ContextFile> = candidates
        .into_iter()
        .take(limit)
        .enumerate()
        .filter_map(|(rank_index, candidate)| {
            let file = lookup.file_by_id(&candidate.file_id)?;
            let symbol_records: Vec<&SymbolRecord> = candidate
                .symbol_ids
                .iter()
                .filter_map(|symbol_id| lookup.symbol_by_id(symbol_id))
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

            let snippet_start = Instant::now();
            let snippets = context_snippets(
                root,
                file,
                &symbol_records,
                snippets_per_file,
                include_snippets,
            );
            snippet_elapsed += snippet_start.elapsed();
            let related_tests_all = related_tests(&lookup, file);
            let imports_all = resolved_imports_for_file(&lookup, &file.path);
            let referenced_by_all = references_to_file(&lookup, &file.path);
            let calls_all = calls_from_file(&lookup, file);
            let called_by_all = called_by_file(&lookup, file);
            let blast_radius = blast_radius_for(
                &imports_all,
                &referenced_by_all,
                &related_tests_all,
                &calls_all,
                &called_by_all,
            );
            let imports = take_strings(imports_all, MAX_CONTEXT_RELATION_FILES);
            let referenced_by = take_strings(referenced_by_all, MAX_CONTEXT_RELATION_FILES);
            let calls = calls_all
                .into_iter()
                .take(MAX_CONTEXT_GRAPH_EDGES)
                .collect();
            let called_by = called_by_all
                .into_iter()
                .take(MAX_CONTEXT_GRAPH_EDGES)
                .collect();
            let related_tests = compact_related_tests(related_tests_all);
            let score = candidate.score();
            let why = take_strings(candidate.why, MAX_CONTEXT_WHY);
            let debug = if why_debug {
                candidate.why_debug.into_iter().take(16).collect()
            } else {
                Vec::new()
            };

            selected_symbols += symbols.len();
            selected_related_tests += related_tests.len();

            Some(ContextFile {
                rank: rank_index + 1,
                score,
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
                why,
                why_debug: debug,
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
        timing: TimingStats {
            index_load_ms: 0,
            ranking_ms,
            graph_expansion_ms,
            snippet_ms: elapsed_ms(snippet_elapsed),
            total_ms: elapsed_ms(total_start.elapsed()),
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
    let context_payload_reduction = context_payload_reduction(
        baseline.estimated_total_tokens,
        callsieve.estimated_packet_tokens,
    );

    Ok(BenchmarkOutput {
        task: task.to_string(),
        root: root_label(root),
        estimator: "local deterministic token estimate".to_string(),
        baseline,
        callsieve,
        context_payload_reduction,
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
    let mut total_baseline_context_payload_tokens = 0;
    let mut total_callsieve_context_payload_tokens = 0;
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
            critical_files: _,
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
        total_baseline_context_payload_tokens += benchmark.baseline.estimated_total_tokens;
        total_callsieve_context_payload_tokens += benchmark.callsieve.estimated_packet_tokens;
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
        baseline_context_payload_tokens_estimate: total_baseline_context_payload_tokens,
        callsieve_context_payload_tokens_estimate: total_callsieve_context_payload_tokens,
        context_payload_reduction: context_payload_reduction(
            total_baseline_context_payload_tokens,
            total_callsieve_context_payload_tokens,
        ),
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

pub fn eval_retrieval(
    root: &Path,
    index: &CodeIndex,
    suite: BenchmarkSuiteInput,
    limit: usize,
    snippets_per_file: usize,
    include_snippets: bool,
) -> Result<EvalRetrievalOutput> {
    let mut task_outputs = Vec::new();
    let mut total_expected_files = 0;
    let mut total_expected_files_found = 0;
    let mut total_critical_files = 0;
    let mut total_critical_files_found = 0;
    let mut total_selected_tokens = 0;

    for task in suite.tasks {
        let BenchmarkSuiteTaskInput {
            id,
            task,
            expected_files,
            critical_files,
            observed: _,
            session: _,
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
        let expected_files_found: Vec<String> = expected_files
            .iter()
            .filter(|file| selected_set.contains(file.as_str()))
            .cloned()
            .collect();
        let expected_files_missing: Vec<String> = expected_files
            .iter()
            .filter(|file| !selected_set.contains(file.as_str()))
            .cloned()
            .collect();
        let critical_files_found: Vec<String> = critical_files
            .iter()
            .filter(|file| selected_set.contains(file.as_str()))
            .cloned()
            .collect();
        let critical_files_missing: Vec<String> = critical_files
            .iter()
            .filter(|file| !selected_set.contains(file.as_str()))
            .cloned()
            .collect();
        let enforced_missing = if critical_files.is_empty() {
            &expected_files_missing
        } else {
            &critical_files_missing
        };
        let mut failure_reasons =
            miss_reasons_for(index, enforced_missing, &selected_files, &benchmark);
        if !critical_files_missing.is_empty() {
            failure_reasons.push(format!(
                "critical files missed: {}",
                critical_files_missing.join(", ")
            ));
        }
        let status = if enforced_missing.is_empty() {
            "pass"
        } else {
            "fail"
        }
        .to_string();
        let selected_tokens = benchmark.callsieve.estimated_packet_tokens;

        total_expected_files += expected_files.len();
        total_expected_files_found += expected_files_found.len();
        total_critical_files += critical_files.len();
        total_critical_files_found += critical_files_found.len();
        total_selected_tokens += selected_tokens;

        task_outputs.push(EvalRetrievalTaskOutput {
            id,
            task,
            status,
            expected_files: expected_files.clone(),
            critical_files: critical_files.clone(),
            selected_files,
            expected_files_found,
            expected_files_missing,
            critical_files_found,
            critical_files_missing,
            recall_at_k: recall(
                expected_files
                    .iter()
                    .filter(|file| {
                        benchmark
                            .callsieve
                            .top_files
                            .iter()
                            .any(|selected| selected.file == **file)
                    })
                    .count(),
                expected_files.len(),
            ),
            critical_recall: recall(
                critical_files
                    .iter()
                    .filter(|file| {
                        benchmark
                            .callsieve
                            .top_files
                            .iter()
                            .any(|selected| selected.file == **file)
                    })
                    .count(),
                critical_files.len(),
            ),
            selected_tokens,
            failure_reasons,
        });
    }

    let failed_tasks = task_outputs
        .iter()
        .filter(|task| task.status == "fail")
        .count();
    let task_count = task_outputs.len();
    let summary = EvalRetrievalSummary {
        task_count,
        passed_tasks: task_count.saturating_sub(failed_tasks),
        failed_tasks,
        expected_files: total_expected_files,
        expected_files_found: total_expected_files_found,
        missed_expected_files: total_expected_files.saturating_sub(total_expected_files_found),
        recall_at_k: recall(total_expected_files_found, total_expected_files),
        critical_files: total_critical_files,
        critical_files_found: total_critical_files_found,
        missed_critical_files: total_critical_files.saturating_sub(total_critical_files_found),
        critical_recall: recall(total_critical_files_found, total_critical_files),
        selected_tokens: total_selected_tokens,
    };

    Ok(EvalRetrievalOutput {
        command: "eval-retrieval",
        status: if failed_tasks == 0 { "pass" } else { "fail" }.to_string(),
        root: root_label(root),
        limit,
        snippets_per_file,
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
            critical_files,
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
                critical_files,
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
            critical_files: Vec::new(),
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
    let mut total_baseline_context_payload_tokens = 0;
    let mut total_callsieve_context_payload_tokens = 0;
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
        let mut repo_baseline_context_payload_tokens = 0;
        let mut repo_callsieve_context_payload_tokens = 0;
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
            repo_baseline_context_payload_tokens +=
                output.summary.baseline_context_payload_tokens_estimate;
            repo_callsieve_context_payload_tokens +=
                output.summary.callsieve_context_payload_tokens_estimate;
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
        total_baseline_context_payload_tokens += repo_baseline_context_payload_tokens;
        total_callsieve_context_payload_tokens += repo_callsieve_context_payload_tokens;
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
            team: repo.team,
            languages: repo.languages,
            clients: repo.clients,
            task_categories: repo.task_categories,
            scale_class: repo.scale_class,
            scale_criteria: repo.scale_criteria,
            proof_tier: repo.proof_tier,
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
            baseline_context_payload_tokens_estimate: repo_baseline_context_payload_tokens,
            callsieve_context_payload_tokens_estimate: repo_callsieve_context_payload_tokens,
            context_payload_reduction: context_payload_reduction(
                repo_baseline_context_payload_tokens,
                repo_callsieve_context_payload_tokens,
            ),
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
        baseline_context_payload_tokens_estimate: total_baseline_context_payload_tokens,
        callsieve_context_payload_tokens_estimate: total_callsieve_context_payload_tokens,
        context_payload_reduction: context_payload_reduction(
            total_baseline_context_payload_tokens,
            total_callsieve_context_payload_tokens,
        ),
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
    let mut client_set = BTreeSet::new();
    let mut task_category_set = BTreeSet::new();
    let mut scale_class_counts = BTreeMap::new();
    let mut observed_session_evidence = ObservedTraceEvidence::default();
    let mut trace_policy_violations = 0usize;
    let mut fresh_indexes = 0usize;
    let mut daemon_fresh_repos = 0usize;
    let mut lsp_enriched_repos = 0usize;
    let mut lsp_available_repos = 0usize;
    let mut codex_bootstrap_repos = 0usize;
    let mut external_repos = 0usize;
    let mut scale_proxy_repos = 0usize;
    let mut transcript_token_accounting_sessions = 0usize;
    let mut observed_trace_accumulator = TraceAccumulator::default();
    let mut controlled_trace_accumulator = TraceAccumulator::default();
    let mut unclassified_trace_accumulator = TraceAccumulator::default();

    for repo in &manifest.repos {
        let index = store::json_store::load_index(&repo.path).ok();
        let status = index_status(&repo.path, index.as_ref());
        let is_external = repo_is_external(repo);
        let repo_scale_class = repo_scale_class_name(repo);
        let codex_bootstrap = codex_bootstrap_installed(&repo.path);
        let daemon_fresh = daemon_is_fresh(&repo.path);
        if is_external {
            external_repos += 1;
        }
        if repo_is_scale_proxy(repo) {
            scale_proxy_repos += 1;
        }
        *scale_class_counts
            .entry(repo_scale_class.clone())
            .or_insert(0usize) += 1;
        for client in &repo.clients {
            client_set.insert(normalized_dimension(client));
        }
        for category in &repo.task_categories {
            task_category_set.insert(normalized_dimension(category));
        }
        for suite_path in repo.suite_paths() {
            for category in task_categories_from_suite(&suite_path)? {
                task_category_set.insert(normalized_dimension(&category));
            }
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
                    if trace_token_accounting_source(&trace_value) == "transcript_context_tokens" {
                        transcript_token_accounting_sessions += trace_summary.observed_sessions;
                    }
                    observed_session_evidence.add_trace(
                        &trace_value,
                        repo,
                        &repo_scale_class,
                        &mut client_set,
                        &mut task_category_set,
                    )?;
                    if repo
                        .thresholds(&manifest.thresholds)
                        .require_transcript_token_accounting
                        && trace_token_accounting_source(&trace_value)
                            != "transcript_context_tokens"
                    {
                        failures.push(PilotFailure {
                            label: repo.label.clone(),
                            path: repo.path.display().to_string(),
                            check: "require_transcript_token_accounting".to_string(),
                            message: format!(
                                "observed trace {} is missing transcript_context_tokens provenance",
                                trace_path.display()
                            ),
                        });
                    }
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
        }
        for trace_path in repo.policy_trace_paths() {
            let trace_json = fs::read_to_string(&trace_path)?;
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
            team: repo.team.clone(),
            languages,
            clients: repo.clients.clone(),
            task_categories: repo.task_categories.clone(),
            scale_class: repo.scale_class.clone(),
            proof_tier: repo.proof_tier.clone(),
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
    let critical_files_still_missed = observed_summary
        .as_ref()
        .map(|summary| summary.critical_files_still_missed)
        .unwrap_or_default()
        + controlled_summary
            .as_ref()
            .map(|summary| summary.critical_files_still_missed)
            .unwrap_or_default()
        + unclassified_summary
            .as_ref()
            .map(|summary| summary.critical_files_still_missed)
            .unwrap_or_default();
    let controlled_replay_ratio = if total_trace_sessions == 0 {
        0.0
    } else {
        controlled_replay_sessions as f64 / total_trace_sessions as f64
    };
    let positive_savings_session_percent = percent(
        observed_session_evidence.positive_savings_sessions,
        observed_session_evidence.sessions,
    );
    let sessions_over_30_percent_savings_percent = percent(
        observed_session_evidence.sessions_over_30_percent_savings,
        observed_session_evidence.sessions,
    );
    let transcript_token_accounting_percent =
        percent(transcript_token_accounting_sessions, observed_sessions);
    let clients: Vec<String> = client_set
        .into_iter()
        .filter(|client| !client.is_empty())
        .collect();
    let task_categories: Vec<String> = task_category_set
        .into_iter()
        .filter(|category| !category.is_empty())
        .collect();

    let proof = PilotProofSummary {
        repos: benchmark.summary.repos,
        sessions: total_trace_sessions,
        planned_tasks: manifest.audit.planned_tasks,
        rejected_sessions: manifest.audit.rejected_sessions,
        token_accounting_sources: manifest.audit.token_accounting_sources.clone(),
        protocol: manifest.protocol.clone(),
        observed_sessions,
        controlled_replay_sessions,
        unclassified_sessions,
        external_repos,
        scale_proxy_repos,
        scale_classes: breakdown_counts(scale_class_counts),
        clients,
        languages: language_set.into_iter().collect(),
        task_categories,
        expected_file_recall: benchmark.summary.expected_file_recall,
        token_reduction_percent: benchmark.summary.average_estimated_token_reduction_percent,
        observed_token_reduction_percent,
        positive_savings_sessions: observed_session_evidence.positive_savings_sessions,
        positive_savings_session_percent,
        sessions_over_30_percent_savings: observed_session_evidence
            .sessions_over_30_percent_savings,
        sessions_over_30_percent_savings_percent,
        controlled_replay_ratio,
        token_savings: benchmark.summary.total_estimated_token_savings,
        avoided_grep_commands: benchmark.summary.total_avoided_grep_commands,
        avoided_file_reads: benchmark.summary.total_avoided_file_reads,
        trace_policy_violations,
        critical_files_still_missed,
        transcript_token_accounting_sessions,
        transcript_token_accounting_percent,
        per_client: breakdown_counts(observed_session_evidence.client_sessions),
        per_scale_class: breakdown_counts(observed_session_evidence.scale_class_sessions),
        per_task_category: breakdown_counts(observed_session_evidence.task_category_sessions),
        product_market: manifest.audit.product_market.clone(),
        scale_validation: manifest.audit.scale_validation.clone(),
        fresh_indexes,
        daemon_fresh_repos,
        lsp_enriched_repos,
        lsp_available_repos,
        codex_bootstrap_repos,
    };
    let session_count = proof.sessions;
    if proof.repos < manifest.thresholds.minimum_repos {
        push_global_failure(
            &mut failures,
            "minimum_repos",
            format!(
                "repos {} are below threshold {}",
                proof.repos, manifest.thresholds.minimum_repos
            ),
        );
    }
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
    if proof.planned_tasks < manifest.thresholds.minimum_planned_tasks {
        failures.push(PilotFailure {
            label: None,
            path: ".".to_string(),
            check: "minimum_planned_tasks".to_string(),
            message: format!(
                "planned tasks {} are below threshold {}",
                proof.planned_tasks, manifest.thresholds.minimum_planned_tasks
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
    if proof.scale_proxy_repos < manifest.thresholds.minimum_scale_proxy_repos {
        push_global_failure(
            &mut failures,
            "minimum_scale_proxy_repos",
            format!(
                "scale proxy repos {} are below threshold {}",
                proof.scale_proxy_repos, manifest.thresholds.minimum_scale_proxy_repos
            ),
        );
    }
    let observed_clients: BTreeSet<String> = proof
        .per_client
        .iter()
        .map(|client| client.name.clone())
        .collect();
    if observed_clients.len() < manifest.thresholds.minimum_clients {
        push_global_failure(
            &mut failures,
            "minimum_clients",
            format!(
                "observed clients {} are below threshold {}",
                observed_clients.len(),
                manifest.thresholds.minimum_clients
            ),
        );
    }
    let missing_clients: Vec<String> = manifest
        .thresholds
        .required_clients
        .iter()
        .map(|client| normalized_dimension(client))
        .filter(|client| !observed_clients.contains(client))
        .collect();
    if !missing_clients.is_empty() {
        push_global_failure(
            &mut failures,
            "required_clients",
            format!("missing required clients: {}", missing_clients.join(", ")),
        );
    }
    if proof.languages.len() < manifest.thresholds.minimum_languages {
        push_global_failure(
            &mut failures,
            "minimum_languages",
            format!(
                "languages {} are below threshold {}",
                proof.languages.len(),
                manifest.thresholds.minimum_languages
            ),
        );
    }
    if proof.task_categories.len() < manifest.thresholds.minimum_task_categories {
        push_global_failure(
            &mut failures,
            "minimum_task_categories",
            format!(
                "task categories {} are below threshold {}",
                proof.task_categories.len(),
                manifest.thresholds.minimum_task_categories
            ),
        );
    }
    if manifest.thresholds.require_transcript_token_accounting
        && proof.transcript_token_accounting_sessions < proof.observed_sessions
    {
        failures.push(PilotFailure {
            label: None,
            path: ".".to_string(),
            check: "require_transcript_token_accounting".to_string(),
            message: format!(
                "transcript-token observed sessions {} are below observed sessions {}",
                proof.transcript_token_accounting_sessions, proof.observed_sessions
            ),
        });
    }
    if proof.positive_savings_session_percent
        < manifest.thresholds.minimum_positive_savings_session_percent
    {
        push_global_failure(
            &mut failures,
            "minimum_positive_savings_session_percent",
            format!(
                "positive-savings sessions {:.1}% are below threshold {:.1}%",
                proof.positive_savings_session_percent,
                manifest.thresholds.minimum_positive_savings_session_percent
            ),
        );
    }
    if proof.sessions_over_30_percent_savings_percent
        < manifest
            .thresholds
            .minimum_sessions_over_30_percent_savings_percent
    {
        push_global_failure(
            &mut failures,
            "minimum_sessions_over_30_percent_savings_percent",
            format!(
                "sessions above 30% savings {:.1}% are below threshold {:.1}%",
                proof.sessions_over_30_percent_savings_percent,
                manifest
                    .thresholds
                    .minimum_sessions_over_30_percent_savings_percent
            ),
        );
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
    if proof.product_market.teams_completed_pilots < manifest.thresholds.minimum_pilot_teams {
        push_global_failure(
            &mut failures,
            "minimum_pilot_teams",
            format!(
                "pilot teams {} are below threshold {}",
                proof.product_market.teams_completed_pilots,
                manifest.thresholds.minimum_pilot_teams
            ),
        );
    }
    if proof.product_market.paid_pilot_or_converted_teams
        < manifest.thresholds.minimum_paid_or_converted_teams
    {
        push_global_failure(
            &mut failures,
            "minimum_paid_or_converted_teams",
            format!(
                "paid or converted teams {} are below threshold {}",
                proof.product_market.paid_pilot_or_converted_teams,
                manifest.thresholds.minimum_paid_or_converted_teams
            ),
        );
    }
    if proof.product_market.teams_with_20_plus_sessions
        < manifest.thresholds.minimum_teams_with_20_sessions
    {
        push_global_failure(
            &mut failures,
            "minimum_teams_with_20_sessions",
            format!(
                "teams with 20+ sessions {} are below threshold {}",
                proof.product_market.teams_with_20_plus_sessions,
                manifest.thresholds.minimum_teams_with_20_sessions
            ),
        );
    }
    if proof.product_market.meaningfully_worse_without_teams
        < manifest.thresholds.minimum_meaningfully_worse_without_teams
    {
        push_global_failure(
            &mut failures,
            "minimum_meaningfully_worse_without_teams",
            format!(
                "meaningfully worse without teams {} are below threshold {}",
                proof.product_market.meaningfully_worse_without_teams,
                manifest.thresholds.minimum_meaningfully_worse_without_teams
            ),
        );
    }
    if proof.product_market.quote_approved_case_study_teams
        < manifest.thresholds.minimum_case_study_teams
    {
        push_global_failure(
            &mut failures,
            "minimum_case_study_teams",
            format!(
                "case-study teams {} are below threshold {}",
                proof.product_market.quote_approved_case_study_teams,
                manifest.thresholds.minimum_case_study_teams
            ),
        );
    }
    if proof.product_market.renewal_expansion_or_loi_teams
        < manifest.thresholds.minimum_renewal_or_loi_teams
    {
        push_global_failure(
            &mut failures,
            "minimum_renewal_or_loi_teams",
            format!(
                "renewal, expansion, or LOI teams {} are below threshold {}",
                proof.product_market.renewal_expansion_or_loi_teams,
                manifest.thresholds.minimum_renewal_or_loi_teams
            ),
        );
    }
    if let Some(max_latency) = manifest.thresholds.maximum_agent_context_p95_latency_ms {
        match proof.scale_validation.agent_context_p95_latency_ms {
            Some(actual) if actual <= max_latency => {}
            Some(actual) => push_global_failure(
                &mut failures,
                "maximum_agent_context_p95_latency_ms",
                format!(
                    "p95 agent-context latency {:.1}ms exceeds threshold {:.1}ms",
                    actual, max_latency
                ),
            ),
            None => push_global_failure(
                &mut failures,
                "maximum_agent_context_p95_latency_ms",
                "p95 agent-context latency is missing".to_string(),
            ),
        }
    }
    if proof.scale_validation.index_failures > manifest.thresholds.maximum_scale_index_failures {
        push_global_failure(
            &mut failures,
            "maximum_scale_index_failures",
            format!(
                "scale index failures {} exceed threshold {}",
                proof.scale_validation.index_failures,
                manifest.thresholds.maximum_scale_index_failures
            ),
        );
    }
    if proof.scale_validation.stale_index_failures
        > manifest.thresholds.maximum_stale_index_failures
    {
        push_global_failure(
            &mut failures,
            "maximum_stale_index_failures",
            format!(
                "stale index failures {} exceed threshold {}",
                proof.scale_validation.stale_index_failures,
                manifest.thresholds.maximum_stale_index_failures
            ),
        );
    }
    if proof.scale_validation.crashes > manifest.thresholds.maximum_scale_crashes {
        push_global_failure(
            &mut failures,
            "maximum_scale_crashes",
            format!(
                "scale validation crashes {} exceed threshold {}",
                proof.scale_validation.crashes, manifest.thresholds.maximum_scale_crashes
            ),
        );
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
    if proof.critical_files_still_missed > manifest.thresholds.maximum_critical_misses {
        failures.push(PilotFailure {
            label: None,
            path: ".".to_string(),
            check: "maximum_critical_misses".to_string(),
            message: format!(
                "critical misses {} exceed threshold {}",
                proof.critical_files_still_missed, manifest.thresholds.maximum_critical_misses
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

pub fn enterprise_proof_report(
    manifest: BenchmarkReportManifest,
    limit: usize,
    snippets_per_file: usize,
    include_snippets: bool,
) -> Result<ProofReportOutput> {
    let mut output = proof_report(manifest, limit, snippets_per_file, include_snippets)?;
    output.command = "enterprise-proof-report";
    output.claim = "Broad developer-session proof is gated on 1,000 observed paired sessions, multi-client coverage, scale-proxy repositories, strict trace policy, and paid-pilot PMF evidence. Do not claim broad coverage until status is pass.";
    Ok(output)
}

#[derive(Default)]
struct ObservedTraceEvidence {
    sessions: usize,
    positive_savings_sessions: usize,
    sessions_over_30_percent_savings: usize,
    client_sessions: BTreeMap<String, usize>,
    scale_class_sessions: BTreeMap<String, usize>,
    task_category_sessions: BTreeMap<String, usize>,
}

impl ObservedTraceEvidence {
    fn add_trace(
        &mut self,
        value: &serde_json::Value,
        repo: &BenchmarkReportRepoInput,
        scale_class: &str,
        client_set: &mut BTreeSet<String>,
        task_category_set: &mut BTreeSet<String>,
    ) -> Result<()> {
        let default_client = trace_metadata_string(value, "client")
            .or_else(|| repo.clients.first().cloned())
            .unwrap_or_else(|| "unknown".to_string());
        let default_category = task_category_from_value(value)
            .or_else(|| repo.task_categories.first().cloned())
            .unwrap_or_else(|| "unknown".to_string());
        let scale_class = normalized_dimension(scale_class);

        for task in trace_tasks_from_value(value.clone()) {
            let Some(comparison) = observed_comparison_from_task(&task)? else {
                continue;
            };
            let client = trace_metadata_string(&task, "client")
                .or_else(|| trace_metadata_string(value, "client"))
                .or_else(|| repo.clients.first().cloned())
                .unwrap_or_else(|| default_client.clone());
            let task_category = task_category_from_value(&task)
                .or_else(|| task_category_from_value(value))
                .or_else(|| repo.task_categories.first().cloned())
                .unwrap_or_else(|| default_category.clone());
            let client = normalized_or_unknown(&client);
            let task_category = normalized_or_unknown(&task_category);
            let observed = observed_session_output(comparison);

            self.sessions += 1;
            if observed.savings.token_savings > 0 {
                self.positive_savings_sessions += 1;
            }
            if observed.savings.token_reduction_percent > 30.0 {
                self.sessions_over_30_percent_savings += 1;
            }
            increment_count(&mut self.client_sessions, client.clone());
            increment_count(&mut self.scale_class_sessions, scale_class.clone());
            increment_count(&mut self.task_category_sessions, task_category.clone());
            client_set.insert(client);
            task_category_set.insert(task_category);
        }

        Ok(())
    }
}

fn observed_comparison_from_task(
    task: &serde_json::Value,
) -> Result<Option<ObservedSessionComparison>> {
    for key in ["session", "trace", "observed"] {
        if let Some(value) = task.get(key)
            && value.get("baseline").is_some()
            && value.get("callsieve").is_some()
        {
            return Ok(Some(serde_json::from_value(value.clone())?));
        }
    }

    if task.get("baseline").is_some() && task.get("callsieve").is_some() {
        return Ok(Some(serde_json::from_value(task.clone())?));
    }

    Ok(None)
}

fn task_categories_from_suite(path: &Path) -> Result<Vec<String>> {
    let json = fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&json)?;
    let mut categories = Vec::new();

    categories.extend(string_array(value.get("task_categories")));
    if let Some(tasks) = value.get("tasks").and_then(serde_json::Value::as_array) {
        for task in tasks {
            if let Some(category) = task_category_from_value(task) {
                categories.push(category);
            }
        }
    }

    categories.sort();
    categories.dedup();
    Ok(categories)
}

fn task_category_from_value(value: &serde_json::Value) -> Option<String> {
    optional_string(value.get("task_category"))
        .or_else(|| optional_string(value.get("category")))
        .or_else(|| trace_metadata_string(value, "task_category"))
        .or_else(|| trace_metadata_string(value, "category"))
}

fn trace_metadata_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get("metadata")
        .and_then(|metadata| metadata.get(key))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn repo_scale_class_name(repo: &BenchmarkReportRepoInput) -> String {
    repo.scale_class
        .as_deref()
        .or(repo.proof_tier.as_deref())
        .map(normalized_dimension)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            if repo.external {
                "external".to_string()
            } else {
                "local_harness".to_string()
            }
        })
}

fn repo_is_scale_proxy(repo: &BenchmarkReportRepoInput) -> bool {
    let scale_class = repo_scale_class_name(repo);
    scale_class.contains("scale_proxy")
        || scale_class.contains("microsoft_scale")
        || repo
            .scale_criteria
            .iter()
            .map(|criteria| normalized_dimension(criteria))
            .any(|criteria| {
                criteria.contains("1m_loc")
                    || criteria.contains("100k_files")
                    || criteria.contains("1000_modules")
                    || criteria.contains("monorepo")
                    || criteria.contains("enterprise")
            })
}

fn normalized_or_unknown(value: &str) -> String {
    let value = normalized_dimension(value);
    if value.is_empty() {
        "unknown".to_string()
    } else {
        value
    }
}

fn normalized_dimension(value: &str) -> String {
    let mut normalized = String::new();
    let mut last_separator = false;
    for ch in value.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch.to_ascii_lowercase());
            last_separator = false;
        } else if !last_separator {
            normalized.push('_');
            last_separator = true;
        }
    }
    normalized.trim_matches('_').to_string()
}

fn increment_count(counts: &mut BTreeMap<String, usize>, key: String) {
    *counts.entry(key).or_insert(0) += 1;
}

fn breakdown_counts(counts: BTreeMap<String, usize>) -> Vec<BreakdownCount> {
    counts
        .into_iter()
        .filter(|(name, _)| !name.is_empty())
        .map(|(name, count)| BreakdownCount { name, count })
        .collect()
}

fn percent(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        (numerator as f64 / denominator as f64) * 100.0
    }
}

fn push_global_failure(failures: &mut Vec<PilotFailure>, check: &str, message: String) {
    failures.push(PilotFailure {
        label: None,
        path: ".".to_string(),
        check: check.to_string(),
        message,
    });
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
        let critical_files = string_array(value.get("critical_files"));
        let observed: ObservedSessionComparison = serde_json::from_value(value.clone())?;
        return Ok(trace_summary_from_tasks(
            vec![TraceTaskInput {
                id: optional_string(value.get("id")),
                task: optional_string(value.get("task")),
                expected_files,
                critical_files,
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

fn add_graph_context(
    lookup: &IndexLookup<'_>,
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
        let Some(file) = lookup.file_by_id(file_id) else {
            continue;
        };

        for imported_path in resolved_imports_for_file(lookup, &file.path) {
            let Some(imported_file) = lookup.file_by_path(&imported_path) else {
                continue;
            };
            let entry = grouped
                .entry(imported_file.id.clone())
                .or_insert_with(|| ContextCandidate::new(imported_file.id.clone(), 0, usize::MAX));
            entry.add_graph_boost(
                "graph_imported_file",
                IMPORTED_FILE_BOOST,
                0.5,
                format!("referenced by matched file: {}", file.path),
            );
        }

        for referencing_path in references_to_file(lookup, &file.path) {
            let Some(referencing_file) = lookup.file_by_path(&referencing_path) else {
                continue;
            };
            let entry = grouped
                .entry(referencing_file.id.clone())
                .or_insert_with(|| {
                    ContextCandidate::new(referencing_file.id.clone(), 0, usize::MAX)
                });
            entry.add_graph_boost(
                "graph_referencing_file",
                REFERENCING_FILE_BOOST,
                0.5,
                format!("references matched file: {}", file.path),
            );
        }
    }
}

fn add_reference_context(
    lookup: &IndexLookup<'_>,
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

    for file_id in matched_file_ids {
        let Some(file) = lookup.file_by_id(file_id) else {
            continue;
        };

        for reference in lookup.references_from_path(&file.path) {
            if let Some(target_path) = reference.target_path.as_deref()
                && let Some(target_file) = lookup.file_by_path(target_path)
            {
                let entry = grouped.entry(target_file.id.clone()).or_insert_with(|| {
                    ContextCandidate::new(target_file.id.clone(), 0, usize::MAX)
                });
                entry.add_graph_boost(
                    "graph_callee",
                    CALLEE_BOOST,
                    reference.confidence,
                    format!(
                        "{} from matched file: {}",
                        reference.kind, reference.target_name
                    ),
                );
            }
        }

        for reference in lookup.references_to_path(&file.path) {
            if reference.source_path != file.path
                && let Some(source_file) = lookup.file_by_path(&reference.source_path)
            {
                let entry = grouped.entry(source_file.id.clone()).or_insert_with(|| {
                    ContextCandidate::new(source_file.id.clone(), 0, usize::MAX)
                });
                entry.add_graph_boost(
                    "graph_caller",
                    CALLER_BOOST,
                    reference.confidence,
                    format!("{} matched file: {}", reference.kind, file.path),
                );
            }
        }
    }

    for symbol_id in matched_symbol_ids {
        for reference in lookup.references_to_symbol(symbol_id) {
            if let Some(source_file) = lookup.file_by_path(&reference.source_path) {
                let entry = grouped.entry(source_file.id.clone()).or_insert_with(|| {
                    ContextCandidate::new(source_file.id.clone(), 0, usize::MAX)
                });
                entry.add_graph_boost(
                    "graph_caller",
                    CALLER_BOOST,
                    reference.confidence,
                    format!(
                        "{} matched symbol: {}",
                        reference.kind, reference.target_name
                    ),
                );
            }
        }

        for reference in lookup.references_from_symbol(symbol_id) {
            if let Some(target_path) = reference.target_path.as_deref()
                && let Some(target_file) = lookup.file_by_path(target_path)
            {
                let entry = grouped.entry(target_file.id.clone()).or_insert_with(|| {
                    ContextCandidate::new(target_file.id.clone(), 0, usize::MAX)
                });
                entry.add_graph_boost(
                    "graph_callee",
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
}

fn imports_for_file(lookup: &IndexLookup<'_>, path: &str) -> Vec<String> {
    lookup
        .imports_from_path(path)
        .iter()
        .copied()
        .map(|import| {
            import
                .resolved_path
                .clone()
                .unwrap_or_else(|| import.imported.clone())
        })
        .collect()
}

fn resolved_imports_for_file(lookup: &IndexLookup<'_>, path: &str) -> Vec<String> {
    let mut imports: Vec<String> = lookup
        .imports_from_path(path)
        .iter()
        .copied()
        .filter_map(|import| import.resolved_path.clone())
        .collect();
    imports.sort();
    imports.dedup();
    imports
}

fn references_to_file(lookup: &IndexLookup<'_>, path: &str) -> Vec<String> {
    let mut references: Vec<String> = lookup
        .imports_to_path(path)
        .iter()
        .copied()
        .map(|import| import.source_path.clone())
        .collect();
    references.sort();
    references.dedup();
    references
}

fn calls_from_symbol(lookup: &IndexLookup<'_>, symbol: &SymbolRecord) -> Vec<ReferenceEdge> {
    lookup
        .references_from_symbol(&symbol.id)
        .iter()
        .copied()
        .filter(|reference| reference.kind == "call")
        .map(|reference| reference_edge(lookup, reference))
        .take(10)
        .collect()
}

fn references_from_symbol(lookup: &IndexLookup<'_>, symbol: &SymbolRecord) -> Vec<ReferenceEdge> {
    lookup
        .references_from_symbol(&symbol.id)
        .iter()
        .copied()
        .filter(|reference| reference.kind != "call")
        .map(|reference| reference_edge(lookup, reference))
        .take(10)
        .collect()
}

fn called_by_symbol(lookup: &IndexLookup<'_>, symbol: &SymbolRecord) -> Vec<ReferenceEdge> {
    lookup
        .references_to_symbol(&symbol.id)
        .iter()
        .copied()
        .filter(|reference| reference.kind == "call")
        .map(|reference| reference_edge(lookup, reference))
        .take(10)
        .collect()
}

fn calls_from_file(lookup: &IndexLookup<'_>, file: &FileRecord) -> Vec<ReferenceEdge> {
    lookup
        .references_from_path(&file.path)
        .iter()
        .copied()
        .filter(|reference| reference.source_path == file.path && reference.kind == "call")
        .map(|reference| reference_edge(lookup, reference))
        .take(10)
        .collect()
}

fn called_by_file(lookup: &IndexLookup<'_>, file: &FileRecord) -> Vec<ReferenceEdge> {
    lookup
        .references_to_path(&file.path)
        .iter()
        .copied()
        .filter(|reference| reference.source_path != file.path && reference.kind == "call")
        .map(|reference| reference_edge(lookup, reference))
        .take(10)
        .collect()
}

fn reference_edge(lookup: &IndexLookup<'_>, reference: &ReferenceRecord) -> ReferenceEdge {
    ReferenceEdge {
        file: reference.source_path.clone(),
        symbol: reference
            .source_symbol_id
            .as_deref()
            .and_then(|symbol_id| lookup.symbol_by_id(symbol_id))
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
        imports: take_string_refs(imports, MAX_CONTEXT_RELATION_FILES),
        referenced_by: take_string_refs(referenced_by, MAX_CONTEXT_RELATION_FILES),
        tests: take_strings(tests, MAX_CONTEXT_RELATED_TESTS),
        calls: take_strings(call_targets, MAX_CONTEXT_RELATION_FILES),
        called_by: take_strings(callers, MAX_CONTEXT_RELATION_FILES),
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

fn take_strings(values: Vec<String>, limit: usize) -> Vec<String> {
    values.into_iter().take(limit).collect()
}

fn take_string_refs(values: &[String], limit: usize) -> Vec<String> {
    values.iter().take(limit).cloned().collect()
}

fn compact_related_tests(tests: Vec<RelatedTest>) -> Vec<RelatedTest> {
    tests
        .into_iter()
        .take(MAX_CONTEXT_RELATED_TESTS)
        .map(|test| RelatedTest {
            file: test.file,
            symbols: test
                .symbols
                .into_iter()
                .take(MAX_CONTEXT_RELATED_TEST_SYMBOLS)
                .collect(),
        })
        .collect()
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
        critical_files,
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
        critical_files,
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

fn context_payload_reduction(
    baseline_context_payload_tokens_estimate: usize,
    callsieve_context_payload_tokens_estimate: usize,
) -> ContextPayloadReduction {
    let context_payload_tokens_saved_estimate = baseline_context_payload_tokens_estimate as isize
        - callsieve_context_payload_tokens_estimate as isize;
    let context_payload_reduction_ratio = if baseline_context_payload_tokens_estimate == 0 {
        0.0
    } else {
        context_payload_tokens_saved_estimate as f64
            / baseline_context_payload_tokens_estimate as f64
    };

    ContextPayloadReduction {
        label: "context_payload_reduction",
        evidence_tier: "platform_neutral_proxy",
        platform_scope: "agent_platform_neutral",
        baseline_context_payload_tokens_estimate,
        callsieve_context_payload_tokens_estimate,
        context_payload_tokens_saved_estimate,
        context_payload_reduction_ratio,
        context_payload_reduction_percent: context_payload_reduction_ratio * 100.0,
        estimator: "local deterministic token estimate, one token per four UTF-8 bytes",
        warning: "This estimates context payload reduction only. It is not observed whole-session token savings.",
    }
}

fn elapsed_ms(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
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
    critical_files_still_missed: usize,
    missed_files: Vec<TraceMiss>,
    critical_missed_files: Vec<TraceMiss>,
}

impl TraceAccumulator {
    fn add_observed(
        &mut self,
        id: Option<String>,
        task: Option<String>,
        expected_files: Vec<String>,
        critical_files: Vec<String>,
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
                id: id.clone(),
                task: task.clone(),
                files: missing,
            });
        }
        let critical_missing: Vec<String> = critical_files
            .into_iter()
            .filter(|file| !read_files.contains(file.as_str()))
            .collect();
        if !critical_missing.is_empty() {
            self.critical_files_still_missed += critical_missing.len();
            self.critical_missed_files.push(TraceMiss {
                id,
                task,
                files: critical_missing,
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
        self.critical_files_still_missed += summary.critical_files_still_missed;
        self.missed_files.extend(summary.missed_files.clone());
        self.critical_missed_files
            .extend(summary.critical_missed_files.clone());
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
            critical_files_still_missed: self.critical_files_still_missed,
            missed_files: self.missed_files,
            critical_missed_files: self.critical_missed_files,
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
            critical_files,
            observed,
            session,
        } = task;
        if let Some(observed) = session.or(observed) {
            accumulator.add_observed(
                id,
                task,
                expected_files,
                critical_files,
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
    snippet_from_lines(&lines, symbol)
}

fn snippet_from_lines(lines: &[&str], symbol: Option<&SymbolRecord>) -> Option<Snippet> {
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

    let Ok(content) = fs::read_to_string(root.join(&file.path)) else {
        return Vec::new();
    };
    let lines: Vec<&str> = content.lines().collect();

    let mut snippets: Vec<Snippet> = symbols
        .iter()
        .take(snippets_per_file)
        .filter_map(|symbol| snippet_from_lines(&lines, Some(*symbol)))
        .collect();

    if snippets.is_empty()
        && let Some(snippet) = snippet_from_lines(&lines, None)
    {
        snippets.push(snippet);
    }

    snippets
}

fn related_tests(lookup: &IndexLookup<'_>, file: &FileRecord) -> Vec<RelatedTest> {
    if file.is_test {
        return Vec::new();
    }

    let stem = file_stem(&file.path).to_ascii_lowercase();
    lookup
        .test_files
        .iter()
        .filter(|candidate| candidate.path.to_ascii_lowercase().contains(stem.as_str()))
        .take(5)
        .map(|test_file| RelatedTest {
            file: test_file.path.clone(),
            symbols: lookup
                .symbols_for_file(&test_file.id)
                .iter()
                .copied()
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

fn trace_token_accounting_source(value: &serde_json::Value) -> &str {
    value
        .get("token_accounting")
        .and_then(|accounting| accounting.get("source"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
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
        || lower.contains("callsieve begin")
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
    fn context_caps_agent_facing_explanations_and_graph_edges() {
        let (temp, index) = fixture_index();
        let output = build_context(
            temp.path(),
            &index,
            "change createSession refreshSession token behavior",
            8,
            2,
            true,
        )
        .unwrap();

        for file in output.read_first {
            assert!(file.symbols.len() <= MAX_CONTEXT_SYMBOLS_PER_FILE);
            assert!(file.why.len() <= MAX_CONTEXT_WHY);
            assert!(file.calls.len() <= MAX_CONTEXT_GRAPH_EDGES);
            assert!(file.called_by.len() <= MAX_CONTEXT_GRAPH_EDGES);
            assert!(file.imports.len() <= MAX_CONTEXT_RELATION_FILES);
            assert!(file.referenced_by.len() <= MAX_CONTEXT_RELATION_FILES);
            assert!(file.related_tests.len() <= MAX_CONTEXT_RELATED_TESTS);
            assert!(file.blast_radius.imports.len() <= MAX_CONTEXT_RELATION_FILES);
            assert!(file.blast_radius.called_by.len() <= MAX_CONTEXT_RELATION_FILES);
        }
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
        assert_eq!(
            output.context_payload_reduction.label,
            "context_payload_reduction"
        );
        assert_eq!(
            output.context_payload_reduction.evidence_tier,
            "platform_neutral_proxy"
        );
        assert_eq!(
            output.context_payload_reduction.platform_scope,
            "agent_platform_neutral"
        );
        assert_eq!(
            output
                .context_payload_reduction
                .baseline_context_payload_tokens_estimate,
            output.baseline.estimated_total_tokens
        );
        assert_eq!(
            output
                .context_payload_reduction
                .callsieve_context_payload_tokens_estimate,
            output.callsieve.estimated_packet_tokens
        );
        assert!(
            output
                .context_payload_reduction
                .context_payload_reduction_percent
                > 0.0
        );
        assert!(
            output
                .context_payload_reduction
                .warning
                .contains("not observed whole-session token savings")
        );
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
                critical_files: Vec::new(),
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
        assert_eq!(
            output.summary.context_payload_reduction.label,
            "context_payload_reduction"
        );
        assert_eq!(
            output.summary.baseline_context_payload_tokens_estimate as isize
                - output.summary.callsieve_context_payload_tokens_estimate as isize,
            output
                .summary
                .context_payload_reduction
                .context_payload_tokens_saved_estimate
        );
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
            protocol: None,
            thresholds: PilotThresholds::default(),
            audit: ProofAuditInput::default(),
            repos: vec![
                BenchmarkReportRepoInput {
                    path: repo_a.path().to_path_buf(),
                    label: Some("repo-a".to_string()),
                    team: None,
                    languages: Vec::new(),
                    clients: Vec::new(),
                    task_categories: Vec::new(),
                    scale_class: None,
                    scale_criteria: Vec::new(),
                    proof_tier: None,
                    external: false,
                    suite_path: Some(suite_a),
                    suite_paths: Vec::new(),
                    trace_path: None,
                    trace_paths: Vec::new(),
                    policy_trace_path: None,
                    policy_trace_paths: Vec::new(),
                    thresholds: None,
                },
                BenchmarkReportRepoInput {
                    path: repo_b.path().to_path_buf(),
                    label: Some("repo-b".to_string()),
                    team: None,
                    languages: Vec::new(),
                    clients: Vec::new(),
                    task_categories: Vec::new(),
                    scale_class: None,
                    scale_criteria: Vec::new(),
                    proof_tier: None,
                    external: false,
                    suite_path: Some(suite_b),
                    suite_paths: Vec::new(),
                    trace_path: None,
                    trace_paths: Vec::new(),
                    policy_trace_path: None,
                    policy_trace_paths: Vec::new(),
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
        assert_eq!(
            output.summary.context_payload_reduction.evidence_tier,
            "platform_neutral_proxy"
        );
        assert_eq!(
            output.summary.baseline_context_payload_tokens_estimate as isize
                - output.summary.callsieve_context_payload_tokens_estimate as isize,
            output
                .summary
                .context_payload_reduction
                .context_payload_tokens_saved_estimate
        );
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
