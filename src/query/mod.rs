pub mod classify;
#[cfg(feature = "embed")]
pub mod embed;
#[cfg(feature = "embed")]
pub mod embed_build;
pub mod formatter;
pub mod ranker;
pub mod stacktrace;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    indexer::{SCHEMA_VERSION, language::Language, ownership::Ownership},
    store::{self, CodeIndex, FileRecord, ImportRecord, ReferenceRecord, SymbolRecord},
};

const MAX_CONTEXT_SYMBOLS_PER_FILE: usize = 4;
const MAX_CONTEXT_WHY: usize = 6;
const MAX_CONTEXT_RELATION_FILES: usize = 5;
const MAX_CONTEXT_GRAPH_EDGES: usize = 1;
const MAX_CONTEXT_RELATED_TESTS: usize = 3;
const MAX_CONTEXT_RELATED_TEST_SYMBOLS: usize = 5;
const MAX_SELECTION_SUMMARY_NEXT_FILES: usize = 2;
const MAX_SKIM_SELECTION_NEXT_FILES: usize = 1;
const MAX_SKIM_GRAPH_HINTS_PER_DIRECTION: usize = 1;
const MAX_SKIM_CALL_PATHS_PER_DIRECTION: usize = 2;
const MAX_SKIM_SYMBOLS_PER_FILE: usize = 1;
const MAX_FOCUS_SYMBOL_SNIPPET_LINES: usize = 120;
const MAX_FOCUS_GRAPH_EDGES: usize = 4;
const MAX_CONTEXT_GRAPH_SCORE: i32 = 240;
const MIN_CONTEXT_CANDIDATE_MATCHES: usize = 128;
const MIN_TASK_SPECIFIC_TEST_SCORE: i32 = 2;
const TASK_MEMORY_SCHEMA_VERSION: u32 = 1;
const TASK_MEMORY_FILE: &str = "task-memory.json";
const MAX_TASK_MEMORY_ENTRIES: usize = 50;
const MAX_TASK_MEMORY_SIMILAR_TASKS: usize = 3;
const MAX_TASK_MEMORY_RECOMMENDED_FILES: usize = 8;
const MAX_TASK_MEMORY_RECOMMENDED_SYMBOLS: usize = 12;
pub const DEFAULT_AGENT_CONTEXT_TOKEN_BUDGET: usize = 1200;
pub const DEFAULT_AGENT_CONTEXT_LIMIT: usize = 5;

pub fn retrieval_contract_fingerprint() -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"callsieve-retrieval-contract-v1\n");
    bytes.extend_from_slice(include_bytes!("mod.rs"));
    bytes.extend_from_slice(b"\n--ranker--\n");
    bytes.extend_from_slice(include_bytes!("ranker.rs"));
    bytes.extend_from_slice(b"\n--formatter--\n");
    bytes.extend_from_slice(include_bytes!("formatter.rs"));
    bytes.extend_from_slice(b"\n--classifier--\n");
    bytes.extend_from_slice(include_bytes!("classify.rs"));
    crate::indexer::stable_content_hash(&bytes)
}

#[cfg(feature = "embed")]
type SemanticScoreMap = BTreeMap<String, SemanticScore>;

#[cfg(feature = "embed")]
#[derive(Debug, Clone)]
struct SemanticScore {
    cosine: f32,
    semantic: f64,
    chunk_symbol: Option<String>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ContextProfile {
    Skim,
    Normal,
    Full,
}

impl ContextProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skim => "skim",
            Self::Normal => "normal",
            Self::Full => "full",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ContextViewOptions {
    pub profile: ContextProfile,
    pub token_budget: Option<usize>,
    pub include_git: bool,
    pub include_call_paths: bool,
}

#[derive(Clone, Copy)]
pub struct ContextOptions<'a> {
    pub limit: usize,
    pub snippets_per_file: usize,
    pub include_snippets: bool,
    pub why_debug: bool,
    pub hybrid: HybridOptions<'a>,
    /// Frames parsed from `--error <file>`. Empty (the default) is a no-op, so
    /// the lexical path is byte-identical when no stack trace is supplied.
    pub error_frames: &'a [stacktrace::StackFrame],
    /// Opt-in: nudge recently-changed / hot files up using git signals. Off by
    /// default so the lexical baseline and the retrieval benchmark are unchanged
    /// until the boost is validated; default skim output omits git hints unless
    /// this boost is active.
    pub git_boost: bool,
    /// Opt-in: boost files that observed agent sessions confirmed reading for
    /// similar tasks (task-memory `confirmed_files`). Off by default until
    /// dogfood traces validate it; off means byte-identical output.
    pub memory_boost: bool,
}

#[derive(Clone, Copy)]
pub struct HybridOptions<'a> {
    pub embeddings: bool,
    #[cfg(feature = "embed")]
    pub embedder: Option<&'a dyn embed::LocalEmbedder>,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl Default for ContextOptions<'_> {
    fn default() -> Self {
        Self {
            limit: 8,
            snippets_per_file: 2,
            include_snippets: true,
            why_debug: false,
            hybrid: HybridOptions::default(),
            error_frames: &[],
            git_boost: false,
            memory_boost: false,
        }
    }
}

impl Default for HybridOptions<'_> {
    fn default() -> Self {
        Self {
            embeddings: false,
            #[cfg(feature = "embed")]
            embedder: None,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<'a> HybridOptions<'a> {
    pub fn embeddings(embeddings: bool) -> Self {
        Self {
            embeddings,
            #[cfg(feature = "embed")]
            embedder: None,
            _marker: std::marker::PhantomData,
        }
    }

    #[cfg(feature = "embed")]
    pub fn with_embedder(embeddings: bool, embedder: &'a dyn embed::LocalEmbedder) -> Self {
        Self {
            embeddings,
            embedder: Some(embedder),
            _marker: std::marker::PhantomData,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct RetrievalCost {
    retrieval_model_tokens: usize,
    retrieval_method: &'static str,
    agent_token_cost_scope: &'static str,
    note: &'static str,
}

pub const fn zero_token_retrieval_cost() -> RetrievalCost {
    RetrievalCost {
        retrieval_model_tokens: 0,
        retrieval_method: "deterministic_local_index",
        agent_token_cost_scope: "retrieval_only",
        note: "CallSieve spends zero AI model tokens on local retrieval. Only the returned context packet consumes agent context tokens when read.",
    }
}

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
    #[serde(skip_serializing_if = "is_false")]
    truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    omitted_lines: Option<usize>,
}

#[derive(Debug, Serialize)]
struct RelatedTest {
    file: String,
    symbols: Vec<String>,
}

fn is_false(value: &bool) -> bool {
    !*value
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
    retrieval_cost: RetrievalCost,
    selection_summary: ContextSelectionSummary,
    read_first: Vec<ContextFile>,
    stats: ContextStats,
    timing: TimingStats,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ContextSelectionSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    top_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_score: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_reason: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    top_signals: Vec<SelectionScoreComponent>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    next_files: Vec<SelectionSummaryFile>,
}

#[derive(Debug, Clone, Serialize)]
struct SelectionSummaryFile {
    file: String,
    score: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct SelectionScoreComponent {
    name: String,
    points: i32,
}

#[derive(Debug, Serialize)]
struct ContextFile {
    rank: usize,
    score: i32,
    selection_confidence: String,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    ownership: Option<Ownership>,
    #[serde(skip_serializing_if = "Option::is_none")]
    git: Option<crate::indexer::git::GitSignal>,
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
    #[serde(skip_serializing)]
    local_work: LocalWorkStats,
}

#[derive(Debug, Serialize)]
struct LocalWorkStats {
    indexed_files: usize,
    indexed_symbols: usize,
    indexed_references: usize,
    /// Files the semantic union pass added that lexical ranking missed.
    /// Limit-capped selection counts cannot reveal this, so benches and
    /// agents need it surfaced explicitly.
    semantic_injected: usize,
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

    pub fn add_warning(&mut self, warning: impl Into<String>) {
        self.warnings.push(warning.into());
    }
}

impl IndexStatusOutput {
    pub fn is_fresh(&self) -> bool {
        self.fresh
    }
}

#[derive(Debug, Serialize)]
pub struct TaskMemoryOutput {
    cache_hit: bool,
    path: String,
    policy: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    similar_tasks: Vec<TaskMemorySimilarTask>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    recommended_files: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    recommended_symbols: Vec<String>,
}

#[derive(Debug, Serialize)]
struct TaskMemorySimilarTask {
    task: String,
    score: f64,
    #[serde(skip_serializing_if = "String::is_empty")]
    client: String,
    shared_terms: Vec<String>,
    read_first_files: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TaskMemoryStore {
    schema_version: u32,
    entries: Vec<TaskMemoryEntry>,
}

impl Default for TaskMemoryStore {
    fn default() -> Self {
        Self {
            schema_version: TASK_MEMORY_SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TaskMemoryEntry {
    task: String,
    task_terms: Vec<String>,
    created_at: u64,
    read_first_files: Vec<String>,
    symbols: Vec<String>,
    tests: Vec<String>,
    /// Files the agent actually read after receiving context in an observed
    /// session — stronger evidence than what the packet suggested.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    confirmed_files: Vec<String>,
    /// Which agent client taught this entry (claude, codex, cursor, ...).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    client: String,
}

struct ScoredTaskMemoryEntry<'a> {
    entry: &'a TaskMemoryEntry,
    score: f64,
    shared_terms: Vec<String>,
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
    packet_quality: ContextPacketQuality,
    top_files: Vec<BenchmarkContextFile>,
}

#[derive(Debug, Default, Clone, Serialize)]
struct ContextPacketQuality {
    tasks: usize,
    selected_files: usize,
    files_with_symbols: usize,
    selected_symbols: usize,
    files_with_snippets: usize,
    snippets: usize,
    files_with_related_tests: usize,
    related_tests: usize,
    files_with_blast_radius: usize,
    blast_radius_hints: usize,
    files_with_call_graph_hints: usize,
    call_graph_hints: usize,
    files_with_non_unknown_risk: usize,
    files_with_selection_reasons: usize,
    selection_reasons: usize,
    files_with_selection_confidence: usize,
    selection_signals: usize,
    next_file_hints: usize,
    focus_targets: usize,
    relationship_followup_targets: usize,
    test_followup_targets: usize,
}

impl ContextPacketQuality {
    fn add(&mut self, other: &Self) {
        self.tasks += other.tasks;
        self.selected_files += other.selected_files;
        self.files_with_symbols += other.files_with_symbols;
        self.selected_symbols += other.selected_symbols;
        self.files_with_snippets += other.files_with_snippets;
        self.snippets += other.snippets;
        self.files_with_related_tests += other.files_with_related_tests;
        self.related_tests += other.related_tests;
        self.files_with_blast_radius += other.files_with_blast_radius;
        self.blast_radius_hints += other.blast_radius_hints;
        self.files_with_call_graph_hints += other.files_with_call_graph_hints;
        self.call_graph_hints += other.call_graph_hints;
        self.files_with_non_unknown_risk += other.files_with_non_unknown_risk;
        self.files_with_selection_reasons += other.files_with_selection_reasons;
        self.selection_reasons += other.selection_reasons;
        self.files_with_selection_confidence += other.files_with_selection_confidence;
        self.selection_signals += other.selection_signals;
        self.next_file_hints += other.next_file_hints;
        self.focus_targets += other.focus_targets;
        self.relationship_followup_targets += other.relationship_followup_targets;
        self.test_followup_targets += other.test_followup_targets;
    }
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
    retrieval_cost: RetrievalCost,
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
    context_selected_files: Vec<String>,
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
    first_correct_file_hit: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_correct_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_correct_file_rank: Option<usize>,
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
    first_correct_file_hits: usize,
    first_correct_file_tasks: usize,
    first_correct_file_rate_at_k: f64,
    baseline_context_payload_tokens_estimate: usize,
    callsieve_context_payload_tokens_estimate: usize,
    context_payload_reduction: ContextPayloadReduction,
    packet_quality: ContextPacketQuality,
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
    first_correct_file_hit: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_correct_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_correct_file_rank: Option<usize>,
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
    first_correct_file_hits: usize,
    first_correct_file_tasks: usize,
    first_correct_file_rate_at_k: f64,
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
    first_correct_file_hits: usize,
    first_correct_file_tasks: usize,
    first_correct_file_rate_at_k: f64,
    baseline_context_payload_tokens_estimate: usize,
    callsieve_context_payload_tokens_estimate: usize,
    context_payload_reduction: ContextPayloadReduction,
    packet_quality: ContextPacketQuality,
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
    first_correct_file_hits: usize,
    first_correct_file_tasks: usize,
    first_correct_file_rate_at_k: f64,
    baseline_context_payload_tokens_estimate: usize,
    callsieve_context_payload_tokens_estimate: usize,
    context_payload_reduction: ContextPayloadReduction,
    packet_quality: ContextPacketQuality,
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

/// Scale applied to an off-topic test file's whole candidate score (symbol plus
/// graph contributions) so a well-connected test file cannot outrank the real
/// implementation on a non-test query. Tests still appear, just not at the top.
const TEST_OFFTOPIC_SCALE: f32 = 0.4;

#[derive(Debug)]
struct ContextCandidate {
    file_id: String,
    best_score: i32,
    graph_score: i32,
    graph_confidence: f64,
    first_rank: usize,
    test_offtopic: bool,
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
            test_offtopic: false,
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
        let raw = self.best_score + self.graph_score + (bonus_count * 5);
        if self.test_offtopic {
            (raw as f32 * TEST_OFFTOPIC_SCALE).round() as i32
        } else {
            raw
        }
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

    /// Consensus support escapes the MAX_CONTEXT_GRAPH_SCORE cap on purpose:
    /// agreement of multiple top-ranked files is a stronger signal than a
    /// single graph edge and must be able to rival content-keyword noise.
    fn add_consensus_boost(&mut self, points: i32, why: String) {
        if self.seen_why.insert(why.clone()) {
            self.best_score += points;
            self.why.push(why.clone());
            self.push_debug(ranker::ScoreComponent {
                name: "graph_consensus".to_string(),
                points,
                detail: why,
            });
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

    #[cfg(feature = "embed")]
    fn push_debug_front(&mut self, component: ranker::ScoreComponent) {
        let key = format!(
            "{}:{}:{}",
            component.name, component.points, component.detail
        );
        if self.seen_debug.insert(key) {
            self.why_debug.insert(0, component);
        }
    }
}

fn selection_confidence_for_score(score: i32, top_score: i32) -> &'static str {
    if score <= 0 {
        return "low";
    }
    if top_score <= 0 {
        return "medium";
    }

    let ratio = score as f64 / top_score as f64;
    if ratio >= 0.8 {
        "high"
    } else if ratio >= 0.45 {
        "medium"
    } else {
        "low"
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
    grep_before_context: usize,
    grep_after_context: usize,
    context_first_compliant: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    violation_details: Vec<TraceCheckViolation>,
}

#[derive(Debug, Serialize)]
pub struct FocusOutput {
    root: String,
    file: String,
    language: Language,
    symbols: Vec<QuerySymbol>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    snippets: Vec<Snippet>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    calls: Vec<FocusEdge>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    references: Vec<FocusEdge>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    called_by: Vec<FocusEdge>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    related_tests: Vec<RelatedTest>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct FocusEdge {
    file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    symbol: Option<String>,
    target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_file: Option<String>,
    line: usize,
}

#[derive(Debug, Serialize)]
pub struct RelatedOutput {
    root: String,
    file: String,
    imports: Vec<String>,
    referenced_by: Vec<String>,
    blast_radius: BlastRadius,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    calls: Vec<ReferenceEdge>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    called_by: Vec<ReferenceEdge>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct TestsOutput {
    root: String,
    file: String,
    related_tests: Vec<RelatedTest>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
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

pub fn focus_file(
    root: &Path,
    index: &CodeIndex,
    file_path: &str,
    symbol_name: Option<&str>,
    line: Option<usize>,
    include_references: bool,
    snippets_per_symbol: usize,
) -> Result<FocusOutput> {
    let lookup = IndexLookup::new(index);
    let file = lookup
        .file_by_path(file_path)
        .ok_or_else(|| anyhow!("file is not indexed: {file_path}"))?;
    let all_symbols = lookup.symbols_for_file(&file.id);
    let mut symbol_records: Vec<&SymbolRecord> = if let Some(symbol_name) = symbol_name {
        let exact_matches = all_symbols
            .iter()
            .copied()
            .filter(|symbol| symbol.name.eq_ignore_ascii_case(symbol_name))
            .collect::<Vec<_>>();
        if exact_matches.is_empty() {
            let symbol_name_lower = symbol_name.to_ascii_lowercase();
            all_symbols
                .iter()
                .copied()
                .filter(|symbol| {
                    symbol
                        .name
                        .to_ascii_lowercase()
                        .contains(&symbol_name_lower)
                })
                .collect()
        } else {
            exact_matches
        }
    } else {
        all_symbols.to_vec()
    };
    if let Some(line) = line {
        symbol_records.retain(|symbol| symbol.start_line <= line && line <= symbol.end_line);
    }
    symbol_records.sort_by_key(|symbol| symbol.start_line);
    if (symbol_name.is_some() || line.is_some()) && symbol_records.is_empty() {
        let selector = match (symbol_name, line) {
            (Some(symbol), Some(line)) => format!("{symbol} at line {line}"),
            (Some(symbol), None) => symbol.to_string(),
            (None, Some(line)) => format!("line {line}"),
            (None, None) => String::new(),
        };
        return Err(anyhow!(
            "symbol selector was not found in indexed file {file_path}: {selector}"
        ));
    }
    if symbol_name.is_none() && line.is_none() {
        symbol_records.truncate(MAX_CONTEXT_SYMBOLS_PER_FILE);
    }

    let symbols = symbol_records
        .iter()
        .map(|symbol| QuerySymbol {
            name: symbol.name.clone(),
            kind: symbol.kind.clone(),
            lines: [symbol.start_line, symbol.end_line],
            visibility: symbol.visibility.clone(),
            signature: symbol.signature.clone(),
        })
        .collect();
    let snippets = if symbol_name.is_some() || line.is_some() {
        focused_symbol_snippets(root, file, &symbol_records, snippets_per_symbol)
    } else {
        context_snippets(
            root,
            file,
            &symbol_records,
            snippets_per_symbol,
            snippets_per_symbol > 0,
        )
    };
    let (calls, references, called_by) = if symbol_name.is_some() || line.is_some() {
        (
            focus_edges_for_symbols(&lookup, &symbol_records, FocusEdgeKind::Calls),
            if include_references {
                focus_edges_for_symbols(&lookup, &symbol_records, FocusEdgeKind::References)
            } else {
                Vec::new()
            },
            focus_edges_for_symbols(&lookup, &symbol_records, FocusEdgeKind::CalledBy),
        )
    } else {
        (Vec::new(), Vec::new(), Vec::new())
    };
    let related_tests = if symbol_name.is_some() || line.is_some() {
        compact_related_tests(related_tests(&lookup, file))
    } else {
        Vec::new()
    };

    Ok(FocusOutput {
        root: root_label(root),
        file: file.path.clone(),
        language: file.language,
        symbols,
        snippets,
        calls,
        references,
        called_by,
        related_tests,
        warnings: stale_warnings(root, index),
    })
}

pub fn related_file(root: &Path, index: &CodeIndex, file_path: &str) -> Result<RelatedOutput> {
    let lookup = IndexLookup::new(index);
    let file = lookup
        .file_by_path(file_path)
        .ok_or_else(|| anyhow!("file is not indexed: {file_path}"))?;
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

    Ok(RelatedOutput {
        root: root_label(root),
        file: file.path.clone(),
        imports: take_strings(imports_all, MAX_CONTEXT_RELATION_FILES),
        referenced_by: take_strings(referenced_by_all, MAX_CONTEXT_RELATION_FILES),
        blast_radius,
        calls: calls_all
            .into_iter()
            .take(MAX_CONTEXT_GRAPH_EDGES)
            .collect(),
        called_by: called_by_all
            .into_iter()
            .take(MAX_CONTEXT_GRAPH_EDGES)
            .collect(),
        warnings: stale_warnings(root, index),
    })
}

pub fn tests_for_file(root: &Path, index: &CodeIndex, file_path: &str) -> Result<TestsOutput> {
    let lookup = IndexLookup::new(index);
    let file = lookup
        .file_by_path(file_path)
        .ok_or_else(|| anyhow!("file is not indexed: {file_path}"))?;
    Ok(TestsOutput {
        root: root_label(root),
        file: file.path.clone(),
        related_tests: compact_related_tests(related_tests(&lookup, file)),
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
                related_tests: compact_related_tests(related_tests(&lookup, file)),
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
    build_context_with(
        root,
        index,
        task,
        ContextOptions {
            limit,
            snippets_per_file,
            include_snippets,
            why_debug,
            hybrid: HybridOptions::default(),
            error_frames: &[],
            git_boost: false,
            memory_boost: false,
        },
    )
}

pub fn build_context_with(
    root: &Path,
    index: &CodeIndex,
    task: &str,
    options: ContextOptions<'_>,
) -> Result<ContextOutput> {
    let total_start = Instant::now();
    let candidate_limit = if options.limit == 0 {
        0
    } else {
        options
            .limit
            .saturating_mul(16)
            .max(MIN_CONTEXT_CANDIDATE_MATCHES)
    };
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
    let query_tokens = ranker::query_tokens(task);
    let query_kind = classify::query_kind(task, &query_tokens);

    let graph_start = Instant::now();
    add_graph_context(&lookup, &ranked, &mut grouped);
    add_reference_context(&lookup, &ranked, &mut grouped);
    if query_kind == classify::QueryKind::NaturalLanguage {
        add_natural_language_module_neighbors(&lookup, &ranked, &mut grouped, &query_tokens);
    }
    let graph_expansion_ms = elapsed_ms(graph_start.elapsed());

    let mut candidates: Vec<ContextCandidate> = grouped.into_values().collect();

    // Semantic-recall union: when embeddings are enabled, let the embedding
    // nearest-neighbors inject files that lexical ranking never surfaced. This
    // runs before the test-offtopic marking below so injected test files get the
    // same down-scaling, and before the hybrid blend so they can be ranked into
    // the read-first set. It is a no-op when embeddings are off (determinism).
    let candidates_before_injection = candidates.len();
    let semantic_scores = add_semantic_candidates(
        root,
        index,
        task,
        &mut candidates,
        options.limit,
        &query_tokens,
        options.hybrid,
    )?;
    let semantic_injected = candidates.len() - candidates_before_injection;

    // Memory-confirmed boosts/injections run before the test-offtopic marking
    // below for the same reason semantic injection does: an injected test
    // file must receive the same down-scaling as one lexical surfaced.
    if options.memory_boost {
        add_memory_confirmed_boost(root, task, &lookup, &mut candidates);
    }

    // A test file that merely references the relevant source files gets a large
    // graph boost; on a non-test query, scale the whole candidate down so it
    // cannot outrank the implementation it tests.
    if !ranker::has_test_intent(&query_tokens) && !ranker::has_hook_meta_intent(&query_tokens) {
        for candidate in &mut candidates {
            if lookup
                .file_by_id(&candidate.file_id)
                .is_some_and(|file| file.is_test)
            {
                candidate.test_offtopic = true;
            }
        }
    }

    // Stack-trace evidence (from `--error`) is a strong, explicit signal: the
    // crash points right at these files. Boost/inject them and clear any
    // test-offtopic penalty so they rank at the top. No-op when no trace given.
    apply_error_context(index, &mut candidates, options.error_frames, options.limit);
    apply_git_boost(index, &mut candidates, options.git_boost);

    let mut warnings = stale_warnings(root, index);
    // Graph consensus exists for vocabulary-gap queries: natural-language
    // issue text matches the consumer layer around the buggy file and the
    // graph closes the hop. Identifier queries carry explicit lexical
    // anchors and keep their proven ordering untouched.
    if query_kind == classify::QueryKind::NaturalLanguage {
        add_graph_consensus_boost(&lookup, &mut candidates);
    }
    sort_candidates_lexical(&mut candidates, &lookup, &query_tokens);
    apply_hybrid_ranking(
        root,
        index,
        task,
        &query_tokens,
        &lookup,
        &mut candidates,
        options.hybrid,
        semantic_scores.as_ref(),
        &mut warnings,
    )?;
    promote_implementation_companion(&lookup, &mut candidates, options.limit, &query_tokens);
    promote_task_specific_test_companion(&lookup, &mut candidates, options.limit, &query_tokens);

    let mut selected_symbols = 0;
    let mut selected_related_tests = 0;
    let mut snippet_elapsed = Duration::ZERO;
    let mut selection_summary = empty_context_selection_summary();
    let top_score = candidates
        .first()
        .map(ContextCandidate::score)
        .unwrap_or_default();
    let read_first: Vec<ContextFile> = candidates
        .into_iter()
        .take(options.limit)
        .enumerate()
        .filter_map(|(rank_index, candidate)| {
            let file = lookup.file_by_id(&candidate.file_id)?;
            let mut symbol_records: Vec<&SymbolRecord> = candidate
                .symbol_ids
                .iter()
                .filter_map(|symbol_id| lookup.symbol_by_id(symbol_id))
                .collect();
            // Snippet the region most relevant to the query first, so a large
            // multi-purpose file points at the matching symbol instead of the
            // first symbol by accumulation order.
            symbol_records.sort_by(|left, right| {
                ranker::symbol_query_affinity(right, &query_tokens)
                    .cmp(&ranker::symbol_query_affinity(left, &query_tokens))
                    .then(left.start_line.cmp(&right.start_line))
            });

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
                options.snippets_per_file,
                options.include_snippets,
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
            let top_score_components = compact_selection_score_components(&candidate.why_debug);
            push_context_selection_summary(
                &mut selection_summary,
                file,
                score,
                &why,
                top_score_components,
            );
            let debug = if options.why_debug {
                candidate.why_debug.into_iter().take(16).collect()
            } else {
                Vec::new()
            };

            selected_symbols += symbols.len();
            selected_related_tests += related_tests.len();

            Some(ContextFile {
                rank: rank_index + 1,
                score,
                selection_confidence: selection_confidence_for_score(score, top_score).to_string(),
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
                ownership: file.ownership.clone(),
                git: file.git.clone(),
                why,
                why_debug: debug,
            })
        })
        .collect();

    Ok(ContextOutput {
        task: task.to_string(),
        root: root_label(root),
        retrieval_cost: zero_token_retrieval_cost(),
        selection_summary,
        stats: ContextStats {
            candidate_matches: ranked.len(),
            selected_files: read_first.len(),
            selected_symbols,
            related_tests: selected_related_tests,
            local_work: LocalWorkStats {
                indexed_files: index.files.len(),
                indexed_symbols: index.symbols.len(),
                indexed_references: index.references.len(),
                semantic_injected,
            },
        },
        timing: TimingStats {
            index_load_ms: 0,
            ranking_ms,
            graph_expansion_ms,
            snippet_ms: elapsed_ms(snippet_elapsed),
            total_ms: elapsed_ms(total_start.elapsed()),
        },
        read_first,
        warnings,
    })
}

pub fn context_read_first_files(context: &ContextOutput) -> Vec<String> {
    context
        .read_first
        .iter()
        .map(|file| file.file.clone())
        .collect()
}

#[derive(Debug, Clone)]
pub struct FocusTarget {
    pub file: String,
    pub symbol: Option<String>,
    pub line: Option<usize>,
    pub is_code: bool,
}

pub fn context_read_first_targets(context: &ContextOutput) -> Vec<FocusTarget> {
    context
        .read_first
        .iter()
        .map(|file| {
            let symbol = focus_symbol_for_context_file(file);
            FocusTarget {
                file: file.file.clone(),
                symbol: symbol.map(|symbol| symbol.name.clone()),
                line: symbol.map(|symbol| symbol.lines[0]),
                is_code: file.language.is_code(),
            }
        })
        .collect()
}

fn focus_symbol_for_context_file(file: &ContextFile) -> Option<&QuerySymbol> {
    file.symbols.iter().find(|symbol| {
        !matches!(
            symbol.kind.as_str(),
            "macro" | "call" | "reference" | "import" | "use" | "include"
        )
    })
}

fn empty_context_selection_summary() -> ContextSelectionSummary {
    ContextSelectionSummary {
        top_file: None,
        top_score: None,
        top_reason: None,
        top_signals: Vec::new(),
        next_files: Vec::new(),
    }
}

fn push_context_selection_summary(
    summary: &mut ContextSelectionSummary,
    file: &FileRecord,
    score: i32,
    why: &[String],
    top_score_components: Vec<SelectionScoreComponent>,
) {
    let file_summary = SelectionSummaryFile {
        file: file.path.clone(),
        score,
        reason: why.first().cloned(),
    };
    if summary.top_file.is_none() {
        summary.top_file = Some(file_summary.file.clone());
        summary.top_score = Some(file_summary.score);
        summary.top_reason = file_summary.reason;
        summary.top_signals = top_score_components;
    } else if summary.next_files.len() < MAX_SELECTION_SUMMARY_NEXT_FILES {
        summary.next_files.push(file_summary);
    }
}

fn compact_selection_score_components(
    components: &[ranker::ScoreComponent],
) -> Vec<SelectionScoreComponent> {
    let mut components = components
        .iter()
        .filter(|component| component.points > 0)
        .map(|component| SelectionScoreComponent {
            name: component.name.clone(),
            points: component.points,
        })
        .collect::<Vec<_>>();
    components.sort_by(|left, right| {
        right
            .points
            .cmp(&left.points)
            .then(left.name.cmp(&right.name))
    });
    components.dedup_by(|left, right| left.name == right.name && left.points == right.points);
    components.truncate(1);
    components
}

fn sort_candidates_lexical(
    candidates: &mut [ContextCandidate],
    lookup: &IndexLookup<'_>,
    query_tokens: &[String],
) {
    candidates.sort_by(|left, right| {
        right
            .score()
            .cmp(&left.score())
            .then_with(|| right.graph_confidence.total_cmp(&left.graph_confidence))
            .then_with(|| {
                ownership_rank(right, query_tokens, lookup).cmp(&ownership_rank(
                    left,
                    query_tokens,
                    lookup,
                ))
            })
            .then(left.first_rank.cmp(&right.first_rank))
            .then(left.file_id.cmp(&right.file_id))
    });
}

/// Lexical score floor given to files named in a `--error` stack trace. Well
/// above ordinary lexical scores (hundreds) so crash-implicated files lead the
/// read-first set, while still being a normal score the hybrid blend can reason
/// about.
const ERROR_TRACE_SCORE: i32 = 5000;

/// Symbol kinds that are call/reference sites rather than definitions; skipped
/// when choosing the symbol enclosing a stack-trace line.
const NON_DEFINITION_SYMBOL_KINDS: &[&str] =
    &["macro", "call", "reference", "import", "use", "include"];

/// Boost (and, where needed, inject) the files a `--error` stack trace points
/// at, attaching the symbol that encloses each frame's line. Deterministic and
/// not embedding-gated; returns immediately when `frames` is empty so the
/// lexical path is unchanged.
fn apply_error_context(
    index: &CodeIndex,
    candidates: &mut Vec<ContextCandidate>,
    frames: &[stacktrace::StackFrame],
    limit: usize,
) {
    if frames.is_empty() || limit == 0 {
        return;
    }
    for frame_match in stacktrace::match_frames(frames, index) {
        let why = match frame_match.line {
            Some(line) => format!("appears in provided stack trace (line {line})"),
            None => "appears in provided stack trace".to_string(),
        };

        let idx = match candidates
            .iter()
            .position(|candidate| candidate.file_id == frame_match.file_id)
        {
            Some(idx) => idx,
            None => {
                let rank = candidates.len();
                candidates.push(ContextCandidate::new(frame_match.file_id.clone(), 0, rank));
                candidates.len() - 1
            }
        };

        // Resolve the enclosing symbol before borrowing the candidate. Prefer
        // the tightest *definition* (function/struct/...) over a call/macro that
        // happens to sit on the same line - a crash points at the function the
        // agent needs to read, not the `panic!` invocation inside it.
        let covering_symbol = frame_match.line.and_then(|line| {
            let enclosing: Vec<&SymbolRecord> = index
                .symbols
                .iter()
                .filter(|symbol| {
                    symbol.file_id == frame_match.file_id
                        && symbol.start_line <= line
                        && line <= symbol.end_line
                })
                .collect();
            enclosing
                .iter()
                .filter(|symbol| !NON_DEFINITION_SYMBOL_KINDS.contains(&symbol.kind.as_str()))
                .min_by_key(|symbol| symbol.end_line.saturating_sub(symbol.start_line))
                .or_else(|| {
                    enclosing
                        .iter()
                        .min_by_key(|symbol| symbol.end_line.saturating_sub(symbol.start_line))
                })
                .map(|symbol| symbol.id.clone())
        });

        let candidate = &mut candidates[idx];
        candidate.best_score = candidate.best_score.max(ERROR_TRACE_SCORE);
        candidate.test_offtopic = false;
        if candidate.seen_why.insert(why.clone()) {
            candidate.why.push(why.clone());
        }
        candidate.push_debug(ranker::ScoreComponent {
            name: "stack_trace".to_string(),
            points: ERROR_TRACE_SCORE,
            detail: why,
        });
        if let Some(symbol_id) = covering_symbol
            && candidate.symbol_ids.len() < MAX_CONTEXT_SYMBOLS_PER_FILE
            && !candidate.symbol_ids.contains(&symbol_id)
        {
            candidate.symbol_ids.push(symbol_id);
        }
    }
}

/// Points added for a file changed at least once in the last 30 days.
const GIT_RECENCY_BOOST: i32 = 150;
/// Points per commit in the last 90 days (capped) - a hotspot signal.
const GIT_HOTSPOT_PER_COMMIT: i32 = 15;
const GIT_HOTSPOT_CAP_COMMITS: u32 = 10;

/// Opt-in recency/hotspot boost from git signals. Off by default so the lexical
/// baseline and the retrieval benchmark stay byte-identical; when enabled it
/// nudges recently-changed and frequently-touched files up with an explicit
/// `why` so the boost is auditable.
fn apply_git_boost(index: &CodeIndex, candidates: &mut [ContextCandidate], enabled: bool) {
    if !enabled {
        return;
    }
    let signals: BTreeMap<&str, &crate::indexer::git::GitSignal> = index
        .files
        .iter()
        .filter_map(|file| file.git.as_ref().map(|git| (file.id.as_str(), git)))
        .collect();

    for candidate in candidates.iter_mut() {
        let Some(git) = signals.get(candidate.file_id.as_str()) else {
            continue;
        };
        let hotspot =
            (git.commits_90d.min(GIT_HOTSPOT_CAP_COMMITS) as i32) * GIT_HOTSPOT_PER_COMMIT;
        let recency = if git.commits_30d > 0 {
            GIT_RECENCY_BOOST
        } else {
            0
        };
        let boost = hotspot + recency;
        if boost == 0 {
            continue;
        }
        let why = if recency > 0 {
            format!(
                "recently changed ({} commits/30d, {} authors/90d)",
                git.commits_30d, git.distinct_authors_90d
            )
        } else {
            format!("hot file ({} commits/90d)", git.commits_90d)
        };
        candidate.best_score = candidate.best_score.saturating_add(boost);
        // Front-insert so the reason for the re-ranking survives the skim view's
        // top-2 `why` truncation - the user opted into this, they should see it.
        if candidate.seen_why.insert(why.clone()) {
            candidate.why.insert(0, why.clone());
        }
        candidate.push_debug(ranker::ScoreComponent {
            name: "git_signal".to_string(),
            points: boost,
            detail: why,
        });
    }
}

/// Semantic-recall union pass. Reads the embedding cache, scores every indexed
/// file the lexical ranker did *not* already surface by cosine to the query, and
/// injects the strongest ones (above `query_kind.cosine_floor()`, capped at
/// `limit`) as zero-lexical-score candidates. This is what lets hybrid exceed
/// the lexical recall ceiling instead of merely reordering the lexical set.
///
/// Determinism: returns immediately when embeddings are off, and the injected
/// set is ordered by `(cosine desc, file_id asc)`, so a fixed cache yields a
/// reproducible result. Cache miss is silent here; `apply_hybrid_ranking` emits
/// the user-facing warning.
#[cfg(feature = "embed")]
fn add_semantic_candidates(
    root: &Path,
    index: &CodeIndex,
    task: &str,
    candidates: &mut Vec<ContextCandidate>,
    limit: usize,
    query_tokens: &[String],
    hybrid: HybridOptions<'_>,
) -> Result<Option<SemanticScoreMap>> {
    use embed::{ExpectedCache, FastembedEmbedder, LocalEmbedder};

    if !hybrid.embeddings || limit == 0 {
        return Ok(None);
    }

    let embedder_id = hybrid
        .embedder
        .map(|embedder| embedder.id())
        .unwrap_or_else(FastembedEmbedder::default_id);
    let fingerprint = embed::index_fingerprint(index);
    let expected = ExpectedCache {
        embedder: &embedder_id,
        index_schema_version: SCHEMA_VERSION,
        fingerprint: &fingerprint,
        expected_file_count: index.files.len(),
    };
    let Some(cache) = embed::read_embeds(root, &expected)? else {
        return Ok(None);
    };

    let owned_embedder;
    let embedder: &dyn LocalEmbedder = if let Some(embedder) = hybrid.embedder {
        embedder
    } else {
        owned_embedder = FastembedEmbedder::new_default()?;
        &owned_embedder
    };
    let query_vectors = embedder.embed(&[task])?;
    let Some(query_vector) = query_vectors.first() else {
        return Ok(None);
    };
    if query_vector.len() != cache.dim {
        return Ok(None);
    }
    let Some(query_unit) = normalize_vector(query_vector) else {
        return Ok(None);
    };
    let semantic_scores = semantic_scores_from_cache(index, &cache, &query_unit);

    let existing: BTreeSet<&str> = candidates
        .iter()
        .map(|candidate| candidate.file_id.as_str())
        .collect();

    let cosine_floor = classify::query_kind(task, query_tokens).cosine_floor();
    let mut scored: Vec<(f32, &str, Option<&str>)> = Vec::new();
    for (file_id, score) in &semantic_scores {
        if existing.contains(file_id.as_str()) {
            continue;
        }
        if score.cosine >= cosine_floor {
            scored.push((
                score.cosine,
                file_id.as_str(),
                score.chunk_symbol.as_deref(),
            ));
        }
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1)));

    let base_rank = candidates.len();
    for (offset, (cosine, file_id, chunk_symbol)) in scored.into_iter().take(limit).enumerate() {
        let mut candidate = ContextCandidate::new(file_id.to_string(), 0, base_rank + offset);
        if let Some(symbol_id) = chunk_symbol
            && candidate.symbol_ids.len() < MAX_CONTEXT_SYMBOLS_PER_FILE
        {
            candidate.symbol_ids.push(symbol_id.to_string());
        }
        let why = if let Some(symbol_id) = chunk_symbol {
            format!(
                "surfaced by semantic recall via {symbol_id} (no lexical match), cosine={cosine:.3}"
            )
        } else {
            format!("surfaced by semantic recall (no lexical match), cosine={cosine:.3}")
        };
        candidate.seen_why.insert(why.clone());
        candidate.why.push(why.clone());
        candidate.push_debug(ranker::ScoreComponent {
            name: "semantic_recall".to_string(),
            points: (cosine * 1000.0).round() as i32,
            detail: why,
        });
        candidates.push(candidate);
    }

    Ok(Some(semantic_scores))
}

#[cfg(not(feature = "embed"))]
fn add_semantic_candidates(
    _root: &Path,
    _index: &CodeIndex,
    _task: &str,
    _candidates: &mut Vec<ContextCandidate>,
    _limit: usize,
    _query_tokens: &[String],
    _hybrid: HybridOptions<'_>,
) -> Result<Option<()>> {
    Ok(None)
}

#[cfg(feature = "embed")]
fn semantic_scores_from_cache(
    index: &CodeIndex,
    cache: &embed::EmbedCache,
    query_unit: &[f32],
) -> SemanticScoreMap {
    let mut scores = BTreeMap::new();
    for (chunk_index, vector) in cache.vectors.iter().enumerate() {
        let Some(owner) = cache.chunk_owners.get(chunk_index) else {
            continue;
        };
        let Some(file) = index.files.get(*owner as usize) else {
            continue;
        };
        let cosine = cosine_with_unit_query(query_unit, vector).unwrap_or(0.0);
        let semantic = ((cosine as f64 + 1.0) / 2.0).clamp(0.0, 1.0);
        let chunk_symbol = cache
            .chunk_symbols
            .get(chunk_index)
            .and_then(|symbol| symbol.clone());
        let next = SemanticScore {
            cosine,
            semantic,
            chunk_symbol,
        };
        scores
            .entry(file.id.clone())
            .and_modify(|current: &mut SemanticScore| {
                if next.cosine > current.cosine {
                    *current = next.clone();
                }
            })
            .or_insert(next);
    }
    scores
}

#[cfg(feature = "embed")]
#[allow(clippy::too_many_arguments)]
fn apply_hybrid_ranking(
    _root: &Path,
    _index: &CodeIndex,
    task: &str,
    query_tokens: &[String],
    lookup: &IndexLookup<'_>,
    candidates: &mut [ContextCandidate],
    hybrid: HybridOptions<'_>,
    semantic_scores: Option<&SemanticScoreMap>,
    warnings: &mut Vec<String>,
) -> Result<()> {
    if !hybrid.embeddings || candidates.is_empty() {
        return Ok(());
    }
    let Some(semantic_scores) = semantic_scores else {
        warnings.push(
            "--embeddings requested but .callsieve/embeds.bin is missing or stale; using lexical ranking"
                .to_string(),
        );
        return Ok(());
    };
    let min_score = candidates
        .iter()
        .map(ContextCandidate::score)
        .min()
        .unwrap_or(0);
    let max_score = candidates
        .iter()
        .map(ContextCandidate::score)
        .max()
        .unwrap_or(0);
    let query_kind = classify::query_kind(task, query_tokens);
    let (lex_weight, semantic_weight) = query_kind.weights();
    let mut order_keys = BTreeMap::new();

    for candidate in candidates.iter_mut() {
        let lex_norm = normalized_lexical_score(candidate.score(), min_score, max_score);
        let (semantic, detail) = if let Some(score) = semantic_scores.get(&candidate.file_id) {
            let best_chunk = score
                .chunk_symbol
                .as_deref()
                .map(|symbol| format!(", best_chunk={symbol}"))
                .unwrap_or_default();
            (
                score.semantic,
                format!(
                    "query_kind={}, lex_norm={:.3}, sem={:.3}, cosine={:.3}{}",
                    query_kind.as_str(),
                    lex_norm,
                    score.semantic,
                    score.cosine,
                    best_chunk
                ),
            )
        } else {
            let why = "semantic embedding missing; neutral semantic score used".to_string();
            if candidate.seen_why.insert(why.clone()) {
                candidate.why.push(why.clone());
            }
            (
                0.5,
                format!(
                    "query_kind={}, lex_norm={:.3}, sem=0.500, missing_vector=true",
                    query_kind.as_str(),
                    lex_norm
                ),
            )
        };
        let order_key = (lex_weight * lex_norm) + (semantic_weight * semantic);
        candidate.push_debug_front(ranker::ScoreComponent {
            name: "semantic_embedding".to_string(),
            points: (order_key * 1000.0).round() as i32,
            detail,
        });
        order_keys.insert(candidate.file_id.clone(), order_key);
    }

    // Identifier queries carry explicit lexical anchors, and across every
    // public bench semantic reordering of them produced zero wins and one
    // persistent loss (a weakly-embedded correct file diluted out of the
    // top-k). Keep their lexical order; the semantic debug annotations above
    // remain for explainability. Natural-language queries keep the full
    // blend — reranking is where all their wins come from.
    if query_kind == classify::QueryKind::Identifier {
        return Ok(());
    }

    let lexical_rank: BTreeMap<String, usize> = candidates
        .iter()
        .enumerate()
        .map(|(rank, candidate)| (candidate.file_id.clone(), rank))
        .collect();

    candidates.sort_by(|left, right| {
        order_keys
            .get(&right.file_id)
            .copied()
            .unwrap_or_default()
            .total_cmp(&order_keys.get(&left.file_id).copied().unwrap_or_default())
            .then_with(|| right.graph_confidence.total_cmp(&left.graph_confidence))
            .then_with(|| {
                ownership_rank(right, query_tokens, lookup).cmp(&ownership_rank(
                    left,
                    query_tokens,
                    lookup,
                ))
            })
            .then(left.first_rank.cmp(&right.first_rank))
            .then(left.file_id.cmp(&right.file_id))
    });

    // Semantic similarity must not lift a test file above source files it
    // trailed lexically (test bodies repeat task vocabulary and embed close
    // to it), unless the query is actually about tests. Bubble such test
    // files back below the source files they displaced; relative order among
    // unconstrained pairs keeps the blended ranking.
    if !ranker::has_test_intent(query_tokens) {
        let is_test = |candidate: &ContextCandidate| {
            lookup
                .file_by_id(&candidate.file_id)
                .is_some_and(|file| file.is_test)
        };
        let mut changed = true;
        while changed {
            changed = false;
            for position in 0..candidates.len().saturating_sub(1) {
                let upper = &candidates[position];
                let lower = &candidates[position + 1];
                let promoted_test_over_source = is_test(upper)
                    && !is_test(lower)
                    && lexical_rank.get(&upper.file_id) > lexical_rank.get(&lower.file_id);
                if promoted_test_over_source {
                    candidates.swap(position, position + 1);
                    changed = true;
                }
            }
        }
    }

    Ok(())
}

#[cfg(not(feature = "embed"))]
#[allow(clippy::too_many_arguments)]
fn apply_hybrid_ranking(
    _root: &Path,
    _index: &CodeIndex,
    _task: &str,
    _query_tokens: &[String],
    _lookup: &IndexLookup<'_>,
    _candidates: &mut [ContextCandidate],
    hybrid: HybridOptions<'_>,
    _semantic_scores: Option<&()>,
    warnings: &mut Vec<String>,
) -> Result<()> {
    if hybrid.embeddings {
        warnings.push(
            "--embeddings requires building with --features embed; using lexical ranking"
                .to_string(),
        );
    }
    Ok(())
}

#[cfg(feature = "embed")]
fn normalized_lexical_score(score: i32, min_score: i32, max_score: i32) -> f64 {
    if max_score == min_score {
        // No spread in lexical scores. When there is no lexical signal at all
        // (all zero - e.g. a candidate set dominated by semantic-recall
        // injections), contribute nothing so the semantic term drives ordering.
        // When the scores are equal but positive the constant cancels out in the
        // blend, so 1.0 is harmless.
        if max_score == 0 { 0.0 } else { 1.0 }
    } else {
        f64::from(score - min_score) / f64::from(max_score - min_score)
    }
}

#[cfg(feature = "embed")]
fn normalize_vector(vector: &[f32]) -> Option<Vec<f32>> {
    let norm = vector
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>()
        .sqrt();
    if norm <= f64::EPSILON {
        return None;
    }
    Some(
        vector
            .iter()
            .map(|value| (*value as f64 / norm) as f32)
            .collect(),
    )
}

#[cfg(feature = "embed")]
fn cosine_with_unit_query(query_unit: &[f32], vector: &[f32]) -> Option<f32> {
    if query_unit.len() != vector.len() {
        return None;
    }
    let vector_unit = normalize_vector(vector)?;
    Some(
        query_unit
            .iter()
            .zip(vector_unit.iter())
            .map(|(left, right)| left * right)
            .sum::<f32>()
            .clamp(-1.0, 1.0),
    )
}

fn ownership_rank(
    candidate: &ContextCandidate,
    query_tokens: &[String],
    lookup: &IndexLookup<'_>,
) -> u8 {
    let Some(file) = lookup.file_by_id(&candidate.file_id) else {
        return 0;
    };
    let Some(ownership) = file.ownership.as_ref() else {
        return 0;
    };
    if ownership.is_empty() {
        return 0;
    }
    let mut owner_terms = BTreeSet::new();
    for owner in ownership.owners.iter().chain(ownership.teams.iter()) {
        let normalized = owner
            .trim_start_matches('@')
            .replace(['@', '/', '.', '-', '_'], " ")
            .to_ascii_lowercase();
        owner_terms.extend(formatter::tokenize(&normalized));
    }
    if query_tokens
        .iter()
        .any(|token| owner_terms.contains(token.as_str()))
    {
        2
    } else {
        1
    }
}

pub fn context_value(context: &ContextOutput, options: ContextViewOptions) -> Result<Value> {
    let mut value = match options.profile {
        ContextProfile::Skim => {
            skim_context_value(context, options.include_git, options.include_call_paths)
        }
        ContextProfile::Normal | ContextProfile::Full => serde_json::to_value(context)?,
    };
    if options.profile == ContextProfile::Normal {
        add_compact_impact_to_full_context(&mut value);
    } else if options.profile == ContextProfile::Full {
        remove_redundant_full_selection_summary(&mut value);
    }
    annotate_context_stats(&mut value, options.profile, options.token_budget, false)?;
    let trimmed = apply_context_token_budget(&mut value, options.token_budget)?;
    annotate_context_stats(&mut value, options.profile, options.token_budget, trimmed)?;
    Ok(value)
}

fn remove_redundant_full_selection_summary(value: &mut Value) {
    if let Some(summary) = value
        .get_mut("selection_summary")
        .and_then(Value::as_object_mut)
    {
        summary.remove("next_files");
    }
}

pub fn value_estimated_tokens(value: &Value) -> Result<usize> {
    Ok(estimate_tokens(&serde_json::to_string(value)?))
}

fn skim_context_value(
    context: &ContextOutput,
    include_git: bool,
    include_call_paths: bool,
) -> Value {
    let read_first_path_indexes = context
        .read_first
        .iter()
        .enumerate()
        .map(|(index, file)| (file.file.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let compact_top_selection_reason = context
        .selection_summary
        .top_reason
        .as_deref()
        .map(compact_reason_for_value);
    let read_first: Vec<Value> = context
        .read_first
        .iter()
        .enumerate()
        .map(|(index, file)| {
            let mut entry = serde_json::Map::new();
            entry.insert("f".to_string(), json!(file.file));
            let symbols = compact_symbols_for_value(&file.symbols);
            if !symbols.is_empty() {
                entry.insert("sy".to_string(), json!(symbols));
            }
            let mut why = compact_why_for_value(&file.why);
            if index == 0
                && let Some(top_reason) = &compact_top_selection_reason
            {
                why.retain(|reason| reason != top_reason);
            }
            if index == 0 && !why.is_empty() {
                entry.insert("w".to_string(), json!(why));
            }
            entry.insert(
                "i".to_string(),
                compact_impact_for_value(file, &read_first_path_indexes),
            );
            if include_git && let Some(git) = compact_git_for_value(file) {
                entry.insert("git".to_string(), git);
            }
            if index == 0
                && let Some(graph_hints) = compact_graph_hints_for_value(file)
            {
                entry.insert("g".to_string(), graph_hints);
            }
            if include_call_paths && let Some(call_paths) = compact_call_paths_for_value(file) {
                entry.insert("cp".to_string(), call_paths);
            }
            Value::Object(entry)
        })
        .collect();

    let selection_summary =
        compact_selection_summary_for_value(&context.selection_summary, &read_first_path_indexes);
    let mut value = json!({
        "root": context.root,
        "retrieval_cost": compact_retrieval_cost_for_value(&context.retrieval_cost),
        "read_first": read_first,
        "stats": {
            "local": compact_local_work_for_value(&context.stats.local_work)
        }
    });
    if should_include_skim_task(&context.task)
        && let Some(object) = value.as_object_mut()
    {
        object.insert("task".to_string(), json!(context.task));
    }
    if !selection_summary
        .as_object()
        .is_some_and(serde_json::Map::is_empty)
        && let Some(object) = value.as_object_mut()
    {
        object.insert("sel".to_string(), selection_summary);
    }
    if !context.warnings.is_empty()
        && let Some(object) = value.as_object_mut()
    {
        object.insert("warnings".to_string(), json!(context.warnings));
    }
    value
}

fn should_include_skim_task(task: &str) -> bool {
    task.contains("Follow-up:")
}

fn compact_selection_summary_for_value(
    summary: &ContextSelectionSummary,
    path_indexes: &BTreeMap<&str, usize>,
) -> Value {
    let mut value = serde_json::Map::new();
    if let Some(top_file) = &summary.top_file {
        let top = compact_selection_entry_for_value(
            top_file,
            summary.top_score,
            summary.top_reason.as_deref(),
            path_indexes,
        );
        value.insert("top".to_string(), Value::Array(top));
    }
    if !summary.top_signals.is_empty() {
        value.insert(
            "sig".to_string(),
            json!(compact_selection_score_components_for_value(
                &summary.top_signals
            )),
        );
    }
    let next_files = summary
        .next_files
        .iter()
        .take(MAX_SKIM_SELECTION_NEXT_FILES)
        .map(|file| {
            Value::Array(compact_selection_entry_for_value(
                &file.file,
                Some(file.score),
                file.reason.as_deref(),
                path_indexes,
            ))
        })
        .collect::<Vec<_>>();
    if !next_files.is_empty() {
        value.insert("next".to_string(), json!(next_files));
    }
    Value::Object(value)
}

fn compact_selection_entry_for_value(
    file: &str,
    score: Option<i32>,
    reason: Option<&str>,
    path_indexes: &BTreeMap<&str, usize>,
) -> Vec<Value> {
    if let Some(index) = path_indexes.get(file) {
        let mut entry = vec![json!(index)];
        if let Some(reason) = reason {
            entry.push(json!(compact_reason_for_value(reason)));
        }
        return entry;
    }

    let mut entry = vec![json!(file)];
    if score.is_some() || reason.is_some() {
        entry.push(json!(score.unwrap_or_default()));
    }
    if let Some(reason) = reason {
        entry.push(json!(compact_reason_for_value(reason)));
    }
    entry
}

fn compact_selection_score_components_for_value(
    components: &[SelectionScoreComponent],
) -> Vec<Value> {
    components
        .iter()
        .map(|component| json!(compact_selection_signal_for_value(&component.name)))
        .collect()
}

fn compact_selection_signal_for_value(name: &str) -> &str {
    match name {
        "exact_symbol" => "sym",
        "symbol_name_keyword_cluster" => "sy",
        "symbol_substring" => "sub",
        "keyword_overlap" => "kw",
        "path_filename" => "p",
        "path_keyword_overlap" => "pt",
        "module_anchor" => "mod",
        "path_intent_cluster" => "pi",
        "filename_keyword_cluster" => "fn",
        "content_keyword_overlap" => "ct",
        "test_file" => "tf",
        "test_proximity" => "test",
        "config_file" => "cfg",
        "config_dependency_intent" => "cfgdep",
        "dependency_manifest_intent" => "dep",
        "workflow_file_intent" => "wf",
        "index_freshness_surface" => "fresh",
        "benchmark_evidence_file_intent" => "bench",
        "benchmark_evidence_doc_intent" => "bdoc",
        "readme_evidence_file_intent" => "readme",
        "competitive_positioning_doc" => "comp",
        "ownership_context_attachment" => "own",
        "docs_intent" => "doc",
        "docs_path_intent" => "docp",
        "command_surface_intent" => "cmd",
        "hook_meta_intent" => "hook",
        "graph_imported_file" => "im",
        "graph_referencing_file" => "ref",
        "graph_callee" => "call",
        "graph_caller" => "caller",
        "stack_trace" => "trace",
        "git_signal" => "git",
        "semantic_recall" => "semr",
        "semantic_embedding" => "seme",
        _ => name,
    }
}

fn compact_why_for_value(reasons: &[String]) -> Vec<String> {
    let mut compact = Vec::new();
    for reason in reasons
        .iter()
        .take(2)
        .map(|reason| compact_reason_for_value(reason))
    {
        if compact_reason_is_redundant(&compact, &reason) {
            continue;
        }
        compact.push(reason);
        if compact.len() >= 2 {
            break;
        }
    }
    compact
}

fn compact_reason_is_redundant(existing: &[String], candidate: &str) -> bool {
    existing.iter().any(|reason| {
        reason == candidate
            || matching_symbol_and_keyword_terms(reason, candidate)
            || matching_symbol_and_keyword_terms(candidate, reason)
    })
}

fn matching_symbol_and_keyword_terms(left: &str, right: &str) -> bool {
    let Some(symbol_terms) = left.strip_prefix("sy:") else {
        return false;
    };
    right.strip_prefix("kw:") == Some(symbol_terms)
}

fn compact_reason_for_value(reason: &str) -> String {
    for (prefix, compact_prefix) in [
        ("path or filename match: ", "p:"),
        ("path keyword overlap: ", "pt:"),
        ("symbol name keyword cluster: ", "sy:"),
        ("keyword overlap: ", "kw:"),
        ("content keyword overlap: ", "ct:"),
        ("exact symbol match: ", "sym:"),
        ("symbol doc keyword cluster: ", "doc:"),
        ("import graph proximity: ", "im:"),
        ("reference graph proximity: ", "ref:"),
        ("call graph proximity: ", "call:"),
        ("test companion: ", "test:"),
    ] {
        if let Some(rest) = reason.strip_prefix(prefix) {
            return format!("{compact_prefix}{rest}");
        }
    }
    match reason {
        "dependency manifest intent" => "manifest intent".to_string(),
        _ => reason.to_string(),
    }
}

fn compact_retrieval_cost_for_value(retrieval_cost: &RetrievalCost) -> Value {
    json!({
        "retrieval_model_tokens": retrieval_cost.retrieval_model_tokens
    })
}

fn compact_symbols_for_value(symbols: &[QuerySymbol]) -> Vec<Value> {
    symbols
        .iter()
        .take(MAX_SKIM_SYMBOLS_PER_FILE)
        .map(|symbol| {
            let mut value = vec![json!(symbol.name)];
            value.push(json!(symbol.lines[0]));
            if symbol.kind != "function" {
                value.push(json!(compact_symbol_kind_for_value(&symbol.kind)));
            }
            Value::Array(value)
        })
        .collect()
}

fn compact_symbol_kind_for_value(kind: &str) -> &str {
    match kind {
        "class" => "cl",
        "method" => "m",
        "interface" => "if",
        "type" => "t",
        "struct" => "s",
        "enum" => "e",
        "trait" => "tr",
        "impl" => "im",
        "constant" => "c",
        "module" => "mod",
        "macro" => "mac",
        "component" => "cmp",
        _ => kind,
    }
}

/// Compact git hint for the skim packet: recency, hotness, and bus-factor in
/// three fields. `None` when the file has no recent history.
fn compact_git_for_value(file: &ContextFile) -> Option<Value> {
    let git = file.git.as_ref()?;
    Some(json!({
        "lm": git.last_modified_unix,
        "c90": git.commits_90d,
        "a90": git.distinct_authors_90d
    }))
}

fn compact_local_work_for_value(local_work: &LocalWorkStats) -> Value {
    let mut value = json!({
        "f": local_work.indexed_files,
        "sy": local_work.indexed_symbols,
        "r": local_work.indexed_references
    });
    if local_work.semantic_injected > 0
        && let Some(object) = value.as_object_mut()
    {
        object.insert("inj".to_string(), json!(local_work.semantic_injected));
    }
    value
}

fn compact_impact_for_value(file: &ContextFile, path_indexes: &BTreeMap<&str, usize>) -> Value {
    let mut tests = BTreeSet::new();
    tests.extend(file.blast_radius.tests.iter().cloned());
    tests.extend(file.related_tests.iter().map(|test| test.file.clone()));
    let tests = tests
        .into_iter()
        .take(MAX_CONTEXT_RELATED_TESTS)
        .collect::<Vec<_>>();
    let mut test_refs = Vec::new();
    for test in &tests {
        if let Some(index) = path_indexes.get(test.as_str()) {
            test_refs.push(json!(index));
        } else {
            test_refs.push(json!(test));
        }
    }
    let (upstream_count, downstream_count) = compact_impact_edge_counts_for_value(file);

    let mut impact = vec![json!(compact_risk_for_value(&file.blast_radius.risk))];
    if !test_refs.is_empty() {
        if test_refs.len() == 1 {
            impact.push(test_refs.remove(0));
        } else {
            impact.push(Value::Array(test_refs));
        }
    }
    if upstream_count > 0 || downstream_count > 0 {
        impact.push(json!(upstream_count));
    }
    if downstream_count > 0 {
        impact.push(json!(downstream_count));
    }
    if let Some(edge_flags) = compact_impact_edge_flags_for_value(file) {
        impact.push(json!(edge_flags));
    }
    Value::Array(impact)
}

fn compact_impact_edge_flags_for_value(file: &ContextFile) -> Option<String> {
    let mut flags = Vec::new();
    let has_tests = !file.blast_radius.tests.is_empty() || !file.related_tests.is_empty();
    if has_tests {
        flags.push("test");
    }
    if !file.blast_radius.imports.is_empty() || !file.imports.is_empty() {
        flags.push("im");
    }
    if !file.blast_radius.calls.is_empty() || !file.calls.is_empty() {
        flags.push("call");
    }
    if !file.blast_radius.referenced_by.is_empty() || !file.referenced_by.is_empty() {
        flags.push("ref");
    }
    if !file.blast_radius.called_by.is_empty() || !file.called_by.is_empty() {
        flags.push("by");
    }
    if flags.is_empty() {
        None
    } else {
        Some(flags.join(","))
    }
}

fn compact_impact_edge_counts_for_value(file: &ContextFile) -> (usize, usize) {
    let mut imports = BTreeSet::new();
    imports.extend(file.imports.iter().map(String::as_str));
    imports.extend(file.blast_radius.imports.iter().map(String::as_str));

    let mut call_targets = BTreeSet::new();
    call_targets.extend(file.blast_radius.calls.iter().map(String::as_str));
    for edge in &file.calls {
        if let Some(target_file) = edge.target_file.as_deref() {
            call_targets.insert(target_file);
        } else {
            call_targets.insert(edge.target.as_str());
        }
    }

    let mut referenced_by = BTreeSet::new();
    referenced_by.extend(file.referenced_by.iter().map(String::as_str));
    referenced_by.extend(file.blast_radius.referenced_by.iter().map(String::as_str));

    let mut called_by = BTreeSet::new();
    called_by.extend(file.blast_radius.called_by.iter().map(String::as_str));
    called_by.extend(file.called_by.iter().map(|edge| edge.file.as_str()));

    (
        imports.len() + call_targets.len(),
        referenced_by.len() + called_by.len(),
    )
}

fn compact_risk_for_value(risk: &str) -> &str {
    match risk {
        "low" => "l",
        "medium" => "m",
        "high" => "h",
        _ => risk,
    }
}

fn compact_graph_hints_for_value(file: &ContextFile) -> Option<Value> {
    let related_tests = file
        .related_tests
        .iter()
        .map(|test| test.file.as_str())
        .collect::<BTreeSet<_>>();
    let upstream = compact_graph_hint_paths(
        file.blast_radius
            .imports
            .iter()
            .chain(file.blast_radius.calls.iter()),
        &file.file,
        &related_tests,
    );
    let downstream = compact_graph_hint_paths(
        file.blast_radius
            .referenced_by
            .iter()
            .chain(file.blast_radius.called_by.iter()),
        &file.file,
        &related_tests,
    );

    if upstream.is_empty() && downstream.is_empty() {
        return None;
    }

    let mut hints = serde_json::Map::new();
    if !upstream.is_empty() {
        hints.insert("u".to_string(), json!(upstream));
    }
    if !downstream.is_empty() {
        hints.insert("d".to_string(), json!(downstream));
    }
    Some(Value::Object(hints))
}

fn compact_graph_hint_paths<'a>(
    paths: impl Iterator<Item = &'a String>,
    current_file: &str,
    related_tests: &BTreeSet<&str>,
) -> Vec<String> {
    paths
        .filter(|path| path.as_str() != current_file)
        .filter(|path| !related_tests.contains(path.as_str()))
        .filter(|path| !crate::indexer::is_test_file(path))
        .filter(|path| is_code_graph_hint_path(path))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(MAX_SKIM_GRAPH_HINTS_PER_DIRECTION)
        .collect()
}

fn compact_call_paths_for_value(file: &ContextFile) -> Option<Value> {
    let calls = compact_call_path_edges(
        file.calls.iter(),
        CompactCallDirection::Outgoing,
        &file.file,
    );
    let called_by = compact_call_path_edges(
        file.called_by.iter(),
        CompactCallDirection::Incoming,
        &file.file,
    );

    if calls.is_empty() && called_by.is_empty() {
        return None;
    }

    let mut paths = serde_json::Map::new();
    if !calls.is_empty() {
        paths.insert("c".to_string(), json!(calls));
    }
    if !called_by.is_empty() {
        paths.insert("by".to_string(), json!(called_by));
    }
    Some(Value::Object(paths))
}

#[derive(Clone, Copy)]
enum CompactCallDirection {
    Outgoing,
    Incoming,
}

fn compact_call_path_edges<'a>(
    edges: impl Iterator<Item = &'a ReferenceEdge>,
    direction: CompactCallDirection,
    current_file: &str,
) -> Vec<Value> {
    let mut seen = BTreeSet::new();
    let mut paths = Vec::new();
    for edge in edges {
        let related_file = match direction {
            CompactCallDirection::Outgoing => edge.target_file.as_deref(),
            CompactCallDirection::Incoming => Some(edge.file.as_str()),
        };
        let Some(related_file) = related_file else {
            continue;
        };
        if related_file == current_file || !is_code_graph_hint_path(related_file) {
            continue;
        }
        let from = edge.symbol.as_deref().unwrap_or_default();
        let key = format!("{related_file}\0{from}\0{}\0{}", edge.target, edge.line);
        if !seen.insert(key) {
            continue;
        }

        let mut item = serde_json::Map::new();
        item.insert("f".to_string(), json!(related_file));
        if !from.is_empty() {
            item.insert("fr".to_string(), json!(from));
        }
        item.insert("t".to_string(), json!(edge.target));
        item.insert("l".to_string(), json!(edge.line));
        paths.push(Value::Object(item));
        if paths.len() >= MAX_SKIM_CALL_PATHS_PER_DIRECTION {
            break;
        }
    }
    paths
}

fn is_code_graph_hint_path(path: &str) -> bool {
    matches!(
        Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str()),
        Some(
            "ts" | "tsx"
                | "js"
                | "jsx"
                | "mjs"
                | "cjs"
                | "py"
                | "rs"
                | "go"
                | "java"
                | "kt"
                | "kts"
                | "swift"
                | "c"
                | "cc"
                | "cpp"
                | "h"
                | "hpp"
                | "cs"
                | "rb"
                | "php"
                | "svelte"
                | "vue"
        )
    )
}

fn add_compact_impact_to_full_context(value: &mut Value) {
    let Some(files) = value.get_mut("read_first").and_then(Value::as_array_mut) else {
        return;
    };

    for file in files {
        let risk = file
            .get("blast_radius")
            .and_then(|blast| blast.get("risk"))
            .cloned()
            .unwrap_or_else(|| json!("unknown"));
        let tests = file
            .get("blast_radius")
            .and_then(|blast| blast.get("tests"))
            .cloned()
            .unwrap_or_else(|| json!([]));
        let upstream_count = file
            .get("imports")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or_default()
            + file
                .get("calls")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or_default();
        let downstream_count = file
            .get("referenced_by")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or_default()
            + file
                .get("called_by")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or_default();
        if let Some(object) = file.as_object_mut() {
            object.insert(
                "impact".to_string(),
                json!({
                    "risk": risk,
                    "tests": tests,
                    "upstream_count": upstream_count,
                    "downstream_count": downstream_count
                }),
            );
        }
    }
}

fn annotate_context_stats(
    value: &mut Value,
    profile: ContextProfile,
    token_budget: Option<usize>,
    trimmed: bool,
) -> Result<()> {
    if !value.get("stats").is_some_and(Value::is_object)
        && let Some(object) = value.as_object_mut()
    {
        object.insert("stats".to_string(), json!({}));
    }
    if let Some(stats) = value.get_mut("stats").and_then(Value::as_object_mut) {
        if profile == ContextProfile::Skim {
            if trimmed {
                stats.insert("trimmed".to_string(), json!(true));
            } else {
                stats.remove("trimmed");
            }
            if let Some(token_budget) = token_budget {
                stats.insert("b".to_string(), json!(token_budget));
            } else {
                stats.remove("b");
            }
            for field in [
                "candidate_matches",
                "profile",
                "selected_files",
                "selected_symbols",
                "related_tests",
                "budget",
                "token_budget",
                "estimated_tokens",
                "tokens",
                "local_work",
            ] {
                stats.remove(field);
            }
        } else {
            stats.insert("profile".to_string(), json!(profile.as_str()));
            stats.insert("trimmed".to_string(), json!(trimmed));
            if let Some(token_budget) = token_budget {
                stats.insert("token_budget".to_string(), json!(token_budget));
            } else {
                stats.remove("token_budget");
            }
            stats.remove("estimated_tokens");
        }
    }
    let estimated_tokens = value_estimated_tokens(value)?;
    if let Some(stats) = value.get_mut("stats").and_then(Value::as_object_mut) {
        let token_field = if profile == ContextProfile::Skim {
            "t"
        } else {
            "estimated_tokens"
        };
        stats.insert(token_field.to_string(), json!(estimated_tokens));
    }
    Ok(())
}

fn apply_context_token_budget(value: &mut Value, token_budget: Option<usize>) -> Result<bool> {
    let Some(token_budget) = token_budget else {
        return Ok(false);
    };
    if value_estimated_tokens(value)? <= token_budget {
        return Ok(false);
    }

    trim_context_detail(value);
    if value_estimated_tokens(value)? <= token_budget {
        return Ok(true);
    }

    while value_estimated_tokens(value)? > token_budget {
        let Some(files) = value.get_mut("read_first").and_then(Value::as_array_mut) else {
            break;
        };
        if files.len() <= 1 {
            break;
        }
        files.pop();
    }
    trim_selection_summary_to_read_first(value);

    let selected_files = value
        .get("read_first")
        .and_then(Value::as_array)
        .map(Vec::len);
    if let Some(stats) = value.get_mut("stats").and_then(Value::as_object_mut)
        && stats.contains_key("selected_files")
        && let Some(selected_files) = selected_files
    {
        stats.insert("selected_files".to_string(), json!(selected_files));
    }

    Ok(true)
}

pub fn trim_selection_summary_to_read_first(value: &mut Value) {
    let selected_len = value
        .get("read_first")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    let selected: BTreeSet<String> = value
        .get("read_first")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|file| {
            file.get("f")
                .or_else(|| file.get("file"))
                .and_then(Value::as_str)
        })
        .map(str::to_string)
        .collect();
    if selected.is_empty() {
        return;
    }
    let summary = if value.get("sel").is_some() {
        value.get_mut("sel")
    } else {
        value.get_mut("selection_summary")
    };
    let Some(summary) = summary.and_then(Value::as_object_mut) else {
        return;
    };
    let next_files = if summary.get("next").is_some() {
        summary.get_mut("next")
    } else {
        summary.get_mut("next_files")
    };
    let Some(next_files) = next_files.and_then(Value::as_array_mut) else {
        return;
    };
    next_files.retain(|file| {
        selection_summary_file_index(file).is_some_and(|index| index < selected_len)
            || selection_summary_file_path(file).is_some_and(|path| selected.contains(path))
    });
}

fn selection_summary_file_index(file: &Value) -> Option<usize> {
    file.as_u64()
        .and_then(|index| usize::try_from(index).ok())
        .or_else(|| {
            file.as_array()
                .and_then(|items| items.first())
                .and_then(Value::as_u64)
                .and_then(|index| usize::try_from(index).ok())
        })
}

fn selection_summary_file_path(file: &Value) -> Option<&str> {
    file.get("f")
        .or_else(|| file.get("file"))
        .and_then(Value::as_str)
        .or_else(|| {
            file.as_array()
                .and_then(|items| items.first())
                .and_then(Value::as_str)
        })
}

fn trim_context_detail(value: &mut Value) {
    let Some(files) = value.get_mut("read_first").and_then(Value::as_array_mut) else {
        return;
    };

    for file in files {
        let Some(object) = file.as_object_mut() else {
            continue;
        };
        object.remove("snippets");
        object.remove("calls");
        object.remove("called_by");
        object.remove("imports");
        object.remove("referenced_by");
        object.remove("why_debug");
        object.remove("graph_hints");
        object.remove("call_paths");
        if let Some(why) = object.get_mut("why").and_then(Value::as_array_mut) {
            why.truncate(2);
        }
        if let Some(symbols) = object.get_mut("symbols").and_then(Value::as_array_mut) {
            for symbol in symbols {
                if let Some(symbol) = symbol.as_object_mut() {
                    symbol.remove("signature");
                    symbol.remove("visibility");
                }
            }
        }
        if let Some(tests) = object
            .get_mut("related_tests")
            .and_then(Value::as_array_mut)
        {
            for test in tests {
                if let Some(test) = test.as_object_mut() {
                    test.remove("symbols");
                }
            }
        }
        if let Some(blast_radius) = object
            .get_mut("blast_radius")
            .and_then(Value::as_object_mut)
        {
            let risk = blast_radius
                .get("risk")
                .cloned()
                .unwrap_or_else(|| json!("unknown"));
            let tests = blast_radius
                .get("tests")
                .cloned()
                .unwrap_or_else(|| json!([]));
            blast_radius.clear();
            blast_radius.insert("risk".to_string(), risk);
            blast_radius.insert("tests".to_string(), tests);
        }
    }
}

fn promote_implementation_companion(
    lookup: &IndexLookup<'_>,
    candidates: &mut Vec<ContextCandidate>,
    limit: usize,
    query_tokens: &[String],
) {
    let selected_len = limit.min(candidates.len());
    if selected_len < 2 {
        return;
    }
    let selected_are_all_tests = candidates.iter().take(selected_len).all(|candidate| {
        lookup
            .file_by_id(&candidate.file_id)
            .is_some_and(|file| file.is_test)
    });
    if !selected_are_all_tests {
        return;
    }

    let Some(companion_index) =
        candidates
            .iter()
            .enumerate()
            .skip(selected_len)
            .find_map(|(index, candidate)| {
                let file = lookup.file_by_id(&candidate.file_id)?;
                (!file.is_test).then_some(index)
            })
    else {
        return;
    };

    let replacement_index = (0..selected_len)
        .min_by_key(|&index| {
            let candidate = &candidates[index];
            let specificity = lookup
                .file_by_id(&candidate.file_id)
                .map(|file| test_candidate_specificity(file, candidate, query_tokens))
                .unwrap_or_default();
            (specificity, candidate.score(), std::cmp::Reverse(index))
        })
        .unwrap_or(selected_len - 1);

    let companion = candidates.remove(companion_index);
    candidates.remove(replacement_index);
    candidates.insert(replacement_index, companion);
}

fn test_candidate_specificity(
    file: &FileRecord,
    candidate: &ContextCandidate,
    query_tokens: &[String],
) -> i32 {
    let path_lower = file.path.to_ascii_lowercase();
    let mut terms: BTreeSet<String> = formatter::tokenize(&file.path).into_iter().collect();
    for reason in &candidate.why {
        terms.extend(formatter::tokenize(reason));
    }

    query_tokens
        .iter()
        .filter(|token| !is_generic_test_specificity_token(token))
        .map(|token| {
            let mut score = 0;
            if terms.contains(token.as_str()) {
                score += 1;
            }
            if path_lower.contains(token.as_str()) {
                score += 1;
            }
            if candidate
                .why
                .iter()
                .any(|reason| reason.to_ascii_lowercase().contains(token.as_str()))
            {
                score += 1;
            }
            score
        })
        .sum()
}

fn promote_task_specific_test_companion(
    lookup: &IndexLookup<'_>,
    candidates: &mut Vec<ContextCandidate>,
    limit: usize,
    query_tokens: &[String],
) {
    let selected_len = limit.min(candidates.len());
    if selected_len < 2 {
        return;
    }
    let selected_has_test = candidates.iter().take(selected_len).any(|candidate| {
        lookup
            .file_by_id(&candidate.file_id)
            .is_some_and(|file| file.is_test)
    });
    if selected_has_test {
        return;
    }

    let Some(companion_index) = candidates
        .iter()
        .enumerate()
        .skip(selected_len)
        .filter_map(|(index, candidate)| {
            let file = lookup.file_by_id(&candidate.file_id)?;
            if !file.is_test || is_test_init_file(&file.path) {
                return None;
            }
            let specificity = test_candidate_specificity(file, candidate, query_tokens);
            (specificity >= MIN_TASK_SPECIFIC_TEST_SCORE).then_some((
                index,
                specificity,
                candidate.score(),
            ))
        })
        .max_by_key(|(_, specificity, score)| (*specificity, *score))
        .map(|(index, _, _)| index)
    else {
        return;
    };

    // Tie-break on the candidate's original rank, not its current position:
    // hybrid reranking reshuffles positions, and a position-sensitive victim
    // choice made the lexical and hybrid arms evict different files on score
    // ties (astropy-14182 lost its correct rank-5 file only in hybrid).
    let replacement_index = (1..selected_len)
        .min_by_key(|&index| {
            let candidate = &candidates[index];
            let keep_priority = lookup
                .file_by_id(&candidate.file_id)
                .map(|file| selected_non_test_keep_priority(file, candidate, query_tokens))
                .unwrap_or_default();
            (
                keep_priority,
                candidate.score(),
                std::cmp::Reverse(candidate.first_rank),
            )
        })
        .unwrap_or(selected_len - 1);

    let companion = candidates.remove(companion_index);
    candidates.remove(replacement_index);
    candidates.insert(replacement_index, companion);
}

fn selected_non_test_keep_priority(
    file: &FileRecord,
    candidate: &ContextCandidate,
    query_tokens: &[String],
) -> i32 {
    let exact_match_bonus =
        candidate.why.iter().any(|reason| {
            reason.starts_with("exact symbol match") || reason.starts_with("exact path")
        }) as i32
            * 15;
    let direct_surface_bonus = candidate_has_direct_surface_signal(candidate) as i32 * 25;
    let domain_module_bonus = candidate
        .why
        .iter()
        .any(|reason| reason == "domain module alias intent") as i32
        * 50;
    let agent_facing_doc_bonus = is_agent_facing_doc_path(&file.path) as i32 * 25;
    let benchmark_evidence_artifact_bonus =
        is_benchmark_evidence_artifact_path(&file.path) as i32 * 25;
    let code_bonus = file.language.is_code() as i32 * 10;
    let graph_only_penalty = candidate_is_graph_only(candidate) as i32 * -20;
    test_candidate_specificity(file, candidate, query_tokens)
        + exact_match_bonus
        + direct_surface_bonus
        + domain_module_bonus
        + agent_facing_doc_bonus
        + benchmark_evidence_artifact_bonus
        + code_bonus
        + graph_only_penalty
}

fn candidate_has_direct_surface_signal(candidate: &ContextCandidate) -> bool {
    candidate.why.iter().any(|reason| {
        reason.starts_with("exact symbol match")
            || reason.starts_with("exact path")
            || reason.starts_with("path or filename match")
            || reason.starts_with("path keyword overlap")
            || reason.starts_with("filename keyword cluster")
            || reason.starts_with("path intent keyword cluster")
            || reason.starts_with("symbol name keyword cluster")
            || reason.starts_with("content keyword overlap")
            || reason == "docs intent"
            || reason == "docs path intent"
            || reason == "command surface intent"
            || reason == "hook doctor and lifecycle implementation intent"
    })
}

fn candidate_is_graph_only(candidate: &ContextCandidate) -> bool {
    !candidate_has_direct_surface_signal(candidate)
}

fn is_agent_facing_doc_path(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    path.starts_with("docs/")
        || matches!(
            path.as_str(),
            "readme.md" | "agents.md" | "claude.md" | "product_brief.md"
        )
}

fn is_benchmark_evidence_artifact_path(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    path.starts_with("benchmarks/evidence/") && path.ends_with(".json")
}

fn is_test_init_file(path: &str) -> bool {
    path.ends_with("/__init__.py") || path.ends_with("/__init__.rs") || path.ends_with("/mod.rs")
}

fn is_generic_test_specificity_token(token: &str) -> bool {
    matches!(token, "test" | "tests" | "spec" | "specs")
}

pub fn benchmark_context_payload_reduction_value(
    benchmark: &BenchmarkOutput,
) -> Result<serde_json::Value> {
    serde_json::to_value(&benchmark.context_payload_reduction)
        .context("failed to serialize context payload reduction")
}

/// Fold an observed session into task memory: the files the agent actually
/// read after receiving context become confirmed associations for this task.
/// Upserts so hook-driven sessions (which bypass `task_memory_for_context`)
/// still learn. Returns the number of confirmed files stored.
pub fn confirm_task_memory_reads(
    root: &Path,
    task: &str,
    packet_files: &[String],
    confirmed_reads: &[String],
    client: &str,
    created_at: u64,
) -> Result<usize> {
    if task.trim().is_empty() || confirmed_reads.is_empty() {
        return Ok(0);
    }
    let path = task_memory_path(root);
    let mut memory = load_task_memory(&path);
    let confirmed: Vec<String> = confirmed_reads
        .iter()
        .filter(|read| !read.trim().is_empty())
        .take(MAX_TASK_MEMORY_RECOMMENDED_FILES)
        .cloned()
        .collect();
    if confirmed.is_empty() {
        return Ok(0);
    }

    let stored;
    if let Some(entry) = memory.entries.iter_mut().find(|entry| entry.task == task) {
        let mut inserted = 0usize;
        for file in &confirmed {
            if !entry.confirmed_files.contains(file) {
                entry.confirmed_files.push(file.clone());
                inserted += 1;
            }
        }
        // Newest evidence wins: drop from the FRONT (oldest) on overflow so
        // fresh observations are never silently discarded.
        if entry.confirmed_files.len() > MAX_TASK_MEMORY_RECOMMENDED_FILES {
            let excess = entry.confirmed_files.len() - MAX_TASK_MEMORY_RECOMMENDED_FILES;
            entry.confirmed_files.drain(0..excess);
        }
        stored = inserted;
        if entry.client.is_empty() {
            entry.client = client.to_string();
        }
    } else {
        stored = confirmed.len();
        memory.entries.push(TaskMemoryEntry {
            task: task.to_string(),
            task_terms: task_memory_terms(task),
            created_at,
            read_first_files: packet_files
                .iter()
                .take(MAX_TASK_MEMORY_RECOMMENDED_FILES)
                .cloned()
                .collect(),
            symbols: Vec::new(),
            tests: Vec::new(),
            confirmed_files: confirmed,
            client: client.to_string(),
        });
        if memory.entries.len() > MAX_TASK_MEMORY_ENTRIES {
            let excess = memory.entries.len() - MAX_TASK_MEMORY_ENTRIES;
            memory.entries.drain(0..excess);
        }
    }
    save_task_memory(&path, &memory)?;
    Ok(stored)
}

/// Session-confirmed associations from task memory boost matching candidates.
/// Only strong matches count (the existing >=2-shared-terms / 0.4-jaccard
/// threshold), and only files an agent verifiably read in an observed session.
const MEMORY_CONFIRMED_BOOST: i32 = 80;

fn add_memory_confirmed_boost(
    root: &Path,
    task: &str,
    lookup: &IndexLookup<'_>,
    candidates: &mut Vec<ContextCandidate>,
) {
    let memory = load_task_memory(&task_memory_path(root));
    let terms = task_memory_terms(task);
    let similar = similar_task_memory_entries(&memory.entries, &terms);
    let mut confirmed: BTreeMap<&str, &str> = BTreeMap::new();
    for scored in &similar {
        for file in &scored.entry.confirmed_files {
            confirmed.entry(file.as_str()).or_insert(&scored.entry.task);
        }
    }
    if confirmed.is_empty() {
        return;
    }
    // Boost matching candidates; confirmed files lexical never surfaced are
    // injected — an observed read for a matched task is direct evidence,
    // unlike speculative semantic injection.
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for candidate in candidates.iter_mut() {
        let Some(file) = lookup.file_by_id(&candidate.file_id) else {
            continue;
        };
        seen.insert(file.path.as_str());
        if let Some(teaching_task) = confirmed.get(file.path.as_str()) {
            candidate.add_consensus_boost(
                MEMORY_CONFIRMED_BOOST,
                format!("agent-confirmed read in a similar session: {teaching_task}"),
            );
        }
    }
    let missing: Vec<(String, String)> = confirmed
        .iter()
        .filter(|(path, _)| !seen.contains(*path))
        .filter_map(|(path, teaching_task)| {
            lookup
                .file_by_path(path)
                .map(|file| (file.id.clone(), (*teaching_task).to_string()))
        })
        .collect();
    for (file_id, teaching_task) in missing {
        let mut candidate = ContextCandidate::new(file_id, 0, usize::MAX);
        candidate.add_consensus_boost(
            MEMORY_CONFIRMED_BOOST,
            format!("agent-confirmed read in a similar session: {teaching_task}"),
        );
        candidates.push(candidate);
    }
}

/// Merge an exported memory store into this repo's store: entries are keyed
/// by task identity, the newer `created_at` wins, confirmed files union, and
/// the FIFO cap holds. Returns (imported, merged_total).
pub fn merge_task_memory(root: &Path, imported: &str) -> Result<(usize, usize)> {
    let incoming: TaskMemoryStore =
        serde_json::from_str(imported).context("failed to parse exported task memory")?;
    let path = task_memory_path(root);
    let mut memory = load_task_memory(&path);
    let mut imported_count = 0usize;
    for entry in incoming.entries {
        if let Some(existing) = memory
            .entries
            .iter_mut()
            .find(|existing| existing.task == entry.task)
        {
            for file in &entry.confirmed_files {
                if !existing.confirmed_files.contains(file) {
                    existing.confirmed_files.push(file.clone());
                }
            }
            existing
                .confirmed_files
                .truncate(MAX_TASK_MEMORY_RECOMMENDED_FILES);
            if entry.created_at > existing.created_at {
                existing.created_at = entry.created_at;
                existing.read_first_files = entry.read_first_files;
                existing.symbols = entry.symbols;
                existing.tests = entry.tests;
            }
            if existing.client.is_empty() {
                existing.client = entry.client;
            }
        } else {
            memory.entries.push(entry);
        }
        imported_count += 1;
    }
    memory.entries.sort_by_key(|entry| entry.created_at);
    if memory.entries.len() > MAX_TASK_MEMORY_ENTRIES {
        let excess = memory.entries.len() - MAX_TASK_MEMORY_ENTRIES;
        memory.entries.drain(0..excess);
    }
    let total = memory.entries.len();
    save_task_memory(&path, &memory)?;
    Ok((imported_count, total))
}

/// Serialized form of this repo's task memory for `memory-export`.
pub fn export_task_memory(root: &Path) -> Result<(String, usize)> {
    let memory = load_task_memory(&task_memory_path(root));
    let count = memory.entries.len();
    Ok((serde_json::to_string_pretty(&memory)?, count))
}

pub fn task_memory_path(root: &Path) -> PathBuf {
    root.join(store::json_store::INDEX_DIR)
        .join(TASK_MEMORY_FILE)
}

pub fn clear_task_memory(root: &Path) -> Result<bool> {
    let path = task_memory_path(root);
    if !path.is_file() {
        return Ok(false);
    }
    fs::remove_file(&path).with_context(|| format!("failed to remove {}", path.display()))?;
    Ok(true)
}

pub fn latest_task_memory_task(root: &Path) -> Option<String> {
    let path = task_memory_path(root);
    load_task_memory(&path)
        .entries
        .into_iter()
        .max_by_key(|entry| entry.created_at)
        .map(|entry| entry.task)
}

pub fn task_memory_for_context(
    root: &Path,
    context: &ContextOutput,
    created_at: u64,
) -> Result<TaskMemoryOutput> {
    let path = task_memory_path(root);
    let mut memory = load_task_memory(&path);
    let current_terms = task_memory_terms(&context.task);
    let similar = similar_task_memory_entries(&memory.entries, &current_terms);
    let similar_tasks: Vec<TaskMemorySimilarTask> = similar
        .iter()
        .map(|scored| TaskMemorySimilarTask {
            task: scored.entry.task.clone(),
            score: rounded_score(scored.score),
            client: scored.entry.client.clone(),
            shared_terms: scored.shared_terms.clone(),
            read_first_files: if scored.entry.read_first_files.is_empty() {
                scored
                    .entry
                    .confirmed_files
                    .iter()
                    .take(MAX_TASK_MEMORY_RECOMMENDED_FILES)
                    .cloned()
                    .collect()
            } else {
                scored
                    .entry
                    .read_first_files
                    .iter()
                    .take(MAX_TASK_MEMORY_RECOMMENDED_FILES)
                    .cloned()
                    .collect()
            },
        })
        .collect();
    let recommended_files = recommended_task_memory_files(&similar);
    let recommended_symbols = recommended_task_memory_symbols(&similar);

    let mut entry = task_memory_entry_from_context(context, current_terms, created_at);
    if !entry.read_first_files.is_empty() {
        // Re-running the same task must refresh recency without erasing what
        // observed sessions taught: carry learned fields into the new entry.
        if let Some(previous) = memory
            .entries
            .iter()
            .find(|existing| existing.task == entry.task)
        {
            entry.confirmed_files = previous.confirmed_files.clone();
            entry.client = previous.client.clone();
        }
        memory.entries.retain(|existing| {
            existing.task != entry.task || existing.read_first_files != entry.read_first_files
        });
        memory.entries.push(entry);
        if memory.entries.len() > MAX_TASK_MEMORY_ENTRIES {
            let excess = memory.entries.len() - MAX_TASK_MEMORY_ENTRIES;
            memory.entries.drain(0..excess);
        }
        save_task_memory(&path, &memory)?;
    }

    Ok(TaskMemoryOutput {
        cache_hit: !similar_tasks.is_empty(),
        path: path.display().to_string(),
        policy: "local_project_memory_only; use as hints, not proof",
        similar_tasks,
        recommended_files,
        recommended_symbols,
    })
}

fn load_task_memory(path: &Path) -> TaskMemoryStore {
    let Ok(data) = fs::read(path) else {
        return TaskMemoryStore::default();
    };
    serde_json::from_slice(&data).unwrap_or_default()
}

fn save_task_memory(path: &Path, memory: &TaskMemoryStore) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, serde_json::to_vec_pretty(memory)?)
        .with_context(|| format!("failed to write task memory {}", path.display()))
}

fn task_memory_entry_from_context(
    context: &ContextOutput,
    task_terms: Vec<String>,
    created_at: u64,
) -> TaskMemoryEntry {
    let mut read_first_files = Vec::new();
    let mut symbols = Vec::new();
    let mut tests = Vec::new();

    for file in &context.read_first {
        push_unique(
            &mut read_first_files,
            file.file.clone(),
            MAX_TASK_MEMORY_RECOMMENDED_FILES,
        );
        for symbol in &file.symbols {
            push_unique(
                &mut symbols,
                symbol.name.clone(),
                MAX_TASK_MEMORY_RECOMMENDED_SYMBOLS * 2,
            );
        }
        for test in &file.related_tests {
            push_unique(&mut tests, test.file.clone(), MAX_CONTEXT_RELATED_TESTS * 2);
        }
    }

    TaskMemoryEntry {
        confirmed_files: Vec::new(),
        client: String::new(),
        task: context.task.clone(),
        task_terms,
        created_at,
        read_first_files,
        symbols,
        tests,
    }
}

fn task_memory_terms(task: &str) -> Vec<String> {
    let mut terms = formatter::tokenize(task);
    terms.sort();
    terms.dedup();
    terms
}

fn similar_task_memory_entries<'a>(
    entries: &'a [TaskMemoryEntry],
    current_terms: &[String],
) -> Vec<ScoredTaskMemoryEntry<'a>> {
    let current_set: BTreeSet<&str> = current_terms.iter().map(String::as_str).collect();
    let mut scored: Vec<ScoredTaskMemoryEntry<'a>> = entries
        .iter()
        .filter_map(|entry| {
            if entry.read_first_files.is_empty() && entry.confirmed_files.is_empty() {
                return None;
            }
            let entry_set: BTreeSet<&str> = entry.task_terms.iter().map(String::as_str).collect();
            let shared_terms: Vec<String> = current_set
                .intersection(&entry_set)
                .map(|term| (*term).to_string())
                .collect();
            let denominator = current_set.len().max(entry_set.len()).max(1);
            let score = shared_terms.len() as f64 / denominator as f64;
            if shared_terms.len() >= 2 || score >= 0.4 {
                Some(ScoredTaskMemoryEntry {
                    entry,
                    score,
                    shared_terms,
                })
            } else {
                None
            }
        })
        .collect();
    scored.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| right.entry.created_at.cmp(&left.entry.created_at))
            .then_with(|| left.entry.task.cmp(&right.entry.task))
    });
    scored.truncate(MAX_TASK_MEMORY_SIMILAR_TASKS);
    scored
}

fn recommended_task_memory_files(similar: &[ScoredTaskMemoryEntry<'_>]) -> Vec<String> {
    let mut files = Vec::new();
    for scored in similar {
        for file in &scored.entry.read_first_files {
            push_unique(&mut files, file.clone(), MAX_TASK_MEMORY_RECOMMENDED_FILES);
        }
    }
    files
}

fn recommended_task_memory_symbols(similar: &[ScoredTaskMemoryEntry<'_>]) -> Vec<String> {
    let mut symbols = Vec::new();
    for scored in similar {
        for symbol in &scored.entry.symbols {
            push_unique(
                &mut symbols,
                symbol.clone(),
                MAX_TASK_MEMORY_RECOMMENDED_SYMBOLS,
            );
        }
    }
    symbols
}

fn push_unique(values: &mut Vec<String>, value: String, limit: usize) {
    if values.len() >= limit || values.iter().any(|existing| existing == &value) {
        return;
    }
    values.push(value);
}

fn rounded_score(score: f64) -> f64 {
    (score * 1000.0).round() / 1000.0
}

pub fn benchmark_context(
    root: &Path,
    index: &CodeIndex,
    task: &str,
    limit: usize,
    snippets_per_file: usize,
    include_snippets: bool,
) -> Result<BenchmarkOutput> {
    benchmark_context_with_options(
        root,
        index,
        task,
        limit,
        snippets_per_file,
        include_snippets,
        ContextViewOptions {
            profile: ContextProfile::Full,
            token_budget: None,
            include_git: false,
            include_call_paths: false,
        },
    )
}

pub fn benchmark_context_with_options(
    root: &Path,
    index: &CodeIndex,
    task: &str,
    limit: usize,
    snippets_per_file: usize,
    include_snippets: bool,
    view_options: ContextViewOptions,
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
    let packet_value = context_value(&context, view_options)?;
    let packet = serde_json::to_string(&packet_value)?;
    let packet_tokens = estimate_tokens(&packet);
    let packet_quality = context_packet_quality(&context);
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
        packet_quality,
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

fn context_packet_quality(context: &ContextOutput) -> ContextPacketQuality {
    let mut quality = ContextPacketQuality {
        tasks: 1,
        selected_files: context.read_first.len(),
        selected_symbols: context.stats.selected_symbols,
        related_tests: context.stats.related_tests,
        selection_signals: context.selection_summary.top_signals.len(),
        next_file_hints: context.selection_summary.next_files.len(),
        ..ContextPacketQuality::default()
    };

    for file in &context.read_first {
        if !file.symbols.is_empty() {
            quality.files_with_symbols += 1;
        }
        if !file.snippets.is_empty() {
            quality.files_with_snippets += 1;
            quality.snippets += file.snippets.len();
        }
        if !file.related_tests.is_empty() {
            quality.files_with_related_tests += 1;
        }
        if file.blast_radius.risk != "unknown" {
            quality.files_with_non_unknown_risk += 1;
        }
        if !file.why.is_empty() {
            quality.files_with_selection_reasons += 1;
            quality.selection_reasons += file.why.len();
        }
        if !file.selection_confidence.is_empty() {
            quality.files_with_selection_confidence += 1;
        }
        if file.language.is_code() {
            quality.focus_targets += 1;
        }
        let blast_radius_hints = file.imports.len()
            + file.referenced_by.len()
            + file.calls.len()
            + file.called_by.len()
            + file.blast_radius.imports.len()
            + file.blast_radius.referenced_by.len()
            + file.blast_radius.tests.len()
            + file.blast_radius.calls.len()
            + file.blast_radius.called_by.len();
        if blast_radius_hints > 0 {
            quality.files_with_blast_radius += 1;
            quality.blast_radius_hints += blast_radius_hints;
        }
        let call_graph_hints = file.calls.len()
            + file.called_by.len()
            + file.blast_radius.calls.len()
            + file.blast_radius.called_by.len();
        if call_graph_hints > 0 {
            quality.files_with_call_graph_hints += 1;
            quality.call_graph_hints += call_graph_hints;
        }
    }

    if context
        .read_first
        .first()
        .is_some_and(|file| file.language.is_code())
    {
        quality.relationship_followup_targets = 1;
        quality.test_followup_targets = 1;
    }

    quality
}

pub fn benchmark_suite(
    root: &Path,
    index: &CodeIndex,
    suite: BenchmarkSuiteInput,
    limit: usize,
    snippets_per_file: usize,
    include_snippets: bool,
) -> Result<BenchmarkSuiteOutput> {
    benchmark_suite_with_options(
        root,
        index,
        suite,
        limit,
        snippets_per_file,
        include_snippets,
        ContextViewOptions {
            profile: ContextProfile::Full,
            token_budget: None,
            include_git: false,
            include_call_paths: false,
        },
    )
}

pub fn benchmark_suite_with_options(
    root: &Path,
    index: &CodeIndex,
    suite: BenchmarkSuiteInput,
    limit: usize,
    snippets_per_file: usize,
    include_snippets: bool,
    view_options: ContextViewOptions,
) -> Result<BenchmarkSuiteOutput> {
    let mut task_outputs = Vec::new();
    let mut total_expected_files = 0;
    let mut total_expected_files_found = 0;
    let mut tasks_with_all_expected_files = 0;
    let mut first_correct_file_hits = 0;
    let mut first_correct_file_tasks = 0;
    let mut total_estimated_token_savings = 0;
    let mut total_baseline_context_payload_tokens = 0;
    let mut total_callsieve_context_payload_tokens = 0;
    let mut total_estimated_reduction_percent = 0.0;
    let mut total_estimated_avoided_grep_commands = 0;
    let mut total_estimated_avoided_file_reads = 0;
    let mut packet_quality = ContextPacketQuality::default();
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
        let benchmark = benchmark_context_with_options(
            root,
            index,
            &task,
            limit,
            snippets_per_file,
            include_snippets,
            view_options,
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
        let first_correct_file = first_correct_file(&selected_files, &task_expected_files);
        let first_correct_file_hit = first_correct_file.is_some();
        if expected_files_for_task > 0 {
            first_correct_file_tasks += 1;
            if first_correct_file_hit {
                first_correct_file_hits += 1;
            }
        }
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
        packet_quality.add(&benchmark.callsieve.packet_quality);

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
            first_correct_file_hit,
            first_correct_file: first_correct_file.as_ref().map(|(file, _)| file.clone()),
            first_correct_file_rank: first_correct_file.map(|(_, rank)| rank),
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
        first_correct_file_hits,
        first_correct_file_tasks,
        first_correct_file_rate_at_k: recall(first_correct_file_hits, first_correct_file_tasks),
        baseline_context_payload_tokens_estimate: total_baseline_context_payload_tokens,
        callsieve_context_payload_tokens_estimate: total_callsieve_context_payload_tokens,
        context_payload_reduction: context_payload_reduction(
            total_baseline_context_payload_tokens,
            total_callsieve_context_payload_tokens,
        ),
        packet_quality,
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
    eval_retrieval_with_options(
        root,
        index,
        suite,
        limit,
        snippets_per_file,
        include_snippets,
        ContextViewOptions {
            profile: ContextProfile::Full,
            token_budget: None,
            include_git: false,
            include_call_paths: false,
        },
    )
}

pub fn eval_retrieval_with_options(
    root: &Path,
    index: &CodeIndex,
    suite: BenchmarkSuiteInput,
    limit: usize,
    snippets_per_file: usize,
    include_snippets: bool,
    view_options: ContextViewOptions,
) -> Result<EvalRetrievalOutput> {
    let mut task_outputs = Vec::new();
    let mut total_expected_files = 0;
    let mut total_expected_files_found = 0;
    let mut total_critical_files = 0;
    let mut total_critical_files_found = 0;
    let mut total_selected_tokens = 0;
    let mut first_correct_file_hits = 0;
    let mut first_correct_file_tasks = 0;

    for task in suite.tasks {
        let BenchmarkSuiteTaskInput {
            id,
            task,
            expected_files,
            critical_files,
            observed: _,
            session: _,
        } = task;
        let benchmark = benchmark_context_with_options(
            root,
            index,
            &task,
            limit,
            snippets_per_file,
            include_snippets,
            view_options,
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
        let first_correct_file = first_correct_file(&selected_files, &expected_files);
        let first_correct_file_hit = first_correct_file.is_some();
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
        if !expected_files.is_empty() {
            first_correct_file_tasks += 1;
            if first_correct_file_hit {
                first_correct_file_hits += 1;
            }
        }

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
            first_correct_file_hit,
            first_correct_file: first_correct_file.as_ref().map(|(file, _)| file.clone()),
            first_correct_file_rank: first_correct_file.map(|(_, rank)| rank),
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
        first_correct_file_hits,
        first_correct_file_tasks,
        first_correct_file_rate_at_k: recall(first_correct_file_hits, first_correct_file_tasks),
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
            model,
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
    let mut total_first_correct_file_hits = 0;
    let mut total_first_correct_file_tasks = 0;
    let mut total_estimated_token_savings = 0;
    let mut total_baseline_context_payload_tokens = 0;
    let mut total_callsieve_context_payload_tokens = 0;
    let mut total_estimated_reduction_percent = 0.0;
    let mut total_avoided_grep_commands = 0;
    let mut total_avoided_file_reads = 0;
    let mut total_packet_quality = ContextPacketQuality::default();
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
        let mut repo_first_correct_file_hits = 0;
        let mut repo_first_correct_file_tasks = 0;
        let mut repo_estimated_token_savings = 0;
        let mut repo_baseline_context_payload_tokens = 0;
        let mut repo_callsieve_context_payload_tokens = 0;
        let mut repo_reduction_percent_total = 0.0;
        let mut repo_avoided_grep_commands = 0;
        let mut repo_avoided_file_reads = 0;
        let mut repo_packet_quality = ContextPacketQuality::default();
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
            repo_first_correct_file_hits += output.summary.first_correct_file_hits;
            repo_first_correct_file_tasks += output.summary.first_correct_file_tasks;
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
            repo_packet_quality.add(&output.summary.packet_quality);
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
        total_first_correct_file_hits += repo_first_correct_file_hits;
        total_first_correct_file_tasks += repo_first_correct_file_tasks;
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
        total_packet_quality.add(&repo_packet_quality);

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
            first_correct_file_hits: repo_first_correct_file_hits,
            first_correct_file_tasks: repo_first_correct_file_tasks,
            first_correct_file_rate_at_k: recall(
                repo_first_correct_file_hits,
                repo_first_correct_file_tasks,
            ),
            baseline_context_payload_tokens_estimate: repo_baseline_context_payload_tokens,
            callsieve_context_payload_tokens_estimate: repo_callsieve_context_payload_tokens,
            context_payload_reduction: context_payload_reduction(
                repo_baseline_context_payload_tokens,
                repo_callsieve_context_payload_tokens,
            ),
            packet_quality: repo_packet_quality,
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
        first_correct_file_hits: total_first_correct_file_hits,
        first_correct_file_tasks: total_first_correct_file_tasks,
        first_correct_file_rate_at_k: recall(
            total_first_correct_file_hits,
            total_first_correct_file_tasks,
        ),
        baseline_context_payload_tokens_estimate: total_baseline_context_payload_tokens,
        callsieve_context_payload_tokens_estimate: total_callsieve_context_payload_tokens,
        context_payload_reduction: context_payload_reduction(
            total_baseline_context_payload_tokens,
            total_callsieve_context_payload_tokens,
        ),
        packet_quality: total_packet_quality,
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

pub fn benchmark_report_trace_policy_check(
    manifest: &BenchmarkReportManifest,
) -> Result<TraceCheckOutput> {
    let mut sessions = 0usize;
    let mut violations = 0usize;
    let mut grep_before_context = 0usize;
    let mut grep_after_context = 0usize;
    let mut violation_details = Vec::new();

    for repo in &manifest.repos {
        for trace_path in repo.policy_trace_paths() {
            let trace_json = fs::read_to_string(&trace_path)?;
            let check = trace_check_from_str_with_options(&trace_json, true)?;
            sessions += check.sessions;
            violations += check.violations;
            grep_before_context += check.grep_before_context;
            grep_after_context += check.grep_after_context;
            violation_details.extend(check.violation_details);
        }
    }

    Ok(TraceCheckOutput {
        status: if violations == 0 { "pass" } else { "fail" }.to_string(),
        strict: true,
        sessions,
        violations,
        grep_before_context,
        grep_after_context,
        context_first_compliant: violations == 0 && sessions > 0,
        violation_details,
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
        let mut repo_grep_before_context = 0usize;
        let mut repo_grep_after_context = 0usize;
        let mut repo_violation_details = Vec::new();
        let mut repo_mislabeled_controlled_replay = false;
        let mut repo_mislabeled_hook_trace = false;
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
            if trace_collection == TraceCollection::ObservedSession
                && trace_has_hook_trace_markers(&trace_value)
            {
                repo_mislabeled_hook_trace = true;
            }
        }
        for trace_path in repo.policy_trace_paths() {
            let trace_json = fs::read_to_string(&trace_path)?;
            let check = trace_check_from_str_with_options(&trace_json, true)?;
            repo_trace_sessions += check.sessions;
            repo_trace_violations += check.violations;
            repo_grep_before_context += check.grep_before_context;
            repo_grep_after_context += check.grep_after_context;
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
            grep_before_context: repo_grep_before_context,
            grep_after_context: repo_grep_after_context,
            context_first_compliant: repo_trace_violations == 0,
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
        if repo_mislabeled_hook_trace {
            repo_failed = true;
            failures.push(PilotFailure {
                label: repo.label.clone(),
                path: repo.path.display().to_string(),
                check: "observed_trace_mislabeled_hook_trace".to_string(),
                message: "trace metadata says observed_session but lifecycle hook trace markers are present"
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
    let mut grep_before_context = 0;
    let mut grep_after_context = 0;
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

        if let Some((grep_index, grep_command)) = first_grep {
            if first_context.is_none_or(|(context_index, _)| grep_index < context_index) {
                grep_before_context += 1;
                violation_details.push(TraceCheckViolation {
                    id: optional_string(task.get("id")),
                    task: optional_string(task.get("task")),
                    event_kind: "grep_before_context".to_string(),
                    first_violation_command: grep_command.clone(),
                    first_grep_command: grep_command.clone(),
                    first_file_read_command: None,
                    first_callsieve_context_command: first_context
                        .map(|(_, command)| command.clone()),
                    reason: "grep or broad search happened before callsieve_context".to_string(),
                });
                continue;
            }

            if first_context.is_some_and(|(context_index, _)| grep_index > context_index) {
                grep_after_context += 1;
            }
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
        grep_before_context,
        grep_after_context,
        context_first_compliant: violations == 0,
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

/// Anchor-weighted graph consensus. Measured motivation
/// (benchmarks/public/results/nl-miss-graph-adjacency-study.json): 82% of the
/// public NL-bench misses are one import/reference/directory hop from the
/// lexical pool, and 17/22 are adjacent to a top-3 candidate — natural-language
/// issue text matches the consumer layer around the buggy file, and the graph
/// closes the hop vocabulary cannot. Files adjacent to the top-ranked
/// candidates get support proportional to how many anchors agree, weighted by
/// anchor rank; same-directory counts only for the top anchor.
const GRAPH_CONSENSUS_BASE: i32 = 60;
const GRAPH_CONSENSUS_ANCHORS: usize = 3;
const NL_MODULE_NEIGHBOR_ANCHORS: usize = 8;
const NL_MODULE_NEIGHBOR_MIN_ANCHORS: usize = 2;
const NL_MODULE_NEIGHBOR_MAX_PER_MODULE: usize = 4;
const NL_MODULE_NEIGHBOR_BASE: i32 = 80;

struct ModuleNeighborSupport {
    top_score: i32,
    anchor_paths: BTreeSet<String>,
}

fn add_natural_language_module_neighbors(
    lookup: &IndexLookup<'_>,
    ranked: &[ranker::RankedMatch],
    grouped: &mut BTreeMap<String, ContextCandidate>,
    query_tokens: &[String],
) {
    if query_has_unique_code_file_stem(lookup, query_tokens) {
        return;
    }

    let mut modules: BTreeMap<String, ModuleNeighborSupport> = BTreeMap::new();

    for ranked_match in ranked.iter().take(NL_MODULE_NEIGHBOR_ANCHORS) {
        let Some(file) = lookup.file_by_id(&ranked_match.file_id) else {
            continue;
        };
        if !module_neighbor_anchor_file(file) {
            continue;
        }
        let entry =
            modules
                .entry(file.module_path.clone())
                .or_insert_with(|| ModuleNeighborSupport {
                    top_score: ranked_match.score,
                    anchor_paths: BTreeSet::new(),
                });
        entry.top_score = entry.top_score.max(ranked_match.score);
        entry.anchor_paths.insert(file.path.clone());
    }

    for (module_path, support) in modules {
        let anchor_count = support.anchor_paths.len();
        if anchor_count < NL_MODULE_NEIGHBOR_MIN_ANCHORS {
            continue;
        }

        let mut siblings: Vec<(i32, &FileRecord)> = lookup
            .files_by_path
            .values()
            .copied()
            .filter(|file| file.module_path == module_path)
            .filter(|file| module_neighbor_anchor_file(file))
            .filter(|file| !support.anchor_paths.contains(file.path.as_str()))
            .map(|file| (module_neighbor_affinity(file, lookup, query_tokens), file))
            .collect();

        siblings.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.path.cmp(&right.1.path))
        });

        for (offset, (affinity, file)) in siblings
            .into_iter()
            .take(NL_MODULE_NEIGHBOR_MAX_PER_MODULE)
            .enumerate()
        {
            let anchor_points = (NL_MODULE_NEIGHBOR_BASE * anchor_count as i32) + (affinity * 12);
            let offset_penalty = (offset as i32) * 8;
            let raw_points = anchor_points.saturating_sub(offset_penalty).max(1);
            let ceiling = support.top_score.saturating_sub(1);
            let points = if ceiling > 0 {
                raw_points.min(ceiling)
            } else {
                raw_points
            };
            let entry = grouped
                .entry(file.id.clone())
                .or_insert_with(|| ContextCandidate::new(file.id.clone(), 0, usize::MAX));
            entry.add_consensus_boost(
                points,
                format!("same module as natural-language anchors: {module_path}"),
            );
        }
    }
}

fn module_neighbor_anchor_file(file: &FileRecord) -> bool {
    file.language.is_code()
        && !file.is_test
        && !file.is_config
        && !file.module_path.is_empty()
        && !is_probably_third_party_path(&file.path)
}

fn module_neighbor_affinity(
    file: &FileRecord,
    lookup: &IndexLookup<'_>,
    query_tokens: &[String],
) -> i32 {
    let query: BTreeSet<&str> = query_tokens.iter().map(String::as_str).collect();
    let path_terms: BTreeSet<String> = path_tokens(&file.path).into_iter().collect();
    let path_overlap = path_terms
        .iter()
        .filter(|term| query.contains(term.as_str()))
        .count() as i32;
    let content_overlap = file
        .content_terms
        .iter()
        .filter(|term| query.contains(term.as_str()))
        .take(6)
        .count() as i32;
    let symbol_overlap = lookup
        .symbols_for_file(&file.id)
        .iter()
        .map(|symbol| {
            module_neighbor_symbol_terms(symbol, file)
                .iter()
                .filter(|term| query.contains(term.as_str()))
                .take(4)
                .count() as i32
        })
        .max()
        .unwrap_or(0);

    (path_overlap * 4) + (content_overlap * 2) + (symbol_overlap * 3)
}

fn module_neighbor_symbol_terms(symbol: &SymbolRecord, file: &FileRecord) -> BTreeSet<String> {
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

fn query_has_unique_code_file_stem(lookup: &IndexLookup<'_>, query_tokens: &[String]) -> bool {
    let query: BTreeSet<&str> = query_tokens
        .iter()
        .filter(|token| token.len() >= 5)
        .map(String::as_str)
        .collect();
    if query.is_empty() {
        return false;
    }

    let mut matching_stems: BTreeMap<&str, usize> =
        query.iter().map(|token| (*token, 0usize)).collect();
    for file in lookup.files_by_path.values().copied() {
        if !file.language.is_code() || file.is_test || file.is_config {
            continue;
        }
        for term in path_tokens(file_stem_for_path(&file.path)) {
            if let Some(count) = matching_stems.get_mut(term.as_str()) {
                *count += 1;
            }
        }
    }

    matching_stems.values().any(|count| *count == 1)
}

fn file_stem_for_path(path: &str) -> &str {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .split('.')
        .next()
        .unwrap_or(path)
}

fn is_probably_third_party_path(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    path.starts_with("cextern/")
        || path.contains("/extern/")
        || path.contains("/vendor/")
        || path.contains("/node_modules/")
}

fn add_graph_consensus_boost(lookup: &IndexLookup<'_>, candidates: &mut [ContextCandidate]) {
    if candidates.len() <= 1 {
        return;
    }
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by(|&left, &right| {
        candidates[right]
            .score()
            .cmp(&candidates[left].score())
            .then_with(|| candidates[left].file_id.cmp(&candidates[right].file_id))
    });

    struct Anchor {
        path: String,
        weight: i32,
        score: i32,
    }
    let anchors: Vec<Anchor> = order
        .iter()
        .take(GRAPH_CONSENSUS_ANCHORS)
        .enumerate()
        .filter_map(|(rank, &index)| {
            let file = lookup.file_by_id(&candidates[index].file_id)?;
            Some(Anchor {
                path: file.path.clone(),
                weight: (GRAPH_CONSENSUS_ANCHORS - rank) as i32,
                score: candidates[index].score(),
            })
        })
        .collect();
    if anchors.is_empty() {
        return;
    }
    let anchor_paths: BTreeSet<&str> = anchors.iter().map(|anchor| anchor.path.as_str()).collect();

    // neighbor path -> (consensus points, agreeing anchor paths)
    let mut consensus: BTreeMap<String, (i32, BTreeSet<String>)> = BTreeMap::new();
    for anchor in &anchors {
        let mut neighbors: BTreeSet<String> = BTreeSet::new();
        neighbors.extend(resolved_imports_for_file(lookup, &anchor.path));
        for import in lookup.imports_to_path(&anchor.path) {
            neighbors.insert(import.source_path.clone());
        }
        for reference in lookup.references_from_path(&anchor.path) {
            if let Some(target) = &reference.target_path {
                neighbors.insert(target.clone());
            }
        }
        for reference in lookup.references_to_path(&anchor.path) {
            neighbors.insert(reference.source_path.clone());
        }
        for neighbor in neighbors {
            if anchor_paths.contains(neighbor.as_str()) {
                continue;
            }
            let entry = consensus.entry(neighbor).or_default();
            if entry.1.insert(anchor.path.clone()) {
                entry.0 += anchor.weight;
            }
        }
    }

    let top_anchor_dir = lookup
        .file_by_path(&anchors[0].path)
        .map(|file| file.module_path.clone());

    for candidate in candidates.iter_mut() {
        let Some(file) = lookup.file_by_id(&candidate.file_id) else {
            continue;
        };
        if anchor_paths.contains(file.path.as_str()) {
            continue;
        }
        let mut points = 0;
        let mut agreeing_anchors = 0usize;
        let mut agreeing: Vec<String> = Vec::new();
        if let Some((edge_points, anchors_agreeing)) = consensus.get(&file.path) {
            points += edge_points;
            agreeing_anchors += anchors_agreeing.len();
            agreeing.extend(anchors_agreeing.iter().cloned());
        }
        if agreeing_anchors > 0 && top_anchor_dir.as_deref() == Some(file.module_path.as_str()) {
            points += 1;
            agreeing.push(format!("same directory as {}", anchors[0].path));
        }
        // A single anchor's neighborhood is too noisy to act on (hub files
        // like a package __init__ or a client facade touch everything);
        // boosting requires independent agreement of at least two anchors.
        // And a recommender outranks its recommendations: the boost may lift
        // a candidate at most to just below the strongest endorsing anchor's
        // pre-boost score, so a truth that IS the top anchor can never be
        // displaced by its own neighborhood.
        if agreeing_anchors >= 2 {
            let endorsement_ceiling = anchors
                .iter()
                .filter(|anchor| {
                    agreeing
                        .iter()
                        .any(|endorser| endorser.contains(anchor.path.as_str()))
                })
                .map(|anchor| anchor.score)
                .max()
                .unwrap_or(0);
            let boost = (points * GRAPH_CONSENSUS_BASE)
                .min((endorsement_ceiling - 1 - candidate.score()).max(0));
            if boost > 0 {
                candidate.add_consensus_boost(
                    boost,
                    format!(
                        "graph consensus with top candidates: {}",
                        agreeing.join(", ")
                    ),
                );
            }
        }
    }
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
    references.extend(
        lookup
            .references_to_path(path)
            .iter()
            .copied()
            .filter(|reference| reference.source_path != path && reference.kind != "call")
            .map(|reference| reference.source_path.clone()),
    );
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
        // the index omits source_range when it equals the reference line
        source_range: reference
            .source_range
            .or(Some([reference.line, reference.line])),
        target_range: reference.target_range,
    }
}

/// Compact impact summary for a just-edited file, sized for a PostToolUse
/// hook message: who calls it, which tests cover it, and the blast-radius
/// risk level. None when the file is not indexed.
#[derive(Debug, Serialize)]
pub struct EditImpact {
    pub file: String,
    pub risk: String,
    pub callers: Vec<String>,
    pub tests: Vec<String>,
}

pub fn edit_impact_for_file(index: &CodeIndex, path: &str) -> Option<EditImpact> {
    let lookup = IndexLookup::new(index);
    let file = lookup.file_by_path(path)?;
    let imports = resolved_imports_for_file(&lookup, &file.path);
    let referenced_by = references_to_file(&lookup, &file.path);
    let tests = related_tests(&lookup, file);
    let calls = calls_from_file(&lookup, file);
    let called_by = called_by_file(&lookup, file);
    let radius = blast_radius_for(&imports, &referenced_by, &tests, &calls, &called_by);
    let mut callers: Vec<String> = called_by.iter().map(|edge| edge.file.clone()).collect();
    callers.sort();
    callers.dedup();
    callers.truncate(3);
    Some(EditImpact {
        file: file.path.clone(),
        risk: radius.risk,
        callers,
        tests: tests.iter().map(|test| test.file.clone()).take(2).collect(),
    })
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
                context_selected_files: Vec::new(),
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
                context_selected_files: Vec::new(),
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
        retrieval_cost: zero_token_retrieval_cost(),
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

fn first_correct_file(
    selected_files: &[String],
    expected_files: &[String],
) -> Option<(String, usize)> {
    if expected_files.is_empty() {
        return None;
    }
    let expected: BTreeSet<&str> = expected_files.iter().map(String::as_str).collect();
    selected_files.iter().enumerate().find_map(|(index, file)| {
        expected
            .contains(file.as_str())
            .then(|| (file.clone(), index + 1))
    })
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

        let covered_files: BTreeSet<&str> = observed
            .callsieve
            .files_read
            .iter()
            .chain(observed.callsieve.context_selected_files.iter())
            .map(String::as_str)
            .collect();
        let missing: Vec<String> = expected_files
            .into_iter()
            .filter(|file| !covered_files.contains(file.as_str()))
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
            .filter(|file| !covered_files.contains(file.as_str()))
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
        truncated: false,
        omitted_lines: None,
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

fn focused_symbol_snippets(
    root: &Path,
    file: &FileRecord,
    symbols: &[&SymbolRecord],
    snippets_per_symbol: usize,
) -> Vec<Snippet> {
    if snippets_per_symbol == 0 {
        return Vec::new();
    }

    let Ok(content) = fs::read_to_string(root.join(&file.path)) else {
        return Vec::new();
    };
    let lines: Vec<&str> = content.lines().collect();
    symbols
        .iter()
        .take(snippets_per_symbol)
        .filter_map(|symbol| focused_symbol_snippet_from_lines(&lines, symbol))
        .collect()
}

fn focused_symbol_snippet_from_lines(lines: &[&str], symbol: &SymbolRecord) -> Option<Snippet> {
    if lines.is_empty() {
        return None;
    }
    let start = symbol.start_line.max(1).min(lines.len());
    let symbol_end = symbol.end_line.max(start).min(lines.len());
    let capped_end = (start + MAX_FOCUS_SYMBOL_SNIPPET_LINES - 1)
        .min(symbol_end)
        .max(start);
    let truncated = capped_end < symbol_end;
    let text = lines[start - 1..capped_end].join("\n");
    Some(Snippet {
        lines: [start, capped_end],
        text,
        truncated,
        omitted_lines: truncated.then_some(symbol_end - capped_end),
    })
}

#[derive(Clone, Copy)]
enum FocusEdgeKind {
    Calls,
    References,
    CalledBy,
}

fn focus_edges_for_symbols(
    lookup: &IndexLookup<'_>,
    symbols: &[&SymbolRecord],
    kind: FocusEdgeKind,
) -> Vec<FocusEdge> {
    symbols
        .iter()
        .flat_map(|symbol| {
            let edges = match kind {
                FocusEdgeKind::Calls => calls_from_symbol(lookup, symbol),
                FocusEdgeKind::References => references_from_symbol(lookup, symbol),
                FocusEdgeKind::CalledBy => called_by_symbol(lookup, symbol),
            };
            edges.into_iter().map(focus_edge_from_reference)
        })
        .take(MAX_FOCUS_GRAPH_EDGES)
        .collect()
}

fn focus_edge_from_reference(edge: ReferenceEdge) -> FocusEdge {
    FocusEdge {
        file: edge.file,
        symbol: edge.symbol,
        target: edge.target,
        target_file: edge.target_file,
        line: edge.line,
    }
}

fn related_tests(lookup: &IndexLookup<'_>, file: &FileRecord) -> Vec<RelatedTest> {
    if file.is_test {
        return Vec::new();
    }

    let stem = file_stem(&file.path).to_ascii_lowercase();
    let mut tests: Vec<(i32, &FileRecord)> = lookup
        .test_files
        .iter()
        .filter_map(|candidate| {
            let candidate_path = candidate.path.to_ascii_lowercase();
            let mut score = 0;
            if candidate_path.contains(stem.as_str()) {
                score += 3;
            }
            if lookup
                .imports_from_path(&candidate.path)
                .iter()
                .any(|import| import.resolved_path.as_deref() == Some(file.path.as_str()))
            {
                score += 6;
            }
            if lookup
                .references_from_path(&candidate.path)
                .iter()
                .any(|reference| reference.target_path.as_deref() == Some(file.path.as_str()))
            {
                score += 6;
            }
            (score > 0).then_some((score, *candidate))
        })
        .collect();

    tests.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.path.cmp(&right.1.path)));
    tests
        .into_iter()
        .take(5)
        .map(|(_, test_file)| RelatedTest {
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

fn trace_has_hook_trace_markers(value: &serde_json::Value) -> bool {
    let collection = value
        .get("metadata")
        .and_then(|metadata| metadata.get("collection"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if collection.ends_with("_hook_trace") {
        return true;
    }

    let source = value
        .get("policy")
        .and_then(|policy| policy.get("source"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if source.contains("lifecycle_hooks") || source.contains("plugin_hooks") {
        return true;
    }

    trace_tasks_from_value(value.clone()).iter().any(|task| {
        task.get("events")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .any(|event| event.get("hook_event").is_some())
    }) || value
        .get("events")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .any(|event| event.get("hook_event").is_some())
}

fn is_grep_command(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let first = lower.split_whitespace().next().unwrap_or_default();
    matches!(first, "rg" | "grep" | "ripgrep")
        || lower.contains(" rg ")
        || lower.contains(" grep ")
        || lower.contains("ripgrep")
        || lower.starts_with("git grep")
        || lower.contains(" git grep ")
        || matches!(first, "find" | "fd")
        || lower.contains(" select-string ")
        || lower.starts_with("select-string ")
        || (lower.contains("get-childitem") && lower.contains("-recurse"))
        || (lower.starts_with("dir ") && lower.contains("/s"))
        || (lower.starts_with("ls ") && lower.contains("-r"))
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
        Language::Php => "php",
        Language::Go => "go",
        Language::Java => "java",
        Language::CSharp => "csharp",
        Language::C => "c",
        Language::Cpp => "cpp",
        Language::Ruby => "ruby",
        Language::Kotlin => "kotlin",
        Language::Swift => "swift",
        Language::Scala => "scala",
        Language::Dart => "dart",
        Language::Lua => "lua",
        Language::Shell => "shell",
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

    fn minimal_file(id: &str, path: &str) -> FileRecord {
        FileRecord {
            id: id.to_string(),
            path: path.to_string(),
            language: Language::Rust,
            size_bytes: 0,
            line_count: 0,
            mtime: 0,
            content_hash: format!("fnv1a64:{id}"),
            is_test: false,
            is_config: false,
            module_path: "src".to_string(),
            content_terms: Vec::new(),
            ownership: None,
            git: None,
        }
    }

    fn minimal_index(files: Vec<FileRecord>) -> CodeIndex {
        CodeIndex {
            schema_version: SCHEMA_VERSION,
            root: ".".to_string(),
            metadata: Default::default(),
            files,
            symbols: Vec::new(),
            imports: Vec::new(),
            references: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn import_record(source: &str, resolved: &str) -> ImportRecord {
        ImportRecord {
            file_id: format!("file:{source}"),
            source_path: source.to_string(),
            imported: resolved.to_string(),
            resolved_path: Some(resolved.to_string()),
            aliases: Vec::new(),
        }
    }

    fn reference_record(source: &str, target: &str, kind: &str) -> ReferenceRecord {
        ReferenceRecord {
            file_id: format!("file:{source}"),
            source_path: source.to_string(),
            source_symbol_id: None,
            target_name: "targetSymbol".to_string(),
            target_symbol_id: None,
            target_path: Some(target.to_string()),
            kind: kind.to_string(),
            line: 1,
            edge_source: "tree_sitter".to_string(),
            confidence: 0.8,
            lsp_method: None,
            source_range: None,
            target_range: None,
        }
    }

    #[test]
    fn graph_consensus_requires_two_independent_anchors() {
        let mut files = vec![
            minimal_file("a1", "src/api/client.rs"),
            minimal_file("a2", "src/api/server.rs"),
            minimal_file("a3", "src/api/router.rs"),
            minimal_file("nb", "src/core/shared.rs"),
            minimal_file("n1", "src/core/single.rs"),
        ];
        for file in &mut files {
            file.module_path = std::path::Path::new(&file.path)
                .parent()
                .unwrap()
                .to_string_lossy()
                .to_string();
        }
        let mut index = minimal_index(files);
        index.imports = vec![
            import_record("src/api/client.rs", "src/core/shared.rs"),
            import_record("src/api/server.rs", "src/core/shared.rs"),
            import_record("src/api/client.rs", "src/core/single.rs"),
        ];
        let lookup = IndexLookup::new(&index);
        let mut candidates = vec![
            ContextCandidate::new("a1".to_string(), 300, 0),
            ContextCandidate::new("a2".to_string(), 200, 1),
            ContextCandidate::new("a3".to_string(), 100, 2),
            ContextCandidate::new("nb".to_string(), 10, 3),
            ContextCandidate::new("n1".to_string(), 10, 4),
        ];

        add_graph_consensus_boost(&lookup, &mut candidates);

        // nb: anchors 1 (weight 3) and 2 (weight 2) agree -> 5 * BASE, but a
        // recommendation never outranks its strongest recommender (score 300),
        // so the boost caps at one below it.
        assert_eq!(candidates[3].score(), 299);
        // n1: single-anchor adjacency is hub noise, no boost
        assert_eq!(candidates[4].score(), 10);
        // anchors themselves are never boosted
        assert_eq!(candidates[0].score(), 300);
        assert_eq!(candidates[1].score(), 200);
        assert_eq!(candidates[2].score(), 100);
    }

    #[test]
    fn references_to_file_includes_non_call_reference_edges() {
        let mut index = minimal_index(vec![
            minimal_file("target", "src/auth/session.ts"),
            minimal_file("importer", "src/auth/router.ts"),
            minimal_file("reader", "src/user/profile.ts"),
            minimal_file("caller", "src/auth/session.test.ts"),
        ]);
        index.imports = vec![import_record("src/auth/router.ts", "src/auth/session.ts")];
        index.references = vec![
            reference_record("src/user/profile.ts", "src/auth/session.ts", "reference"),
            reference_record("src/auth/session.test.ts", "src/auth/session.ts", "call"),
            reference_record("src/auth/session.ts", "src/auth/session.ts", "reference"),
        ];
        let lookup = IndexLookup::new(&index);

        assert_eq!(
            references_to_file(&lookup, "src/auth/session.ts"),
            vec!["src/auth/router.ts", "src/user/profile.ts"]
        );
    }

    #[test]
    fn graph_consensus_same_directory_counts_only_alongside_edges() {
        let mut files = vec![
            minimal_file("a1", "src/api/client.rs"),
            minimal_file("a2", "src/api/server.rs"),
            minimal_file("a3", "src/api/router.rs"),
            minimal_file("sib", "src/api/helpers.rs"),
            minimal_file("sib2", "src/api/other.rs"),
        ];
        for file in &mut files {
            file.module_path = "src/api".to_string();
        }
        let mut index = minimal_index(files);
        index.imports = vec![
            import_record("src/api/server.rs", "src/api/helpers.rs"),
            import_record("src/api/router.rs", "src/api/helpers.rs"),
        ];
        let lookup = IndexLookup::new(&index);
        let mut candidates = vec![
            ContextCandidate::new("a1".to_string(), 300, 0),
            ContextCandidate::new("a2".to_string(), 200, 1),
            ContextCandidate::new("a3".to_string(), 100, 2),
            ContextCandidate::new("sib".to_string(), 10, 3),
            ContextCandidate::new("sib2".to_string(), 10, 4),
        ];

        add_graph_consensus_boost(&lookup, &mut candidates);

        // sib: edges from anchors 2 (weight 2) + 3 (weight 1) plus same-dir
        // with the top anchor (+1) -> 4 * BASE
        assert_eq!(candidates[3].score(), 10 + 4 * GRAPH_CONSENSUS_BASE);
        // sib2: same-directory alone never boosts
        assert_eq!(candidates[4].score(), 10);
    }

    #[test]
    fn natural_language_module_neighbors_inject_same_module_siblings() {
        let temp = tempfile::tempdir().unwrap();
        write(
            temp.path().join("src/payments/router.ts"),
            "export function checkoutRouter() {\n  return 'charge flow';\n}\n",
        );
        write(
            temp.path().join("src/payments/service.ts"),
            "export function chargeService() {\n  return 'checkout flow';\n}\n",
        );
        write(
            temp.path().join("src/payments/settlement.ts"),
            "export function settleLedger() {\n  return 'ok';\n}\n",
        );

        let index = indexer::build_index(temp.path()).unwrap();
        let output =
            build_context(temp.path(), &index, "fix checkout charge flow", 3, 0, false).unwrap();
        let files = context_read_first_files(&output);

        assert!(
            files.contains(&"src/payments/settlement.ts".to_string()),
            "same-module sibling should be injected for natural-language vocabulary gaps: {files:?}"
        );
        let sibling = output
            .read_first
            .iter()
            .find(|file| file.file == "src/payments/settlement.ts")
            .unwrap();
        assert!(
            sibling
                .why
                .iter()
                .any(|why| why.contains("same module as natural-language anchors")),
            "injected sibling should explain module-neighbor evidence: {:?}",
            sibling.why
        );
    }

    #[test]
    fn natural_language_module_neighbors_skip_unique_file_stem_queries() {
        let mut files = vec![
            minimal_file("ownership", "src/indexer/ownership.rs"),
            minimal_file("query", "src/query/mod.rs"),
            minimal_file("ranker", "src/query/ranker.rs"),
            minimal_file("django_models", "django/db/models.py"),
            minimal_file("astropy_models", "astropy/modeling/models.py"),
        ];
        for file in &mut files {
            file.module_path = std::path::Path::new(&file.path)
                .parent()
                .unwrap()
                .to_string_lossy()
                .to_string();
        }
        let index = minimal_index(files);
        let lookup = IndexLookup::new(&index);

        let ownership_tokens =
            ranker::query_tokens("where is ownership information attached to selected files");
        assert!(
            query_has_unique_code_file_stem(&lookup, &ownership_tokens),
            "a unique ownership.rs stem should keep module-neighbor expansion out of the way"
        );

        let models_tokens = ranker::query_tokens("fix nested combined models");
        assert!(
            !query_has_unique_code_file_stem(&lookup, &models_tokens),
            "common stems such as models.py should still allow vocabulary-gap module expansion"
        );
    }

    #[test]
    fn confirm_task_memory_reads_upserts_and_enriches() {
        let temp = tempfile::tempdir().unwrap();
        let task = "investigate session creation lifecycle pool connection handling";

        let stored = confirm_task_memory_reads(
            temp.path(),
            task,
            &["session.ts".to_string()],
            &["obscure.ts".to_string()],
            "claude",
            100,
        )
        .unwrap();
        assert_eq!(stored, 1);

        // Second session for the same task adds without duplicating; the
        // returned count is actual insertions, not inputs (obscure.ts is
        // already known, so only extra.ts counts).
        let stored = confirm_task_memory_reads(
            temp.path(),
            task,
            &[],
            &["obscure.ts".to_string(), "extra.ts".to_string()],
            "cursor",
            200,
        )
        .unwrap();
        assert_eq!(stored, 1);

        let memory = load_task_memory(&task_memory_path(temp.path()));
        assert_eq!(memory.entries.len(), 1);
        let entry = &memory.entries[0];
        assert_eq!(entry.confirmed_files, vec!["obscure.ts", "extra.ts"]);
        assert_eq!(entry.client, "claude", "first teacher keeps provenance");
    }

    #[test]
    fn merge_task_memory_unions_and_caps() {
        let temp = tempfile::tempdir().unwrap();
        confirm_task_memory_reads(
            temp.path(),
            "investigate session creation lifecycle pool connection handling",
            &[],
            &["a.ts".to_string()],
            "claude",
            100,
        )
        .unwrap();

        let exported = serde_json::json!({
            "schema_version": 1,
            "entries": [{
                "task": "investigate session creation lifecycle pool connection handling",
                "task_terms": ["connection","creation","handling","investigate","lifecycle","pool","session"],
                "created_at": 200,
                "read_first_files": ["b.ts"],
                "symbols": [],
                "tests": [],
                "confirmed_files": ["b.ts"],
                "client": "cursor"
            }, {
                "task": "another task entirely about widgets",
                "task_terms": ["another","entirely","task","widgets"],
                "created_at": 150,
                "read_first_files": ["w.ts"],
                "symbols": [],
                "tests": []
            }]
        })
        .to_string();

        let (imported, total) = merge_task_memory(temp.path(), &exported).unwrap();
        assert_eq!(imported, 2);
        assert_eq!(total, 2);

        let memory = load_task_memory(&task_memory_path(temp.path()));
        let merged = memory
            .entries
            .iter()
            .find(|entry| entry.task.starts_with("investigate"))
            .unwrap();
        assert_eq!(merged.confirmed_files, vec!["a.ts", "b.ts"]);
        assert_eq!(merged.created_at, 200, "newer entry wins recency");
        assert_eq!(merged.client, "claude", "local provenance is kept");
    }

    #[test]
    fn memory_boost_injects_confirmed_files_lexical_missed() {
        let temp = tempfile::tempdir().unwrap();
        let index = minimal_index(vec![
            minimal_file("hit", "src/session.rs"),
            minimal_file("obscure", "src/obscure.rs"),
        ]);
        let lookup = IndexLookup::new(&index);
        confirm_task_memory_reads(
            temp.path(),
            "investigate session creation lifecycle pool connection handling",
            &[],
            &["src/obscure.rs".to_string()],
            "claude",
            100,
        )
        .unwrap();

        let mut candidates = vec![ContextCandidate::new("hit".to_string(), 100, 0)];
        add_memory_confirmed_boost(
            temp.path(),
            "fix session creation lifecycle pool connection handling regression",
            &lookup,
            &mut candidates,
        );

        assert_eq!(candidates.len(), 2, "confirmed file must be injected");
        let injected = candidates
            .iter()
            .find(|candidate| candidate.file_id == "obscure")
            .unwrap();
        assert_eq!(injected.score(), MEMORY_CONFIRMED_BOOST);
        assert!(injected.why[0].contains("agent-confirmed read"));
    }

    #[test]
    fn ownership_tie_break_prefers_matching_owner() {
        let mut owned = minimal_file("a", "src/a.rs");
        owned.ownership = Some(Ownership {
            owners: Vec::new(),
            teams: vec!["@org/payments".to_string()],
        });
        let unowned = minimal_file("b", "src/b.rs");
        let index = minimal_index(vec![unowned, owned]);
        let lookup = IndexLookup::new(&index);
        let query_tokens = ranker::query_tokens("payments behavior");
        let mut candidates = vec![
            ContextCandidate::new("b".to_string(), 10, 0),
            ContextCandidate::new("a".to_string(), 10, 1),
        ];

        sort_candidates_lexical(&mut candidates, &lookup, &query_tokens);

        assert_eq!(candidates[0].file_id, "a");
    }

    #[test]
    fn ownership_tie_break_preserves_unowned_order() {
        let index = minimal_index(vec![
            minimal_file("a", "src/a.rs"),
            minimal_file("b", "src/b.rs"),
        ]);
        let lookup = IndexLookup::new(&index);
        let query_tokens = ranker::query_tokens("behavior");
        let mut candidates = vec![
            ContextCandidate::new("b".to_string(), 10, 0),
            ContextCandidate::new("a".to_string(), 10, 1),
        ];

        sort_candidates_lexical(&mut candidates, &lookup, &query_tokens);

        assert_eq!(candidates[0].file_id, "b");
        assert_eq!(candidates[1].file_id, "a");
    }

    #[cfg(feature = "embed")]
    struct FakeEmbedder;

    #[cfg(feature = "embed")]
    impl embed::LocalEmbedder for FakeEmbedder {
        fn id(&self) -> embed::EmbedderId {
            embed::EmbedderId::new("fake-hybrid", "v1")
        }

        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
            Ok(texts
                .iter()
                .map(|text| {
                    if text.contains("relative") {
                        vec![0.0, 1.0]
                    } else {
                        vec![1.0, 0.0]
                    }
                })
                .collect())
        }
    }

    #[cfg(feature = "embed")]
    fn write_fake_cache(root: &Path, index: &CodeIndex, embedder: &FakeEmbedder) {
        let cache = embed::EmbedCache {
            embedder: embed::LocalEmbedder::id(embedder),
            index_schema_version: SCHEMA_VERSION,
            fingerprint: embed::index_fingerprint(index),
            dim: 2,
            vectors: vec![vec![0.0, -1.0], vec![0.0, 1.0]],
            chunk_owners: vec![0, 1],
            chunk_symbols: vec![Some("lex".to_string()), Some("sem".to_string())],
        };
        embed::write_embeds(root, &cache, false).unwrap();
    }

    #[cfg(feature = "embed")]
    #[test]
    fn hybrid_blend_uses_query_kind_weights() {
        let temp = tempfile::tempdir().unwrap();
        let index = minimal_index(vec![
            minimal_file("lex", "src/lex.rs"),
            minimal_file("sem", "src/sem.rs"),
        ]);
        let lookup = IndexLookup::new(&index);
        let embedder = FakeEmbedder;
        write_fake_cache(temp.path(), &index, &embedder);

        let natural_tokens = ranker::query_tokens("fix relative location header");
        let mut natural_candidates = vec![
            ContextCandidate::new("lex".to_string(), 100, 0),
            ContextCandidate::new("sem".to_string(), 90, 1),
        ];
        let natural_scores = add_semantic_candidates(
            temp.path(),
            &index,
            "fix relative location header",
            &mut natural_candidates,
            5,
            &natural_tokens,
            HybridOptions::with_embedder(true, &embedder),
        )
        .unwrap();
        let mut warnings = Vec::new();
        apply_hybrid_ranking(
            temp.path(),
            &index,
            "fix relative location header",
            &natural_tokens,
            &lookup,
            &mut natural_candidates,
            HybridOptions::with_embedder(true, &embedder),
            natural_scores.as_ref(),
            &mut warnings,
        )
        .unwrap();
        assert!(warnings.is_empty());
        assert_eq!(natural_candidates[0].file_id, "sem");

        let identifier_tokens = ranker::query_tokens("fix RelativeLocationHeader");
        let mut identifier_candidates = vec![
            ContextCandidate::new("lex".to_string(), 100, 0),
            ContextCandidate::new("sem".to_string(), 90, 1),
        ];
        let identifier_scores = add_semantic_candidates(
            temp.path(),
            &index,
            "fix RelativeLocationHeader",
            &mut identifier_candidates,
            5,
            &identifier_tokens,
            HybridOptions::with_embedder(true, &embedder),
        )
        .unwrap();
        apply_hybrid_ranking(
            temp.path(),
            &index,
            "fix RelativeLocationHeader",
            &identifier_tokens,
            &lookup,
            &mut identifier_candidates,
            HybridOptions::with_embedder(true, &embedder),
            identifier_scores.as_ref(),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(identifier_candidates[0].file_id, "lex");
    }

    #[cfg(feature = "embed")]
    #[test]
    fn hybrid_blend_does_not_lift_tests_above_source_without_test_intent() {
        let temp = tempfile::tempdir().unwrap();
        let mut test_file = minimal_file("testfile", "tests/table_test.rs");
        test_file.is_test = true;
        let index = minimal_index(vec![minimal_file("source", "src/rst.rs"), test_file]);
        let lookup = IndexLookup::new(&index);
        let embedder = FakeEmbedder;
        // source embeds far from the query, the test file embeds on top of it
        let cache = embed::EmbedCache {
            embedder: embed::LocalEmbedder::id(&embedder),
            index_schema_version: SCHEMA_VERSION,
            fingerprint: embed::index_fingerprint(&index),
            dim: 2,
            vectors: vec![vec![0.0, -1.0], vec![0.0, 1.0]],
            chunk_owners: vec![0, 1],
            chunk_symbols: vec![None, None],
        };
        embed::write_embeds(temp.path(), &cache, false).unwrap();

        let task = "fix relative ordering warnings";
        let tokens = ranker::query_tokens(task);
        let mut candidates = vec![
            ContextCandidate::new("source".to_string(), 100, 0),
            ContextCandidate::new("testfile".to_string(), 90, 1),
        ];
        let scores = add_semantic_candidates(
            temp.path(),
            &index,
            task,
            &mut candidates,
            5,
            &tokens,
            HybridOptions::with_embedder(true, &embedder),
        )
        .unwrap();
        let mut warnings = Vec::new();
        apply_hybrid_ranking(
            temp.path(),
            &index,
            task,
            &tokens,
            &lookup,
            &mut candidates,
            HybridOptions::with_embedder(true, &embedder),
            scores.as_ref(),
            &mut warnings,
        )
        .unwrap();
        assert_eq!(
            candidates[0].file_id, "source",
            "semantic similarity must not promote a test file above the source it trailed"
        );

        // With explicit test intent the guard stands down.
        let task = "fix relative ordering tests";
        let tokens = ranker::query_tokens(task);
        let mut candidates = vec![
            ContextCandidate::new("source".to_string(), 100, 0),
            ContextCandidate::new("testfile".to_string(), 90, 1),
        ];
        let scores = add_semantic_candidates(
            temp.path(),
            &index,
            task,
            &mut candidates,
            5,
            &tokens,
            HybridOptions::with_embedder(true, &embedder),
        )
        .unwrap();
        apply_hybrid_ranking(
            temp.path(),
            &index,
            task,
            &tokens,
            &lookup,
            &mut candidates,
            HybridOptions::with_embedder(true, &embedder),
            scores.as_ref(),
            &mut warnings,
        )
        .unwrap();
        assert_eq!(
            candidates[0].file_id, "testfile",
            "test-intent queries keep the blended order"
        );
    }

    #[test]
    fn compact_local_work_emits_injection_count_only_when_nonzero() {
        let quiet = LocalWorkStats {
            indexed_files: 10,
            indexed_symbols: 5,
            indexed_references: 2,
            semantic_injected: 0,
        };
        assert!(compact_local_work_for_value(&quiet).get("inj").is_none());

        let injecting = LocalWorkStats {
            semantic_injected: 3,
            ..quiet
        };
        assert_eq!(
            compact_local_work_for_value(&injecting)
                .get("inj")
                .and_then(Value::as_u64),
            Some(3)
        );
    }

    #[cfg(feature = "embed")]
    #[test]
    fn identifier_queries_let_semantic_promote_but_never_demote() {
        let temp = tempfile::tempdir().unwrap();
        let index = minimal_index(vec![
            minimal_file("anchor", "src/rst.rs"),
            minimal_file("rival", "src/connect.rs"),
        ]);
        let lookup = IndexLookup::new(&index);
        let embedder = FakeEmbedder;
        // anchor embeds far from the query, rival embeds on top of it
        let cache = embed::EmbedCache {
            embedder: embed::LocalEmbedder::id(&embedder),
            index_schema_version: SCHEMA_VERSION,
            fingerprint: embed::index_fingerprint(&index),
            dim: 2,
            vectors: vec![vec![0.0, -1.0], vec![0.0, 1.0]],
            chunk_owners: vec![0, 1],
            chunk_symbols: vec![None, None],
        };
        embed::write_embeds(temp.path(), &cache, false).unwrap();

        // Identifier-kind task (camelCase signal) with "relative" so the fake
        // embedder maps the query next to the rival's vector.
        let task = "fix RelativeHeader output";
        let tokens = ranker::query_tokens(task);
        assert_eq!(
            classify::query_kind(task, &tokens),
            classify::QueryKind::Identifier
        );
        let mut candidates = vec![
            ContextCandidate::new("anchor".to_string(), 100, 0),
            ContextCandidate::new("rival".to_string(), 90, 1),
        ];
        let scores = add_semantic_candidates(
            temp.path(),
            &index,
            task,
            &mut candidates,
            5,
            &tokens,
            HybridOptions::with_embedder(true, &embedder),
        )
        .unwrap();
        let mut warnings = Vec::new();
        apply_hybrid_ranking(
            temp.path(),
            &index,
            task,
            &tokens,
            &lookup,
            &mut candidates,
            HybridOptions::with_embedder(true, &embedder),
            scores.as_ref(),
            &mut warnings,
        )
        .unwrap();
        assert_eq!(
            candidates[0].file_id, "anchor",
            "identifier queries must not let semantic similarity demote the lexical leader"
        );
    }

    #[cfg(feature = "embed")]
    #[test]
    fn hybrid_blend_is_deterministic() {
        let temp = tempfile::tempdir().unwrap();
        let index = minimal_index(vec![
            minimal_file("lex", "src/lex.rs"),
            minimal_file("sem", "src/sem.rs"),
        ]);
        let lookup = IndexLookup::new(&index);
        let embedder = FakeEmbedder;
        write_fake_cache(temp.path(), &index, &embedder);
        let tokens = ranker::query_tokens("fix relative location header");

        let mut first = vec![
            ContextCandidate::new("lex".to_string(), 100, 0),
            ContextCandidate::new("sem".to_string(), 90, 1),
        ];
        let mut second = vec![
            ContextCandidate::new("lex".to_string(), 100, 0),
            ContextCandidate::new("sem".to_string(), 90, 1),
        ];
        for candidates in [&mut first, &mut second] {
            let semantic_scores = add_semantic_candidates(
                temp.path(),
                &index,
                "fix relative location header",
                candidates,
                5,
                &tokens,
                HybridOptions::with_embedder(true, &embedder),
            )
            .unwrap();
            apply_hybrid_ranking(
                temp.path(),
                &index,
                "fix relative location header",
                &tokens,
                &lookup,
                candidates,
                HybridOptions::with_embedder(true, &embedder),
                semantic_scores.as_ref(),
                &mut Vec::new(),
            )
            .unwrap();
        }

        let first_order: Vec<&str> = first
            .iter()
            .map(|candidate| candidate.file_id.as_str())
            .collect();
        let second_order: Vec<&str> = second
            .iter()
            .map(|candidate| candidate.file_id.as_str())
            .collect();
        assert_eq!(first_order, second_order);
    }

    #[cfg(feature = "embed")]
    #[test]
    fn semantic_scores_from_cache_selects_best_chunk_per_file() {
        let index = minimal_index(vec![
            minimal_file("target", "src/target.rs"),
            minimal_file("other", "src/other.rs"),
        ]);
        let embedder = FakeEmbedder;
        let cache = embed::EmbedCache {
            embedder: embed::LocalEmbedder::id(&embedder),
            index_schema_version: SCHEMA_VERSION,
            fingerprint: embed::index_fingerprint(&index),
            dim: 2,
            vectors: vec![vec![1.0, 0.0], vec![0.0, 1.0], vec![0.5, 0.5]],
            chunk_owners: vec![0, 0, 1],
            chunk_symbols: vec![
                Some("symbol:weak".to_string()),
                Some("symbol:best".to_string()),
                Some("symbol:other".to_string()),
            ],
        };

        let scores = semantic_scores_from_cache(&index, &cache, &[0.0, 1.0]);
        let target = scores.get("target").expect("target score");

        assert_eq!(target.chunk_symbol.as_deref(), Some("symbol:best"));
        assert!((target.cosine - 1.0).abs() < f32::EPSILON);
    }

    #[cfg(feature = "embed")]
    #[test]
    fn semantic_recall_injects_files_lexical_missed() {
        let temp = tempfile::tempdir().unwrap();
        let index = minimal_index(vec![
            minimal_file("lex", "src/lex.rs"),
            minimal_file("sem", "src/sem.rs"),
            minimal_file("noise", "src/noise.rs"),
        ]);
        let embedder = FakeEmbedder;
        // Cache aligned to file order. "sem" lives on the same axis the query
        // embeds onto; "lex"/"noise" are orthogonal to it.
        let cache = embed::EmbedCache {
            embedder: embed::LocalEmbedder::id(&embedder),
            index_schema_version: SCHEMA_VERSION,
            fingerprint: embed::index_fingerprint(&index),
            dim: 2,
            vectors: vec![vec![1.0, 0.0], vec![0.0, 1.0], vec![1.0, 0.0]],
            chunk_owners: vec![0, 1, 2],
            chunk_symbols: vec![
                Some("lex".to_string()),
                Some("sem".to_string()),
                Some("noise".to_string()),
            ],
        };
        embed::write_embeds(temp.path(), &cache, false).unwrap();

        // Only the lexical hit is a candidate; "sem" and "noise" are not, so the
        // pre-union recall ceiling excludes the semantically-relevant "sem".
        let mut candidates = vec![ContextCandidate::new("lex".to_string(), 100, 0)];

        // "relative" makes FakeEmbedder return [0,1] -> close to "sem".
        let task = "relative redirect handling";
        add_semantic_candidates(
            temp.path(),
            &index,
            task,
            &mut candidates,
            5,
            &ranker::query_tokens(task),
            HybridOptions::with_embedder(true, &embedder),
        )
        .unwrap();

        let ids: Vec<&str> = candidates.iter().map(|c| c.file_id.as_str()).collect();
        assert!(
            ids.contains(&"sem"),
            "semantic recall should inject the file lexical missed, got {ids:?}"
        );
        assert!(
            !ids.contains(&"noise"),
            "below-floor file must not be injected, got {ids:?}"
        );

        let injected = candidates.iter().find(|c| c.file_id == "sem").unwrap();
        assert_eq!(injected.best_score, 0, "injected file has no lexical score");
        assert_eq!(injected.symbol_ids, vec!["sem".to_string()]);
        assert!(
            injected.why.iter().any(|w| w.contains("semantic recall")),
            "injected file should explain its provenance"
        );
        assert!(
            injected.why.iter().any(|w| w.contains("via sem")),
            "injected file should name the matched symbol in why text"
        );
    }

    #[cfg(feature = "embed")]
    #[test]
    fn semantic_recall_is_noop_when_embeddings_off() {
        let temp = tempfile::tempdir().unwrap();
        let index = minimal_index(vec![
            minimal_file("lex", "src/lex.rs"),
            minimal_file("sem", "src/sem.rs"),
        ]);
        let embedder = FakeEmbedder;
        let cache = embed::EmbedCache {
            embedder: embed::LocalEmbedder::id(&embedder),
            index_schema_version: SCHEMA_VERSION,
            fingerprint: embed::index_fingerprint(&index),
            dim: 2,
            vectors: vec![vec![1.0, 0.0], vec![0.0, 1.0]],
            chunk_owners: vec![0, 1],
            chunk_symbols: vec![Some("lex".to_string()), Some("sem".to_string())],
        };
        embed::write_embeds(temp.path(), &cache, false).unwrap();

        let mut candidates = vec![ContextCandidate::new("lex".to_string(), 100, 0)];
        // Default options have embeddings off -> deterministic lexical behavior.
        let task = "relative redirect handling";
        add_semantic_candidates(
            temp.path(),
            &index,
            task,
            &mut candidates,
            5,
            &ranker::query_tokens(task),
            HybridOptions::default(),
        )
        .unwrap();
        assert_eq!(candidates.len(), 1, "no injection when embeddings are off");
    }

    #[test]
    fn error_context_surfaces_stack_trace_files_first() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::write(
            temp.path().join("src/popular.rs"),
            "pub fn popular_feature() {\n    // popular feature popular feature work\n}\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("src/crash.rs"),
            "pub fn handle_request() {\n    panic!(\"boom\");\n}\n",
        )
        .unwrap();
        let index = indexer::build_index(temp.path()).unwrap();

        // Without the trace, the keyword-matching file leads.
        let baseline = build_context_with(
            temp.path(),
            &index,
            "popular feature work",
            ContextOptions {
                limit: 8,
                snippets_per_file: 0,
                include_snippets: false,
                why_debug: false,
                hybrid: HybridOptions::default(),
                error_frames: &[],
                git_boost: false,
                memory_boost: false,
            },
        )
        .unwrap();
        assert_ne!(
            baseline.read_first.first().map(|f| f.file.as_str()),
            Some("src/crash.rs"),
            "crash.rs should not lead without a stack trace"
        );

        // With the trace, crash.rs is promoted to the top with provenance and
        // the enclosing symbol attached.
        let frames = stacktrace::parse_stack_trace("thread 'main' panicked at src/crash.rs:2:5");
        let context = build_context_with(
            temp.path(),
            &index,
            "popular feature work",
            ContextOptions {
                limit: 8,
                snippets_per_file: 0,
                include_snippets: false,
                why_debug: false,
                hybrid: HybridOptions::default(),
                error_frames: &frames,
                git_boost: false,
                memory_boost: false,
            },
        )
        .unwrap();
        let first = context.read_first.first().expect("a read-first file");
        assert_eq!(first.file, "src/crash.rs", "stack-trace file should lead");
        assert!(
            first.why.iter().any(|w| w.contains("stack trace")),
            "promoted file should explain its provenance, got {:?}",
            first.why
        );
        assert!(
            first.symbols.iter().any(|s| s.name == "handle_request"),
            "the symbol enclosing the trace line should be attached"
        );
    }

    #[test]
    fn git_boost_raises_hot_files_only_when_enabled() {
        let mut hot = minimal_file("hot", "src/hot.rs");
        hot.git = Some(crate::indexer::git::GitSignal {
            last_modified_unix: 100,
            commits_30d: 4,
            commits_90d: 8,
            distinct_authors_90d: 3,
            churn_90d: 200,
        });
        let cold = minimal_file("cold", "src/cold.rs");
        let index = minimal_index(vec![hot, cold]);

        let make = || {
            vec![
                ContextCandidate::new("cold".to_string(), 100, 0),
                ContextCandidate::new("hot".to_string(), 100, 1),
            ]
        };
        let score_of = |candidates: &[ContextCandidate], id: &str| {
            candidates
                .iter()
                .find(|candidate| candidate.file_id == id)
                .unwrap()
                .score()
        };

        // Disabled: equal lexical scores stay equal (no-op, determinism).
        let mut off = make();
        apply_git_boost(&index, &mut off, false);
        assert_eq!(score_of(&off, "hot"), score_of(&off, "cold"));

        // Enabled: the hot file is raised above the cold one and explains why.
        let mut on = make();
        apply_git_boost(&index, &mut on, true);
        assert!(score_of(&on, "hot") > score_of(&on, "cold"));
        let hot_candidate = on.iter().find(|c| c.file_id == "hot").unwrap();
        assert!(
            hot_candidate
                .why
                .iter()
                .any(|w| w.contains("recently changed")),
            "boosted file should explain the git signal, got {:?}",
            hot_candidate.why
        );
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
    fn skim_context_can_surface_compact_call_paths_when_enabled() {
        let (temp, index) = fixture_index();
        let output = build_context(
            temp.path(),
            &index,
            "change createSession behavior",
            8,
            2,
            false,
        )
        .unwrap();
        let value = context_value(
            &output,
            ContextViewOptions {
                profile: ContextProfile::Skim,
                token_budget: Some(DEFAULT_AGENT_CONTEXT_TOKEN_BUDGET),
                include_git: false,
                include_call_paths: true,
            },
        )
        .unwrap();
        let session_file = value["read_first"]
            .as_array()
            .unwrap()
            .iter()
            .find(|file| file["f"] == "src/auth/session.ts")
            .unwrap();

        assert!(session_file.get("snippets").is_none());
        assert!(session_file.get("file").is_none());
        assert!(session_file.get("call_paths").is_none());
        let calls = session_file["cp"]["c"].as_array().unwrap();
        assert!(calls.len() <= MAX_SKIM_CALL_PATHS_PER_DIRECTION);
        assert!(
            calls
                .iter()
                .any(|edge| { edge["t"] == "tokenFor" && edge["f"] == "src/auth/token.ts" })
        );
        let called_by = session_file["cp"]["by"].as_array().unwrap();
        assert!(called_by.len() <= MAX_SKIM_CALL_PATHS_PER_DIRECTION);
        assert!(called_by.iter().any(|edge| {
            edge["t"] == "createSession" && edge["f"] == "src/auth/session.test.ts"
        }));
    }

    #[test]
    fn skim_context_drops_redundant_keyword_reasons() {
        let reasons = vec![
            "symbol name keyword cluster: graph, hints".to_string(),
            "keyword overlap: graph, hints".to_string(),
            "test companion: tests/cli.rs".to_string(),
        ];

        assert_eq!(
            compact_why_for_value(&reasons),
            vec!["sy:graph, hints".to_string()]
        );
    }

    #[test]
    fn skim_context_uses_short_reason_codes() {
        assert_eq!(
            compact_reason_for_value("exact symbol match: createSession"),
            "sym:createSession"
        );
        assert_eq!(
            compact_reason_for_value("content keyword overlap: token, budget"),
            "ct:token, budget"
        );
        assert_eq!(
            compact_reason_for_value("path keyword overlap: agent, cli"),
            "pt:agent, cli"
        );
        assert_eq!(
            compact_reason_for_value("test companion: tests/cli.rs"),
            "test:tests/cli.rs"
        );
    }

    #[test]
    fn skim_context_uses_short_risk_codes() {
        assert_eq!(compact_risk_for_value("low"), "l");
        assert_eq!(compact_risk_for_value("medium"), "m");
        assert_eq!(compact_risk_for_value("high"), "h");
        assert_eq!(compact_risk_for_value("unknown"), "unknown");
    }

    #[test]
    fn skim_context_uses_short_symbol_kind_codes() {
        let constant = vec![QuerySymbol {
            name: "MAX_SKIM_SYMBOLS_PER_FILE".to_string(),
            kind: "constant".to_string(),
            lines: [37, 37],
            visibility: "private".to_string(),
            signature: "const MAX_SKIM_SYMBOLS_PER_FILE: usize = 1;".to_string(),
        }];
        let class = vec![QuerySymbol {
            name: "Session".to_string(),
            kind: "class".to_string(),
            lines: [3, 9],
            visibility: "exported".to_string(),
            signature: "export class Session".to_string(),
        }];
        let function = vec![QuerySymbol {
            name: "createSession".to_string(),
            kind: "function".to_string(),
            lines: [12, 48],
            visibility: "exported".to_string(),
            signature: "export function createSession".to_string(),
        }];

        assert_eq!(
            compact_symbols_for_value(&constant),
            vec![json!(["MAX_SKIM_SYMBOLS_PER_FILE", 37, "c"])]
        );
        assert_eq!(
            compact_symbols_for_value(&class),
            vec![json!(["Session", 3, "cl"])]
        );
        assert_eq!(
            compact_symbols_for_value(&function),
            vec![json!(["createSession", 12])]
        );
    }

    #[test]
    fn skim_context_uses_short_selection_signal_codes() {
        let components = vec![
            SelectionScoreComponent {
                name: "symbol_name_keyword_cluster".to_string(),
                points: 420,
            },
            SelectionScoreComponent {
                name: "competitive_positioning_doc".to_string(),
                points: 760,
            },
        ];

        assert_eq!(
            compact_selection_score_components_for_value(&components),
            vec![json!("sy"), json!("comp")]
        );
    }

    #[test]
    fn selection_confidence_uses_relative_score_tiers() {
        assert_eq!(selection_confidence_for_score(100, 100), "high");
        assert_eq!(selection_confidence_for_score(45, 100), "medium");
        assert_eq!(selection_confidence_for_score(44, 100), "low");
        assert_eq!(selection_confidence_for_score(0, 100), "low");
        assert_eq!(selection_confidence_for_score(10, 0), "medium");
    }

    #[test]
    fn skim_context_uses_read_first_indexes_for_selection_files() {
        let summary = ContextSelectionSummary {
            top_file: Some("src/auth/session.ts".to_string()),
            top_score: Some(140),
            top_reason: Some("exact symbol match: createSession".to_string()),
            top_signals: Vec::new(),
            next_files: vec![SelectionSummaryFile {
                file: "src/auth/session.test.ts".to_string(),
                score: 92,
                reason: Some("test companion: src/auth/session.test.ts".to_string()),
            }],
        };
        let path_indexes = BTreeMap::from([
            ("src/auth/session.ts", 0usize),
            ("src/auth/session.test.ts", 2usize),
        ]);

        assert_eq!(
            compact_selection_summary_for_value(&summary, &path_indexes),
            json!({
                "top": [0, "sym:createSession"],
                "next": [[2, "test:src/auth/session.test.ts"]]
            })
        );
        assert_eq!(
            compact_selection_summary_for_value(&summary, &BTreeMap::new()),
            json!({
                "top": ["src/auth/session.ts", 140, "sym:createSession"],
                "next": [["src/auth/session.test.ts", 92, "test:src/auth/session.test.ts"]]
            })
        );
    }

    #[test]
    fn selection_summary_trim_preserves_indexed_next_files() {
        let mut value = json!({
            "read_first": [
                {"f": "src/auth/session.ts"},
                {"f": "src/auth/session.test.ts"}
            ],
            "sel": {
                "next": [
                    [1, "test:src/auth/session.test.ts"],
                    [2, "missing"],
                    ["src/auth/session.test.ts", 92, "test:path"],
                    ["src/auth/missing.ts", 91, "missing:path"]
                ]
            }
        });

        trim_selection_summary_to_read_first(&mut value);

        assert_eq!(
            value["sel"]["next"],
            json!([
                [1, "test:src/auth/session.test.ts"],
                ["src/auth/session.test.ts", 92, "test:path"]
            ])
        );
    }

    #[test]
    fn skim_context_uses_read_first_indexes_for_impact_tests() {
        let file = ContextFile {
            rank: 1,
            score: 42,
            selection_confidence: "high".to_string(),
            file: "src/auth/session.ts".to_string(),
            language: Language::TypeScript,
            symbols: Vec::new(),
            snippets: Vec::new(),
            imports: vec!["src/auth/token.ts".to_string()],
            referenced_by: Vec::new(),
            blast_radius: BlastRadius {
                imports: Vec::new(),
                referenced_by: Vec::new(),
                tests: vec!["src/auth/session.test.ts".to_string()],
                calls: Vec::new(),
                called_by: Vec::new(),
                risk: "medium".to_string(),
            },
            calls: Vec::new(),
            called_by: Vec::new(),
            related_tests: Vec::new(),
            ownership: None,
            git: None,
            why: Vec::new(),
            why_debug: Vec::new(),
        };
        let path_indexes = BTreeMap::from([
            ("src/auth/session.ts", 0usize),
            ("src/auth/session.test.ts", 2usize),
        ]);

        assert_eq!(
            compact_impact_for_value(&file, &path_indexes),
            json!(["m", 2, 1, "test,im"])
        );
    }

    #[test]
    fn skim_context_impact_flags_name_graph_edge_kinds() {
        let file = ContextFile {
            rank: 1,
            score: 42,
            selection_confidence: "high".to_string(),
            file: "src/auth/session.ts".to_string(),
            language: Language::TypeScript,
            symbols: Vec::new(),
            snippets: Vec::new(),
            imports: Vec::new(),
            referenced_by: vec!["src/auth/session.test.ts".to_string()],
            blast_radius: BlastRadius {
                imports: vec!["src/auth/token.ts".to_string()],
                referenced_by: vec!["src/auth/session.test.ts".to_string()],
                tests: vec!["src/auth/session.test.ts".to_string()],
                calls: vec!["src/auth/token.ts".to_string()],
                called_by: vec!["src/auth/session.test.ts".to_string()],
                risk: "high".to_string(),
            },
            calls: vec![ReferenceEdge {
                file: "src/auth/session.ts".to_string(),
                symbol: Some("createSession".to_string()),
                target: "tokenFor".to_string(),
                target_file: Some("src/auth/token.ts".to_string()),
                kind: "call".to_string(),
                line: 4,
                edge_source: "tree_sitter".to_string(),
                confidence: 0.8,
                lsp_method: None,
                source_range: Some([4, 4]),
                target_range: None,
            }],
            called_by: vec![ReferenceEdge {
                file: "src/auth/session.test.ts".to_string(),
                symbol: None,
                target: "createSession".to_string(),
                target_file: Some("src/auth/session.ts".to_string()),
                kind: "call".to_string(),
                line: 4,
                edge_source: "tree_sitter".to_string(),
                confidence: 0.8,
                lsp_method: None,
                source_range: Some([4, 4]),
                target_range: None,
            }],
            related_tests: Vec::new(),
            ownership: None,
            git: None,
            why: Vec::new(),
            why_debug: Vec::new(),
        };
        let path_indexes = BTreeMap::from([
            ("src/auth/session.ts", 0usize),
            ("src/auth/session.test.ts", 2usize),
        ]);

        assert_eq!(
            compact_impact_for_value(&file, &path_indexes),
            json!(["h", 2, 2, 2, "test,im,call,ref,by"])
        );
    }

    #[test]
    fn skim_context_graph_hints_are_non_test_previews() {
        let file = ContextFile {
            rank: 1,
            score: 42,
            selection_confidence: "high".to_string(),
            file: "src/auth/session.ts".to_string(),
            language: Language::TypeScript,
            symbols: Vec::new(),
            snippets: Vec::new(),
            imports: Vec::new(),
            referenced_by: Vec::new(),
            blast_radius: BlastRadius {
                imports: vec![
                    "src/auth/token.ts".to_string(),
                    "src/auth/util.ts".to_string(),
                ],
                referenced_by: vec![
                    "editors/vscode/test/suite/index.ts".to_string(),
                    "src/auth/caller.ts".to_string(),
                    "src/auth/session.spec.ts".to_string(),
                    "src/auth/session.test.ts".to_string(),
                ],
                tests: Vec::new(),
                calls: Vec::new(),
                called_by: Vec::new(),
                risk: "medium".to_string(),
            },
            calls: Vec::new(),
            called_by: Vec::new(),
            related_tests: vec![RelatedTest {
                file: "src/auth/session.test.ts".to_string(),
                symbols: Vec::new(),
            }],
            ownership: None,
            git: None,
            why: Vec::new(),
            why_debug: Vec::new(),
        };

        let graph = compact_graph_hints_for_value(&file).unwrap();
        assert_eq!(graph["u"].as_array().unwrap().len(), 1);
        assert_eq!(graph["u"][0], "src/auth/token.ts");
        assert_eq!(graph["d"].as_array().unwrap().len(), 1);
        assert_eq!(graph["d"][0], "src/auth/caller.ts");
    }

    #[test]
    fn skim_context_uses_short_git_keys() {
        let file = ContextFile {
            rank: 1,
            score: 42,
            selection_confidence: "high".to_string(),
            file: "src/lib.rs".to_string(),
            language: Language::Rust,
            symbols: Vec::new(),
            snippets: Vec::new(),
            imports: Vec::new(),
            referenced_by: Vec::new(),
            blast_radius: BlastRadius {
                imports: Vec::new(),
                referenced_by: Vec::new(),
                tests: Vec::new(),
                calls: Vec::new(),
                called_by: Vec::new(),
                risk: "low".to_string(),
            },
            calls: Vec::new(),
            called_by: Vec::new(),
            related_tests: Vec::new(),
            ownership: None,
            git: Some(crate::indexer::git::GitSignal {
                last_modified_unix: 1234,
                commits_30d: 2,
                commits_90d: 7,
                distinct_authors_90d: 3,
                churn_90d: 99,
            }),
            why: Vec::new(),
            why_debug: Vec::new(),
        };

        let git = compact_git_for_value(&file).unwrap();

        assert_eq!(git["lm"], 1234);
        assert_eq!(git["c90"], 7);
        assert_eq!(git["a90"], 3);
        assert!(git.get("last_modified_unix").is_none());
        assert!(git.get("commits_90d").is_none());
        assert!(git.get("authors_90d").is_none());

        let context = ContextOutput {
            task: "inspect git hints".to_string(),
            root: ".".to_string(),
            retrieval_cost: zero_token_retrieval_cost(),
            selection_summary: ContextSelectionSummary {
                top_file: None,
                top_score: None,
                top_reason: None,
                top_signals: Vec::new(),
                next_files: Vec::new(),
            },
            read_first: vec![file],
            stats: ContextStats {
                candidate_matches: 1,
                selected_files: 1,
                selected_symbols: 0,
                related_tests: 0,
                local_work: LocalWorkStats {
                    indexed_files: 1,
                    indexed_symbols: 0,
                    indexed_references: 0,
                    semantic_injected: 0,
                },
            },
            timing: TimingStats::default(),
            warnings: Vec::new(),
        };
        let default = skim_context_value(&context, false, false);
        assert!(default["read_first"][0].get("git").is_none());
        let opt_in = skim_context_value(&context, true, false);
        assert_eq!(opt_in["read_first"][0]["git"]["lm"], 1234);
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
    fn context_promotes_module_anchor_when_parent_module_matches_task() {
        let temp = tempfile::tempdir().unwrap();
        write(
            temp.path().join("src/main.rs"),
            "mod filter;\nmod walk;\n\nfn ensure_use_hidden_option_for_leading_dot_pattern() {}\n",
        );
        write(
            temp.path().join("src/walk.rs"),
            "pub fn walk_directory_tree() {\n  ignore_hidden_entries();\n}\n\nfn ignore_hidden_entries() {}\n",
        );
        write(
            temp.path().join("src/filter/mod.rs"),
            "mod owner;\nmod size;\nmod time;\n\npub use owner::OwnerFilter;\n",
        );
        write(
            temp.path().join("src/filter/owner.rs"),
            "pub struct OwnerFilter;\n\npub const IGNORE: OwnerFilter = OwnerFilter;\n\npub fn filter_ignore(owner: OwnerFilter) -> Option<OwnerFilter> {\n  Some(owner)\n}\n",
        );
        write(
            temp.path().join("src/filter/size.rs"),
            "pub struct SizeFilter;\n\npub fn filter_size() -> SizeFilter {\n  SizeFilter\n}\n",
        );
        write(
            temp.path().join("tests/testenv/mod.rs"),
            "pub fn create_config_directory_with_global_ignore() {}\n",
        );

        let index = indexer::build_index(temp.path()).unwrap();
        let output = build_context(
            temp.path(),
            &index,
            "change directory walking filters and hidden ignore behavior",
            4,
            0,
            true,
        )
        .unwrap();

        assert!(
            output
                .read_first
                .iter()
                .any(|file| file.file == "src/filter/mod.rs"),
            "module anchor should fit into compact read_first output"
        );
    }

    #[test]
    fn context_promotes_implementation_companion_for_test_heavy_tasks() {
        let temp = tempfile::tempdir().unwrap();
        write(
            temp.path().join("httpx/__init__.py"),
            "from ._client import Client, AsyncClient\n",
        );
        write(
            temp.path().join("httpx/_client.py"),
            "class Client:\n    def get(self, url, follow_redirects=False):\n        return self._send_handling_redirects(url)\n\n    def _send_handling_redirects(self, request):\n        return request\n\nclass AsyncClient:\n    async def get(self, url, follow_redirects=False):\n        return await self._send_handling_redirects(url)\n\n    async def _send_handling_redirects(self, request):\n        return request\n",
        );
        for name in [
            "test_client",
            "test_async_client",
            "test_headers",
            "test_redirects",
        ] {
            let content = if name == "test_redirects" {
                "import httpx\n\n\ndef test_redirect_301():\n    client = httpx.Client()\n    response = client.get('https://example.org/redirect', follow_redirects=True)\n    assert response\n\n\ndef test_redirect_history():\n    client = httpx.Client()\n    response = client.get('https://example.org/redirect-chain', follow_redirects=True)\n    assert response\n"
            } else {
                "import httpx\n\n\ndef test_client_closed_state_using_implicit_open():\n    client = httpx.Client()\n    client.get('https://example.org')\n\n\ndef test_client_header_defaults():\n    client = httpx.Client()\n    response = client.get('https://example.org')\n    assert response\n"
            };
            write(temp.path().join(format!("tests/client/{name}.py")), content);
        }

        let index = indexer::build_index(temp.path()).unwrap();
        let output = build_context(
            temp.path(),
            &index,
            "change redirect handling and client redirect tests",
            4,
            1,
            true,
        )
        .unwrap();
        let files = context_read_first_files(&output);

        assert!(
            files.contains(&"httpx/_client.py".to_string()),
            "selected files: {files:?}"
        );
        assert!(
            files.contains(&"tests/client/test_redirects.py".to_string()),
            "selected files: {files:?}"
        );
    }

    #[test]
    fn context_promotes_task_specific_test_companion_for_implementation_tasks() {
        let temp = tempfile::tempdir().unwrap();
        write(
            temp.path().join("httpx/_config.py"),
            "class Timeout:\n    def as_dict(self):\n        return {'timeout': 5}\n",
        );
        write(
            temp.path().join("httpx/_client.py"),
            "from ._config import Timeout\n\nclass Client:\n    def request(self, timeout=None):\n        return Timeout()\n",
        );
        write(
            temp.path().join("httpx/_models.py"),
            "class Request:\n    pass\n",
        );
        write(
            temp.path().join("docs/advanced/timeouts.md"),
            "Timeout configuration controls client request behavior.\n",
        );
        write(
            temp.path().join(".github/ISSUE_TEMPLATE/config.yml"),
            "name: timeout configuration request\n",
        );
        write(
            temp.path().join("tests/test_timeouts.py"),
            "import httpx\n\n\ndef test_timeout_configuration_for_client_request():\n    client = httpx.Client()\n    response = client.request('GET', 'https://example.org', timeout=5)\n    assert response\n",
        );

        let index = indexer::build_index(temp.path()).unwrap();
        let output = build_context(
            temp.path(),
            &index,
            "change timeout configuration and client request behavior",
            4,
            0,
            false,
        )
        .unwrap();
        let files = context_read_first_files(&output);

        assert!(
            files.contains(&"tests/test_timeouts.py".to_string()),
            "selected files: {files:?}"
        );
        assert!(
            files.contains(&"httpx/_config.py".to_string()),
            "selected files: {files:?}"
        );
    }

    #[test]
    fn context_keeps_mcp_docs_when_promoting_cli_test_companion() {
        let temp = tempfile::tempdir().unwrap();
        write(
            temp.path().join("src/mcp.rs"),
            "pub fn execute_context_tool() {\n    call_tool();\n}\n\nfn call_tool() {}\n",
        );
        write(
            temp.path().join("src/cli.rs"),
            "pub enum Command { Mcp }\n\npub fn run() {\n    setup_agent();\n}\n\nfn setup_agent() {}\n",
        );
        write(
            temp.path().join("src/query/mod.rs"),
            "pub fn build_context() {}\nfn add_graph_context() {}\nfn add_reference_context() {}\n",
        );
        write(
            temp.path().join("tests/cli.rs"),
            "#[test]\nfn mcp_lists_and_calls_context_tool() {\n    assert!(true);\n}\n",
        );
        write(
            temp.path().join("docs/MCP.md"),
            "MCP callsieve_context tool setup documentation for coding agents and client config.\n",
        );
        write(
            temp.path().join("src/indexer/lsp.rs"),
            "pub fn server_specs() {\n    let _context = \"mcp tool\";\n}\n",
        );
        write(
            temp.path().join("src/indexer/imports.rs"),
            "pub fn add_import_context() {}\n",
        );

        let index = indexer::build_index(temp.path()).unwrap();
        let output = build_context(
            temp.path(),
            &index,
            "add an MCP tool that exposes CallSieve context packets to coding agents",
            5,
            0,
            false,
        )
        .unwrap();
        let files = context_read_first_files(&output);

        for expected in ["src/mcp.rs", "src/cli.rs", "docs/MCP.md", "tests/cli.rs"] {
            assert!(
                files.contains(&expected.to_string()),
                "expected {expected}; selected files: {files:?}"
            );
        }
    }

    #[test]
    fn test_companion_promotion_does_not_evict_top_ranked_implementation() {
        let temp = tempfile::tempdir().unwrap();
        write(
            temp.path().join("src/query/mod.rs"),
            "pub fn enterprise_proof_report() {}\npub fn proof_report() {}\npub struct ProofReportOutput;\n",
        );
        write(
            temp.path().join("docs/ENTERPRISE_PROOF.md"),
            "Enterprise proof evidence gates and report schema.\n",
        );
        write(
            temp.path().join("tests/cli.rs"),
            "#[test]\nfn enterprise_proof_report_requires_clients_and_session_savings_ratios() {}\n",
        );

        let index = indexer::build_index(temp.path()).unwrap();
        let output = build_context(
            temp.path(),
            &index,
            "change enterprise_proof_report proof report implementation",
            2,
            0,
            false,
        )
        .unwrap();
        let files = context_read_first_files(&output);

        assert!(
            files.contains(&"src/query/mod.rs".to_string()),
            "top implementation should be retained when test companion is promoted: {files:?}"
        );
        assert!(
            files.contains(&"tests/cli.rs".to_string()),
            "task-specific test should still be promoted: {files:?}"
        );
    }

    #[test]
    fn test_companion_promotion_keeps_domain_module_implementation() {
        let temp = tempfile::tempdir().unwrap();
        write(
            temp.path().join("django/db/migrations/serializer.py"),
            "class BaseSerializer:\n    pass\n\nclass EnumSerializer(BaseSerializer):\n    def serialize(self):\n        return 'migration enum default value name generated'\n",
        );
        write(
            temp.path().join("django/db/models/enums.py"),
            "class Choices:\n    pass\n\nclass Status:\n    GOOD = 'Good'\n",
        );
        write(
            temp.path()
                .join("django/core/management/commands/makemigrations.py"),
            "def handle():\n    return 'generated migration file'\n",
        );
        write(
            temp.path().join("django/db/models/fields/__init__.py"),
            "class CharField:\n    def __init__(self, default=None):\n        self.default = default\n",
        );
        write(
            temp.path()
                .join("tests/model_inheritance_regress/models.py"),
            "from django.db import models\n\nclass Item(models.Model):\n    status = models.CharField(default='Good', max_length=128)\n",
        );

        let index = indexer::build_index(temp.path()).unwrap();
        let output = build_context(
            temp.path(),
            &index,
            "generated migration uses enum value instead of enum name for a default",
            5,
            0,
            false,
        )
        .unwrap();
        let files = context_read_first_files(&output);

        assert!(
            files.contains(&"django/db/migrations/serializer.py".to_string()),
            "domain-module implementation should be retained when test companion is promoted: {files:?}"
        );
    }

    #[test]
    fn proof_report_context_keeps_query_surface_at_default_limit() {
        let temp = tempfile::tempdir().unwrap();
        write(
            temp.path().join("src/query/mod.rs"),
            "pub fn enterprise_proof_report() {}\npub fn proof_report() {}\npub struct ProofReportOutput;\npub fn evidence_pack_schema_gate() {}\n",
        );
        write(
            temp.path().join("docs/ENTERPRISE_PROOF.md"),
            "Enterprise proof evidence gates, proof report schema, observed sessions, and strict trace policy.\n",
        );
        write(
            temp.path().join("src/query/ranker.rs"),
            "fn has_benchmark_evidence_intent() {}\nfn benchmark_evidence_doc_score() {}\nfn proof_report_recall_gate() {}\n",
        );
        write(
            temp.path().join("README.md"),
            "CallSieve proves enterprise evidence, proof report, and recall results for agents.\n",
        );
        write(
            temp.path().join("src/cli/mod.rs"),
            "struct EvidencePackOutput;\nfn evidence_pack_protocol() {}\nfn competitive_report_evidence_requires_savings_and_strict_trace_policy() {}\n",
        );
        write(
            temp.path().join("docs/BENCHMARKS.md"),
            "Benchmark evidence proof report manifest suite trace and recall documentation.\n",
        );
        write(
            temp.path()
                .join("benchmarks/evidence/enterprise-proof-manifest.example.json"),
            "{\"proof\":\"evidence\",\"report\":\"manifest\"}\n",
        );
        write(
            temp.path().join("src/bench_public.rs"),
            "pub fn write_report() {}\npub fn compare_enterprise_proof_evidence() {}\n",
        );
        write(
            temp.path().join("tests/cli.rs"),
            "#[test]\nfn enterprise_proof_report_requires_clients_and_session_savings_ratios() {}\n#[test]\nfn evidence_pack_preserves_pmf_metrics_and_redacts_team_identifiers() {}\n",
        );

        let index = indexer::build_index(temp.path()).unwrap();
        let output = build_context(
            temp.path(),
            &index,
            "improve enterprise proof evidence pack proof report schemas and gates",
            8,
            0,
            false,
        )
        .unwrap();
        let files = context_read_first_files(&output);

        assert!(
            files.contains(&"src/query/mod.rs".to_string()),
            "proof implementation surface should be retained at the default limit: {files:?}"
        );
        assert!(
            files.contains(
                &"benchmarks/evidence/enterprise-proof-manifest.example.json".to_string()
            ),
            "proof evidence artifact should be retained at the default limit: {files:?}"
        );
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
                        context_selected_files: Vec::new(),
                        notes: Vec::new(),
                    },
                    callsieve: ObservedSessionMetrics {
                        grep_commands: 1,
                        file_reads: 3,
                        tokens: 4_000,
                        commands: vec!["callsieve context".to_string()],
                        files_read: vec!["src/auth/session.ts".to_string()],
                        context_selected_files: Vec::new(),
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
        assert_eq!(output.summary.first_correct_file_hits, 1);
        assert_eq!(output.summary.first_correct_file_tasks, 1);
        assert_eq!(output.summary.first_correct_file_rate_at_k, 1.0);
        assert!(output.tasks[0].first_correct_file_hit);
        assert_eq!(
            output.tasks[0].first_correct_file.as_deref(),
            Some("src/auth/session.ts")
        );
        assert_eq!(output.tasks[0].first_correct_file_rank, Some(1));
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
